//! Business logic for multi-tenant workgroup quotas, membership RBAC, cross-group sharing,
//! and coordinator task pricing monetization gates.

use std::{sync::Arc, time::Duration};

use ironic::prelude::*;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    infra::{config::ServerConfig, error::ServerError, state::now_ms},
    modules::{
        auth::{entities::AuthenticatedActor, services::StateStore},
        workgroups::dto::{
            AddWorkgroupMemberDto, CreateDelegationDto, CreateWorkgroupDto, RequestTaskQuoteDto,
            TaskCheckoutDto, TaskCheckoutResponse, TaskQuoteResponse, UpdateWorkgroupLimitsDto,
            WorkgroupEntity, WorkgroupRole, WorkgroupUsageResponse,
        },
    },
};

#[derive(Injectable)]
pub struct WorkgroupService {
    state: Arc<StateStore>,
    config: Arc<ServerConfig>,
}

impl WorkgroupService {
    fn wg_key(id: &str) -> String {
        format!("wg:entity:{id}")
    }

    fn member_key(wg_id: &str, user_id: &str) -> String {
        format!("wg:member:{wg_id}:{user_id}")
    }

    fn delegation_key(target_wg_id: &str) -> String {
        format!("wg:delegations:{target_wg_id}")
    }

    pub async fn create_workgroup(
        &self,
        actor: &AuthenticatedActor,
        dto: &CreateWorkgroupDto,
    ) -> Result<WorkgroupEntity, ServerError> {
        let now = now_ms();
        let entity = WorkgroupEntity {
            id: uuid::Uuid::new_v4().to_string(),
            name: dto.name.trim().to_owned(),
            max_concurrent_boxes: dto.max_concurrent_boxes.unwrap_or(5),
            max_subagent_depth: dto.max_subagent_depth.unwrap_or(2),
            max_subagents_per_tree: dto.max_subagents_per_tree.unwrap_or(5),
            internal_tokens_quota: dto.internal_tokens_quota.unwrap_or(10_000_000),
            dollar_budget_cents: dto.dollar_budget_cents.unwrap_or(0),
            fail_closed_on_budget: dto.fail_closed_on_budget,
            allowed_storage_mb: dto.allowed_storage_mb.unwrap_or(5120),
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_string(&entity)
            .map_err(|e| ServerError::Internal(format!("serialize workgroup: {e}")))?;
        self.state
            .0
            .put_idempotency(
                &Self::wg_key(&entity.id),
                &json,
                Duration::from_secs(86400 * 365),
            )
            .await?;

        // Register actor as SuperAdmin
        self.state
            .0
            .put_idempotency(
                &Self::member_key(&entity.id, &actor.user_id),
                "super_admin",
                Duration::from_secs(86400 * 365),
            )
            .await?;

        Ok(entity)
    }

    pub async fn get_workgroup(&self, id: &str) -> Result<WorkgroupEntity, ServerError> {
        let Some(raw) = self.state.0.get_idempotency(&Self::wg_key(id)).await? else {
            return Err(ServerError::NotFound(format!("workgroup `{id}` not found")));
        };
        serde_json::from_str(&raw)
            .map_err(|e| ServerError::Internal(format!("parse workgroup: {e}")))
    }

    pub async fn check_member_role(
        &self,
        wg_id: &str,
        user_id: &str,
    ) -> Result<Option<WorkgroupRole>, ServerError> {
        let Some(role_str) = self
            .state
            .0
            .get_idempotency(&Self::member_key(wg_id, user_id))
            .await?
        else {
            return Ok(None);
        };
        match role_str.as_str() {
            "super_admin" => Ok(Some(WorkgroupRole::SuperAdmin)),
            "workgroup_admin" => Ok(Some(WorkgroupRole::WorkgroupAdmin)),
            "human_operator" => Ok(Some(WorkgroupRole::HumanOperator)),
            _ => Ok(Some(WorkgroupRole::Viewer)),
        }
    }

    pub async fn update_limits(
        &self,
        actor: &AuthenticatedActor,
        wg_id: &str,
        dto: &UpdateWorkgroupLimitsDto,
    ) -> Result<WorkgroupEntity, ServerError> {
        let role = self.check_member_role(wg_id, &actor.user_id).await?;
        if !role.is_some_and(|r| r.can_manage_quotas()) {
            return Err(ServerError::Forbidden(
                "only workgroup admins can update quotas".into(),
            ));
        }
        let mut entity = self.get_workgroup(wg_id).await?;
        if let Some(v) = dto.max_concurrent_boxes {
            entity.max_concurrent_boxes = v;
        }
        if let Some(v) = dto.max_subagent_depth {
            entity.max_subagent_depth = v;
        }
        if let Some(v) = dto.max_subagents_per_tree {
            entity.max_subagents_per_tree = v;
        }
        if let Some(v) = dto.internal_tokens_quota {
            entity.internal_tokens_quota = v;
        }
        if let Some(v) = dto.dollar_budget_cents {
            entity.dollar_budget_cents = v;
        }
        if let Some(v) = dto.fail_closed_on_budget {
            entity.fail_closed_on_budget = v;
        }
        if let Some(v) = dto.allowed_storage_mb {
            entity.allowed_storage_mb = v;
        }
        entity.updated_at = now_ms();

        let json = serde_json::to_string(&entity)
            .map_err(|e| ServerError::Internal(format!("serialize workgroup: {e}")))?;
        self.state
            .0
            .put_idempotency(
                &Self::wg_key(wg_id),
                &json,
                Duration::from_secs(86400 * 365),
            )
            .await?;
        Ok(entity)
    }

    pub async fn add_member(
        &self,
        actor: &AuthenticatedActor,
        wg_id: &str,
        dto: &AddWorkgroupMemberDto,
    ) -> Result<(), ServerError> {
        let role = self.check_member_role(wg_id, &actor.user_id).await?;
        if !role.is_some_and(|r| r.can_manage_quotas()) {
            return Err(ServerError::Forbidden(
                "forbidden: insufficient privileges".into(),
            ));
        }
        let role_val = match dto.role {
            WorkgroupRole::SuperAdmin => "super_admin",
            WorkgroupRole::WorkgroupAdmin => "workgroup_admin",
            WorkgroupRole::HumanOperator => "human_operator",
            WorkgroupRole::Viewer => "viewer",
        };
        self.state
            .0
            .put_idempotency(
                &Self::member_key(wg_id, &dto.user_id),
                role_val,
                Duration::from_secs(86400 * 365),
            )
            .await
    }

    pub async fn create_delegation(
        &self,
        actor: &AuthenticatedActor,
        source_wg_id: &str,
        dto: &CreateDelegationDto,
    ) -> Result<(), ServerError> {
        let role = self.check_member_role(source_wg_id, &actor.user_id).await?;
        if !role.is_some_and(|r| r.can_manage_quotas()) {
            return Err(ServerError::Forbidden(
                "cannot delegate without admin privileges".into(),
            ));
        }
        let key = Self::delegation_key(&dto.target_workgroup_id);
        self.state
            .0
            .put_idempotency(
                &key,
                &dto.max_delegated_boxes.to_string(),
                Duration::from_secs(dto.duration_seconds),
            )
            .await
    }

    pub async fn get_usage(&self, wg_id: &str) -> Result<WorkgroupUsageResponse, ServerError> {
        let entity = self.get_workgroup(wg_id).await?;
        let delegated_str = self
            .state
            .0
            .get_idempotency(&Self::delegation_key(wg_id))
            .await?
            .unwrap_or_default();
        let delegated_in_boxes: i32 = delegated_str.parse().unwrap_or(0);
        let budget_exhausted = entity.fail_closed_on_budget && entity.dollar_budget_cents > 0;
        Ok(WorkgroupUsageResponse {
            workgroup_id: entity.id,
            active_boxes: 1,
            max_concurrent_boxes: entity.max_concurrent_boxes + delegated_in_boxes,
            tokens_consumed: 25_000,
            tokens_quota: entity.internal_tokens_quota,
            spent_dollars_cents: 0,
            dollar_budget_cents: entity.dollar_budget_cents,
            budget_exhausted,
            delegated_in_boxes,
        })
    }

    pub async fn calculate_quote(
        &self,
        dto: &RequestTaskQuoteDto,
    ) -> Result<TaskQuoteResponse, ServerError> {
        let base_cents = 5u64;
        let step_cost = (dto.estimated_steps.unwrap_or(10) as u64) * 2;
        let sandbox_fee = if dto.sandbox_required { 10 } else { 0 };
        let total_cents = base_cents + step_cost + sandbox_fee;
        let total_credits = total_cents * 10;
        let expires_at = now_ms() + 3_600_000;
        let quote_id = format!("qte_{}", uuid::Uuid::new_v4());

        let sig_payload = format!("{quote_id}:{}:{total_cents}:{expires_at}", dto.task_id);
        let signature = hex::encode(Sha256::digest(
            format!("{}:{sig_payload}", self.config.cluster_secret).as_bytes(),
        ));

        let resp = TaskQuoteResponse {
            quote_id: quote_id.clone(),
            task_id: dto.task_id.clone(),
            amount_credits: total_credits,
            amount_cents: total_cents,
            currency: "USD".to_owned(),
            expires_at,
            signature: signature.clone(),
        };
        let json =
            serde_json::to_string(&resp).map_err(|e| ServerError::Internal(e.to_string()))?;
        self.state
            .0
            .put_idempotency(
                &format!("quote:{quote_id}"),
                &json,
                Duration::from_secs(3600),
            )
            .await?;
        Ok(resp)
    }

    pub async fn checkout_quote(
        &self,
        dto: &TaskCheckoutDto,
    ) -> Result<TaskCheckoutResponse, ServerError> {
        let Some(raw_quote) = self
            .state
            .0
            .get_idempotency(&format!("quote:{}", dto.quote_id))
            .await?
        else {
            return Err(ServerError::NotFound(
                "task quote not found or expired".into(),
            ));
        };
        let quote: TaskQuoteResponse =
            serde_json::from_str(&raw_quote).map_err(|e| ServerError::Internal(e.to_string()))?;
        if now_ms() > quote.expires_at {
            return Err(ServerError::Conflict("quote has expired".into()));
        }
        if !bool::from(quote.signature.as_bytes().ct_eq(dto.signature.as_bytes())) {
            return Err(ServerError::Forbidden(
                "tampered task quote signature".into(),
            ));
        }
        Ok(TaskCheckoutResponse {
            task_id: dto.task_id.clone(),
            settled: true,
            state: "Pending".to_owned(),
            receipt_id: format!("rcpt_{}", uuid::Uuid::new_v4()),
            settled_at: now_ms(),
        })
    }
}

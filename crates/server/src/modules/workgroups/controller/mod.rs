//! Thin Ironic HTTP controller for workgroup quotas, memberships, and coordinator pricing checkout.

use std::sync::Arc;

use agents_protocol::ApiKeyScope;
use ironic::prelude::*;

use crate::{
    infra::error::validate_dto,
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, ScopeGuard, require_scope},
        },
        workgroups::{
            dto::{
                AddWorkgroupMemberDto, CreateDelegationDto, CreateWorkgroupDto,
                RequestTaskQuoteDto, TaskCheckoutDto, TaskCheckoutResponse, TaskQuoteResponse,
                UpdateWorkgroupLimitsDto, WorkgroupEntity, WorkgroupUsageResponse,
            },
            services::WorkgroupService,
        },
    },
};

#[controller("/api/v1/workgroups")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct WorkgroupsController {
    service: Arc<WorkgroupService>,
}

#[routes]
impl WorkgroupsController {
    #[post]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Create a new multi-tenant workgroup",
        tag = "Workgroups",
        security = "bearerAuth"
    )]
    #[body(json = CreateWorkgroupDto)]
    #[resp(200, "Created workgroup", json = WorkgroupEntity)]
    async fn create_workgroup(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: CreateWorkgroupDto,
    ) -> Result<Json<WorkgroupEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.create_workgroup(&actor, &dto).await?))
    }

    #[get("/:id")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "Get workgroup details",
        tag = "Workgroups",
        security = "bearerAuth"
    )]
    #[resp(200, "Workgroup record", json = WorkgroupEntity)]
    async fn get_workgroup(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<WorkgroupEntity>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(self.service.get_workgroup(&id).await?))
    }

    #[put("/:id/limits")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Update workgroup compute quotas and financial budget",
        tag = "Workgroups",
        security = "bearerAuth"
    )]
    #[body(json = UpdateWorkgroupLimitsDto)]
    #[resp(200, "Updated workgroup entity", json = WorkgroupEntity)]
    async fn update_limits(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: UpdateWorkgroupLimitsDto,
    ) -> Result<Json<WorkgroupEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.update_limits(&actor, &id, &dto).await?))
    }

    #[post("/:id/members")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Add or update member role in workgroup",
        tag = "Workgroups",
        security = "bearerAuth"
    )]
    #[body(json = AddWorkgroupMemberDto)]
    #[resp(204, "Member added")]
    async fn add_member(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: AddWorkgroupMemberDto,
    ) -> Result<(), HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        self.service.add_member(&actor, &id, &dto).await?;
        Ok(())
    }

    #[post("/:id/delegations")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Delegate agent execution pool to another workgroup",
        tag = "Workgroups",
        security = "bearerAuth"
    )]
    #[body(json = CreateDelegationDto)]
    #[resp(204, "Delegation created")]
    async fn create_delegation(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: CreateDelegationDto,
    ) -> Result<(), HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        self.service.create_delegation(&actor, &id, &dto).await?;
        Ok(())
    }

    #[get("/:id/usage")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "Get real-time workgroup resource and budget usage",
        tag = "Workgroups",
        security = "bearerAuth"
    )]
    #[resp(200, "Workgroup usage record", json = WorkgroupUsageResponse)]
    async fn get_usage(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<WorkgroupUsageResponse>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(self.service.get_usage(&id).await?))
    }
}

#[controller("/api/v1/coordinator")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct CoordinatorController {
    service: Arc<WorkgroupService>,
}

#[routes]
impl CoordinatorController {
    #[post("/quotes")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "Request a signed price quote for a task before execution",
        tag = "Coordinator",
        security = "bearerAuth"
    )]
    #[body(json = RequestTaskQuoteDto)]
    #[resp(200, "Signed task price quote", json = TaskQuoteResponse)]
    async fn get_quote(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: RequestTaskQuoteDto,
    ) -> Result<Json<TaskQuoteResponse>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(self.service.calculate_quote(&dto).await?))
    }

    #[post("/checkout")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Settle payment or balance debit to unblock task execution",
        tag = "Coordinator",
        security = "bearerAuth"
    )]
    #[body(json = TaskCheckoutDto)]
    #[resp(200, "Settlement confirmation", json = TaskCheckoutResponse)]
    async fn checkout(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: TaskCheckoutDto,
    ) -> Result<Json<TaskCheckoutResponse>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.checkout_quote(&dto).await?))
    }
}

//! Thin Ironic HTTP controller for multi-level isolated secrets and secret groups.

use std::sync::Arc;

use ironic::prelude::*;
use pulsar_protocol::ApiKeyScope;

use crate::{
    infra::{
        error::{ErrorResponse, validate_dto},
        observability::PerRouteTimeoutInterceptor,
    },
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, ScopeGuard, require_scope},
        },
        secrets::{
            dto::{AddGroupMemberDto, CreateSecretGroupDto, UpsertSecretDto},
            entities::{SecretActionResponse, SecretGroupEntity, SecretMetadataEntity},
            services::SecretService,
        },
    },
};

#[derive(serde::Deserialize, garde::Validate, ironic::prelude::OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListSecretsQuery {
    #[garde(skip)]
    pub scope_type: Option<String>,
    #[garde(skip)]
    pub scope_id: Option<String>,
}

#[controller("/api/v1/secrets")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct SecretsController {
    service: Arc<SecretService>,
}

#[routes]
impl SecretsController {
    #[get("/")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "List secret metadata (names, scopes, target hosts; values never exposed)",
        tag = "Secrets",
        security = "bearerAuth"
    )]
    #[resp(200, "Secret metadata list", json = Vec<SecretMetadataEntity>)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    async fn list(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[query] query: ListSecretsQuery,
    ) -> Result<Json<Vec<SecretMetadataEntity>>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(
            self.service
                .list_metadata(
                    &actor.user_id,
                    query.scope_type.as_deref(),
                    query.scope_id.as_deref(),
                )
                .await?,
        ))
    }

    #[post("/")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Store an encrypted isolated secret (never readable back in plaintext)",
        tag = "Secrets",
        security = "bearerAuth"
    )]
    #[body(json = UpsertSecretDto)]
    #[resp(200, "Stored secret metadata", json = SecretMetadataEntity)]
    #[resp(403, "Forbidden reserved system secret", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    async fn upsert(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: UpsertSecretDto,
    ) -> Result<Json<SecretMetadataEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.upsert(&actor.user_id, &dto).await?))
    }

    #[post("/groups")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Create a shared secret group",
        tag = "Secrets",
        security = "bearerAuth"
    )]
    #[body(json = CreateSecretGroupDto)]
    #[resp(200, "Created secret group", json = SecretGroupEntity)]
    async fn create_group(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: CreateSecretGroupDto,
    ) -> Result<Json<SecretGroupEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.create_group(&actor.user_id, &dto).await?))
    }

    #[post("/groups/{id}/members")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Bind an agent, worker, or coordinator to a secret group",
        tag = "Secrets",
        security = "bearerAuth"
    )]
    #[body(json = AddGroupMemberDto)]
    #[resp(200, "Member bound to secret group", json = SecretActionResponse)]
    #[resp(404, "Group not found", json = ErrorResponse)]
    async fn add_group_member(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: AddGroupMemberDto,
    ) -> Result<Json<SecretActionResponse>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        self.service
            .add_group_member(&actor.user_id, &id, &dto)
            .await?;
        Ok(Json(SecretActionResponse { ok: true, id }))
    }

    #[delete("/{id}")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Delete an owner-scoped secret",
        tag = "Secrets",
        security = "bearerAuth"
    )]
    #[resp(200, "Secret deleted", json = SecretActionResponse)]
    #[resp(404, "Secret not found", json = ErrorResponse)]
    async fn delete_secret(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<SecretActionResponse>, HttpError> {
        require_scope(&actor, ApiKeyScope::Write)?;
        self.service.delete(&actor.user_id, &id).await?;
        Ok(Json(SecretActionResponse { ok: true, id }))
    }
}

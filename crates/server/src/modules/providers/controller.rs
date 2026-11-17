//! Thin Ironic HTTP controller for dynamic inference providers.

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
        providers::{
            dto::UpsertProviderDto,
            entities::{
                InferenceProviderEntity, ProviderActionResponse, ProviderHandshakeResponse,
                ProviderModelsCatalogResponse,
            },
            services::ProviderService,
        },
    },
};

#[controller("/api/v1/providers")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct ProvidersController {
    service: Arc<ProviderService>,
}

#[routes]
impl ProvidersController {
    #[get("/")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "List configured inference providers (API keys redacted)",
        tag = "Providers",
        security = "bearerAuth"
    )]
    #[resp(200, "Inference providers", json = Vec<InferenceProviderEntity>)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn list(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
    ) -> Result<Json<Vec<InferenceProviderEntity>>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(self.service.list().await?))
    }

    #[post("/")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Create or update an inference provider with Argon2id + AES-256-GCM encrypted API key",
        tag = "Providers",
        security = "bearerAuth"
    )]
    #[body(json = UpsertProviderDto)]
    #[resp(200, "Upserted inference provider", json = InferenceProviderEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn upsert(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: UpsertProviderDto,
    ) -> Result<Json<InferenceProviderEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.upsert(&dto).await?))
    }

    #[post("/{id}/default")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Set default inference provider",
        tag = "Providers",
        security = "bearerAuth"
    )]
    #[resp(200, "Default provider updated", json = ProviderActionResponse)]
    #[resp(404, "Provider not found", json = ErrorResponse)]
    async fn set_default(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<ProviderActionResponse>, HttpError> {
        require_scope(&actor, ApiKeyScope::Write)?;
        self.service.set_default(&id).await?;
        Ok(Json(ProviderActionResponse { ok: true, id }))
    }

    #[delete("/{id}")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Delete an inference provider",
        tag = "Providers",
        security = "bearerAuth"
    )]
    #[resp(200, "Provider deleted", json = ProviderActionResponse)]
    #[resp(404, "Provider not found", json = ErrorResponse)]
    #[resp(409, "Cannot delete last active provider", json = ErrorResponse)]
    async fn delete_provider(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<ProviderActionResponse>, HttpError> {
        require_scope(&actor, ApiKeyScope::Write)?;
        self.service.delete(&id).await?;
        Ok(Json(ProviderActionResponse { ok: true, id }))
    }

    #[get("/{id}/models")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "Dynamically discover available models from provider upstream GET /v1/models",
        tag = "Providers",
        security = "bearerAuth"
    )]
    #[resp(200, "Discovered models catalog", json = ProviderModelsCatalogResponse)]
    #[resp(404, "Provider not found", json = ErrorResponse)]
    async fn discover_models(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<ProviderModelsCatalogResponse>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(self.service.discover_models(&id).await?))
    }

    #[post("/{id}/test")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "Execute a 3-second handshake test against the upstream provider and classify errors",
        tag = "Providers",
        security = "bearerAuth"
    )]
    #[resp(200, "Handshake test result", json = ProviderHandshakeResponse)]
    #[resp(404, "Provider not found", json = ErrorResponse)]
    async fn test_provider(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<ProviderHandshakeResponse>, HttpError> {
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.test_handshake(&id).await?))
    }
}

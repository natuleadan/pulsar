//! Thin Ironic HTTP controller for session management and draft recovery.

use std::sync::Arc;

use agents_protocol::{ApiKeyScope, CursorPage};
use ironic::{Pagination, Response, prelude::*};

use crate::{
    infra::{
        error::{ErrorResponse, validate_dto},
        observability::{PerRouteTimeoutInterceptor, slice_paginated},
    },
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, ScopeGuard, require_scope},
        },
        sessions::{
            dto::{
                AppendSessionEventDto, ApproveSessionPlanDto, ContinueTurnDto,
                ContinueTurnResponse, CreateSessionDto, ImportSessionDto, ImportSessionResponse,
                ListSessionsQuery, PlanApprovalResponse, ProjectedConversationSnapshot,
                RevertSessionDto, RevertSessionResponse, SaveDraftDto, SearchSessionsQuery,
                SessionArchiveResponse, SessionEventLogEntry, SessionExportBundle,
                SessionSearchHit, UpdateSessionDto,
            },
            entities::{DraftSnapshot, SessionEntity},
            services::SessionService,
        },
    },
    openapi::PaginatedPageResponse,
};

#[controller("/api/v1/sessions")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct SessionsController {
    service: Arc<SessionService>,
}

#[routes]
impl SessionsController {
    #[post]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Create a session",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = CreateSessionDto)]
    #[resp(200, "Created session", json = SessionEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn create(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: CreateSessionDto,
    ) -> Result<Json<SessionEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let session = self.service.create(&actor, &dto).await?;
        Ok(Json(session))
    }

    #[get]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "List caller sessions",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Cursor-paginated sessions", json = CursorPage<SessionEntity>)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(408, "Request timeout", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn list(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[query] query: ListSessionsQuery,
    ) -> Result<Json<CursorPage<SessionEntity>>, HttpError> {
        validate_dto(&query)?;
        require_scope(&actor, ApiKeyScope::Read)?;
        let page = self.service.list(&actor, &query).await?;
        Ok(Json(page))
    }

    #[get("/page")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "List caller sessions with native offset pagination",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Paginated sessions", json = PaginatedPageResponse)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(408, "Request timeout", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn list_page(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[decorator(Pagination)] page: Pagination,
    ) -> Result<Response, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let query = ListSessionsQuery {
            limit: Some(100),
            cursor: None,
            archived: None,
        };
        let cursor_page = self.service.list(&actor, &query).await?;
        let (items, total) = slice_paginated(&cursor_page.items, page.offset(), page.limit());
        Response::paginated(&items, total, page.offset(), page.limit())
    }

    #[get("/:id")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "Get a session by ID",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Session record", json = SessionEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "BOLA ownership violation", json = ErrorResponse)]
    #[resp(404, "Session not found", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn get(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<SessionEntity>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let session = self.service.get_owned(&actor, &id).await?;
        Ok(Json(session))
    }

    #[patch("/:id")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Update a session",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = UpdateSessionDto)]
    #[resp(200, "Updated session", json = SessionEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Session not found", json = ErrorResponse)]
    #[resp(409, "Optimistic concurrency conflict", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn update(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: UpdateSessionDto,
    ) -> Result<Json<SessionEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let updated = self.service.update(&actor, &id, &dto).await?;
        Ok(Json(updated))
    }

    #[put("/:id/draft")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Persist a session prompt draft",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = SaveDraftDto)]
    #[resp(200, "Saved draft snapshot", json = DraftSnapshot)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Session not found", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn save_draft(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: SaveDraftDto,
    ) -> Result<Json<DraftSnapshot>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let draft = self.service.save_draft(&actor, &id, &dto).await?;
        Ok(Json(draft))
    }

    #[get("/:id/draft")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "Recover saved session draft",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Recovered draft snapshot", json = DraftSnapshot)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Draft not found", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn load_draft(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<DraftSnapshot>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let draft = self.service.load_draft(&actor, &id).await?;
        Ok(Json(draft))
    }

    #[delete("/:id")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Delete a session",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(204, "Session deleted")]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Session not found", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn delete(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<(), HttpError> {
        require_scope(&actor, ApiKeyScope::Write)?;
        self.service.delete(&actor, &id).await?;
        Ok(())
    }

    #[post("/:id/plan/approve")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Approve or reject a session plan gate to transition between Plan and Act modes",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = ApproveSessionPlanDto)]
    #[resp(200, "Plan approval verdict", json = PlanApprovalResponse)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Session not found", json = ErrorResponse)]
    async fn approve_plan(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: ApproveSessionPlanDto,
    ) -> Result<Json<PlanApprovalResponse>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.approve_plan(&actor, &id, &dto).await?))
    }

    #[post("/:id/continue")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Explicitly continue a two-level agent turn after step limit or human pause",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = ContinueTurnDto)]
    #[resp(200, "Turn continuation granted", json = ContinueTurnResponse)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Session not found", json = ErrorResponse)]
    async fn continue_turn(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: ContinueTurnDto,
    ) -> Result<Json<ContinueTurnResponse>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.continue_turn(&actor, &id, &dto).await?))
    }

    #[get("/search")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "Search sessions across titles and append-only event logs",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Matching sessions", json = Vec<SessionSearchHit>)]
    async fn search(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[query] query: SearchSessionsQuery,
    ) -> Result<Json<Vec<SessionSearchHit>>, HttpError> {
        validate_dto(&query)?;
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(self.service.search_sessions(&actor, &query).await?))
    }

    #[post("/import")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Import a faithful JSONL session bundle with automatic ID collision remapping",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = ImportSessionDto)]
    #[resp(200, "Imported session summary", json = ImportSessionResponse)]
    async fn import_session(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: ImportSessionDto,
    ) -> Result<Json<ImportSessionResponse>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.import_session(&actor, &dto).await?))
    }

    #[post("/:id/events")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Append an immutable event to the session event log",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = AppendSessionEventDto)]
    #[resp(200, "Appended session event", json = SessionEventLogEntry)]
    async fn append_event(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: AppendSessionEventDto,
    ) -> Result<Json<SessionEventLogEntry>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.append_event(&actor, &id, &dto).await?))
    }

    #[get("/:id/snapshot")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "Derive deterministic conversation state snapshot from append-only event log",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Projected conversation snapshot", json = ProjectedConversationSnapshot)]
    async fn get_snapshot(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<ProjectedConversationSnapshot>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(
            self.service.get_projected_snapshot(&actor, &id).await?,
        ))
    }

    #[post("/:id/revert")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Atomically roll back session conversation and VFS commit state to a target turn",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = RevertSessionDto)]
    #[resp(200, "Session revert summary", json = RevertSessionResponse)]
    async fn revert_session(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: RevertSessionDto,
    ) -> Result<Json<RevertSessionResponse>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.revert_to_turn(&actor, &id, &dto).await?))
    }

    #[post("/:id/export")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "Export faithful JSONL bundle of session metadata and event history",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Session JSONL export bundle", json = SessionExportBundle)]
    async fn export_session(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<SessionExportBundle>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(self.service.export_session(&actor, &id).await?))
    }

    #[post("/:id/archive")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Compress session event log with Zstandard into cold storage",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Cold archive summary", json = SessionArchiveResponse)]
    async fn archive_cold(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<SessionArchiveResponse>, HttpError> {
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.archive_cold(&actor, &id).await?))
    }

    #[post("/:id/unarchive")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Decompress and restore cold-archived session event log in place",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Unarchive restoration summary", json = SessionArchiveResponse)]
    async fn unarchive_cold(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<SessionArchiveResponse>, HttpError> {
        require_scope(&actor, ApiKeyScope::Write)?;
        Ok(Json(self.service.unarchive_cold(&actor, &id).await?))
    }
}

//! Thin Ironic HTTP controller for task artifact uploads and temporary presigned URLs.

use std::sync::Arc;

use agents_protocol::ApiKeyScope;
use ironic::{HeaderName, HeaderValue, HttpStatus, MultipartForm, MultipartFormData, prelude::*};
use serde::Deserialize;

use crate::{
    infra::error::ErrorResponse,
    modules::{
        artifacts::{
            dto::{
                ArtifactListResponse, ArtifactRecord, PresignQueryDto, PresignedArtifactResponse,
                UploadArtifactDto,
            },
            services::ArtifactService,
        },
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, ScopeGuard},
        },
    },
};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MultipartArtifactFields {
    #[serde(default)]
    ttl_secs: Option<u64>,
}

#[controller("/api/v1/tasks")]
#[derive(Injectable)]
pub struct ArtifactsController {
    service: Arc<ArtifactService>,
}

#[routes]
impl ArtifactsController {
    #[post("/:task_id/artifacts")]
    #[guard(AuthGuard)]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Upload a task output artifact to S3/RustFS and return a temporary presigned URL",
        tag = "Artifacts",
        security = "bearerAuth"
    )]
    #[resp(200, "Stored artifact record", json = ArtifactRecord)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    async fn upload_artifact(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param("task_id")] task_id: String,
        #[body] dto: UploadArtifactDto,
    ) -> Result<Json<ArtifactRecord>, HttpError> {
        Ok(Json(
            self.service.upload_as_actor(&actor, &task_id, &dto).await?,
        ))
    }

    #[post("/:task_id/artifacts/multipart")]
    #[guard(AuthGuard)]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Upload a raw task output artifact via multipart/form-data",
        tag = "Artifacts",
        security = "bearerAuth"
    )]
    #[resp(200, "Stored artifact record", json = ArtifactRecord)]
    #[resp(400, "Validation error", json = ErrorResponse)]
    async fn upload_artifact_multipart(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param("task_id")] task_id: String,
        #[decorator(MultipartForm::<MultipartArtifactFields>)] form: MultipartFormData<
            MultipartArtifactFields,
        >,
    ) -> Result<Json<ArtifactRecord>, HttpError> {
        let file = form.files.into_iter().next().ok_or_else(|| {
            HttpError::bad_request("missing_file", "missing multipart file field")
        })?;
        let filename = file
            .file_name
            .unwrap_or_else(|| format!("{}.bin", file.field_name));
        Ok(Json(
            self.service
                .upload_raw_as_actor(
                    &actor,
                    &task_id,
                    &filename,
                    file.content_type,
                    file.data,
                    form.fields.ttl_secs,
                )
                .await?,
        ))
    }

    #[post("/:task_id/artifacts/worker")]
    #[api(
        summary = "Internal Boxer worker endpoint to upload packaged task artifacts",
        tag = "Artifacts"
    )]
    #[resp(200, "Stored artifact record", json = ArtifactRecord)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    async fn upload_artifact_worker(
        &self,
        #[header("x-boxer-worker-token")] worker_tok: String,
        #[param("task_id")] task_id: String,
        #[body] dto: UploadArtifactDto,
    ) -> Result<Json<ArtifactRecord>, HttpError> {
        Ok(Json(
            self.service
                .upload_as_worker(&worker_tok, &task_id, &dto)
                .await?,
        ))
    }

    #[get("/:task_id/artifacts")]
    #[guard(AuthGuard)]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "List artifacts for a task with fresh temporary presigned URLs",
        tag = "Artifacts",
        security = "bearerAuth"
    )]
    #[resp(200, "Task artifact list", json = ArtifactListResponse)]
    async fn list_artifacts(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param("task_id")] task_id: String,
        #[query] query: PresignQueryDto,
    ) -> Result<Json<ArtifactListResponse>, HttpError> {
        Ok(Json(
            self.service
                .list_for_task(&actor, &task_id, query.ttl)
                .await?,
        ))
    }

    #[get("/:task_id/artifacts/:filename/presign")]
    #[guard(AuthGuard)]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "Generate a temporary presigned URL for a task artifact",
        tag = "Artifacts",
        security = "bearerAuth"
    )]
    #[resp(200, "Presigned artifact URL", json = PresignedArtifactResponse)]
    async fn presign_artifact(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param("task_id")] task_id: String,
        #[param("filename")] filename: String,
        #[query] query: PresignQueryDto,
    ) -> Result<Json<PresignedArtifactResponse>, HttpError> {
        Ok(Json(
            self.service
                .presign_artifact(&actor, &task_id, &filename, query.ttl)
                .await?,
        ))
    }

    #[get("/:task_id/artifacts/:filename/download")]
    #[guard(AuthGuard)]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "Download raw task artifact bytes through authenticated proxy",
        tag = "Artifacts",
        security = "bearerAuth"
    )]
    #[resp(200, "Raw artifact bytes", json = Vec<u8>)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Not found", json = ErrorResponse)]
    async fn download_artifact(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param("task_id")] task_id: String,
        #[param("filename")] filename: String,
    ) -> Result<Response, HttpError> {
        let (content_type, bytes) = self
            .service
            .download_bytes(&actor, &task_id, &filename)
            .await?;
        let mut resp = Response::bytes(HttpStatus::OK, bytes);
        if let Ok(val) = HeaderValue::from_str(&content_type) {
            resp.headers_mut()
                .insert(HeaderName::from_static("content-type"), val);
        }
        Ok(resp)
    }
}

//! Task artifact storage, S3/RustFS presigned URLs, and BOLA-checked downloads.

pub mod controller;
pub mod dto;
pub mod services;

use ironic::prelude::*;

pub use controller::ArtifactsController;
pub use services::ArtifactService;

/// Registers the artifact storage service and HTTP controller.
#[derive(Module)]
#[module(
    imports = [
        crate::app::InfraModule,
        crate::modules::tasks::TasksModule,
        crate::modules::audit::AuditModule,
    ],
    providers = [ArtifactService],
    controllers = [ArtifactsController],
    exports = [ArtifactService],
)]
pub struct ArtifactsModule;

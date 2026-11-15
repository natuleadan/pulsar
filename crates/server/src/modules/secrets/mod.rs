//! Multi-level isolated user/agent/worker secrets and shared secret groups.

pub mod controller;
pub mod dto;
pub mod entities;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::SecretsController;
pub use repositories::{
    InMemorySecretRepository, PgSecretRepository, SecretRepository, SecretStore,
};
pub use services::SecretService;

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule],
    providers = [SecretService],
    controllers = [SecretsController],
    exports = [SecretService],
)]
pub struct SecretsModule;

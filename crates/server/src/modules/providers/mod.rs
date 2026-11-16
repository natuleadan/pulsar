//! Dynamic inference providers module (Argon2id + AES-256-GCM encrypted API keys).

pub mod controller;
pub mod crypto;
pub mod dto;
pub mod entities;
pub mod harness;
pub mod instances;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::ProvidersController;
pub use repositories::{
    InMemoryProviderRepository, PgProviderRepository, ProviderRepository, ProviderStore,
};
pub use services::ProviderService;

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule],
    providers = [ProviderService],
    controllers = [ProvidersController],
    exports = [ProviderService],
)]
pub struct ProvidersModule;

//! Multi-tenant workgroups and coordinator monetization module.

pub mod controller;
pub mod dto;
pub mod services;

use ironic::prelude::*;

pub use controller::{CoordinatorController, WorkgroupsController};
pub use services::WorkgroupService;

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule],
    providers = [WorkgroupService],
    controllers = [WorkgroupsController, CoordinatorController],
    exports = [WorkgroupService],
)]
pub struct WorkgroupsModule;

//! Skills management module: dynamic installation, 4 built-in system skills,
//! and prompt context budget shaping.

pub mod controller;
pub mod services;

pub use controller::SkillController;
pub use services::{SkillService, SkillStore};

use ironic::prelude::*;

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule],
    providers = [SkillService],
    controllers = [SkillController],
    exports = [SkillService]
)]
pub struct SkillsModule;

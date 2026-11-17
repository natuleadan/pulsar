//! External robot and IoT device ingress gateway module.

pub mod controller;
pub mod pairing;
pub mod services;

pub use controller::RobotGatewayController;
pub use pairing::{PairedDeviceSession, PairingError, PairingGateway};
pub use services::{RobotGatewayService, RobotRegistryStore};

use ironic::prelude::*;

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule],
    providers = [RobotGatewayService],
    controllers = [RobotGatewayController],
    exports = [RobotGatewayService]
)]
pub struct RobotsModule;

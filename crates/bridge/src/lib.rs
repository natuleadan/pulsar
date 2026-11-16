//! Dedicated local hardware, robotics, IoT device bridge, and zero-trust LAN shield.

#![forbid(unsafe_code)]

pub mod discovery;
pub mod emergency;
pub mod pairing;
pub mod ros2;
pub mod security;
pub mod serial;

pub use discovery::{DiscoveredLocalDevice, LocalDeviceRegistry};
pub use emergency::{DEFAULT_DEADMAN_TIMEOUT_MS, DeadManWatchdog};
pub use pairing::{PairedDeviceSession, PairingError, PairingGateway};
pub use ros2::{Ros2TopicBridge, TwistMessage, Vector3, command_frame_to_twist};
pub use security::{
    BridgeSecurityError, is_private_lan_ip, validate_geofence, validate_outbound_ip,
    validate_twist_limits,
};
pub use serial::{SerialBridgeError, SerialBridgeManager, SerialDeviceStatus, SerialPortConfig};

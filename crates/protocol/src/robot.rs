//! Protocol types and data transfer objects for external robot/IoT device ingress.

use serde::{Deserialize, Serialize};

/// Handshake payload sent by an external robot or edge IoT hardware device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHandshake {
    pub device_id: String,
    pub device_type: String,
    pub public_key_ed25519: String,
    pub firmware_version: String,
    pub signature: String,
}

/// Registration response issued by the Server Perimeter Guardian to an authenticated device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRegistrationResponse {
    pub device_id: String,
    pub session_token: String,
    pub lease_ttl_secs: u64,
    pub ingress_ws_endpoint: String,
}

/// Inbound telemetry frame emitted by hardware devices towards the server gateway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RobotTelemetryFrame {
    pub device_id: String,
    pub timestamp_ms: u64,
    pub sensor_data: serde_json::Value,
    pub emergency_stop_status: bool,
    pub battery_percentage: Option<f32>,
}

/// Outbound command frame dispatched by server or Boxer agents towards the robot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RobotCommandFrame {
    pub command_id: String,
    pub device_id: String,
    pub action: String,
    pub parameters: serde_json::Value,
    pub emergency_stop_override: bool,
    pub issued_at_ms: u64,
}

/// Safety alert emitted when an emergency stop or geofence violation trips.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RobotSafetyAlert {
    pub device_id: String,
    pub reason: String,
    pub tripped_at_ms: u64,
    pub active_emergency_stop: bool,
}

/// Status summary for a registered robot device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RobotDeviceSummary {
    pub device_id: String,
    pub device_type: String,
    pub firmware_version: String,
    pub connected: bool,
    pub emergency_stop_active: bool,
    pub last_seen_ms: u64,
}

/// NATS subjects for robot gateway messaging.
pub const NATS_SUBJECT_ROBOT_TELEMETRY_PREFIX: &str = "robot.telemetry";
pub const NATS_SUBJECT_ROBOT_COMMANDS_PREFIX: &str = "robot.commands";
pub const NATS_SUBJECT_ROBOT_SAFETY_ALERTS: &str = "robot.safety.alerts";

#[must_use]
pub fn robot_telemetry_subject(device_id: &str) -> String {
    format!("{NATS_SUBJECT_ROBOT_TELEMETRY_PREFIX}.{device_id}")
}

#[must_use]
pub fn robot_command_subject(device_id: &str) -> String {
    format!("{NATS_SUBJECT_ROBOT_COMMANDS_PREFIX}.{device_id}")
}

#[cfg(feature = "openapi")]
impl ironic::OpenApiSchema for DeviceHandshake {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "DeviceHandshake",
            "required": ["deviceId", "deviceType", "publicKeyEd25519", "firmwareVersion", "signature"],
            "properties": {
                "deviceId": { "type": "string", "example": "robot-quad-01" },
                "deviceType": { "type": "string", "example": "quadruped" },
                "publicKeyEd25519": { "type": "string", "example": "e4d9b8a7c6..." },
                "firmwareVersion": { "type": "string", "example": "1.4.2" },
                "signature": { "type": "string", "example": "sig_ed25519_..." }
            }
        })
    }
}

#[cfg(feature = "openapi")]
impl ironic::OpenApiSchema for DeviceRegistrationResponse {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "DeviceRegistrationResponse",
            "required": ["deviceId", "sessionToken", "leaseTtlSecs", "ingressWsEndpoint"],
            "properties": {
                "deviceId": { "type": "string", "example": "robot-quad-01" },
                "sessionToken": { "type": "string", "example": "rbt_tok_9b8a7c6d" },
                "leaseTtlSecs": { "type": "integer", "example": 3600 },
                "ingressWsEndpoint": { "type": "string", "example": "/api/v1/robots/robot-quad-01/ws" }
            }
        })
    }
}

#[cfg(feature = "openapi")]
impl ironic::OpenApiSchema for RobotTelemetryFrame {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "RobotTelemetryFrame",
            "required": ["deviceId", "timestampMs", "sensorData", "emergencyStopStatus"],
            "properties": {
                "deviceId": { "type": "string", "example": "robot-quad-01" },
                "timestampMs": { "type": "integer", "example": 1731600000000_u64 },
                "sensorData": { "type": "object" },
                "emergencyStopStatus": { "type": "boolean", "example": false },
                "batteryPercentage": { "type": "number", "example": 87.5 }
            }
        })
    }
}

#[cfg(feature = "openapi")]
impl ironic::OpenApiSchema for RobotCommandFrame {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "RobotCommandFrame",
            "required": ["commandId", "deviceId", "action", "parameters", "emergencyStopOverride", "issuedAtMs"],
            "properties": {
                "commandId": { "type": "string", "example": "cmd_01j8xyz" },
                "deviceId": { "type": "string", "example": "robot-quad-01" },
                "action": { "type": "string", "example": "move_trajectory" },
                "parameters": { "type": "object" },
                "emergencyStopOverride": { "type": "boolean", "example": false },
                "issuedAtMs": { "type": "integer", "example": 1731600000000_u64 }
            }
        })
    }
}

#[cfg(feature = "openapi")]
impl ironic::OpenApiSchema for RobotSafetyAlert {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "RobotSafetyAlert",
            "required": ["deviceId", "reason", "trippedAtMs", "activeEmergencyStop"],
            "properties": {
                "deviceId": { "type": "string", "example": "robot-quad-01" },
                "reason": { "type": "string", "example": "Hardware E-STOP button pressed" },
                "trippedAtMs": { "type": "integer", "example": 1731600000000_u64 },
                "activeEmergencyStop": { "type": "boolean", "example": true }
            }
        })
    }
}

#[cfg(feature = "openapi")]
impl ironic::OpenApiSchema for RobotDeviceSummary {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "RobotDeviceSummary",
            "required": ["deviceId", "deviceType", "firmwareVersion", "connected", "emergencyStopActive", "lastSeenMs"],
            "properties": {
                "deviceId": { "type": "string", "example": "robot-quad-01" },
                "deviceType": { "type": "string", "example": "quadruped" },
                "firmwareVersion": { "type": "string", "example": "1.4.2" },
                "connected": { "type": "boolean", "example": true },
                "emergencyStopActive": { "type": "boolean", "example": false },
                "lastSeenMs": { "type": "integer", "example": 1731600000000_u64 }
            }
        })
    }
}

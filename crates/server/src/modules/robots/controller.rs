//! HTTP and WebSocket gateway controller for hardware robot/IoT devices.

use std::sync::Arc;

use agents_protocol::{
    DeviceHandshake, DeviceRegistrationResponse, RobotCommandFrame, RobotDeviceSummary,
    RobotSafetyAlert, RobotTelemetryFrame,
};
use ironic::Json;
use ironic::prelude::*;

use super::services::RobotGatewayService;

/// DTO for hardware robot registration handshake.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, garde::Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHandshakeDto {
    #[garde(skip)]
    pub device_id: String,
    #[garde(skip)]
    pub device_type: String,
    #[garde(skip)]
    pub public_key_ed25519: String,
    #[garde(skip)]
    pub firmware_version: String,
    #[garde(skip)]
    pub signature: String,
}

impl From<DeviceHandshakeDto> for DeviceHandshake {
    fn from(dto: DeviceHandshakeDto) -> Self {
        Self {
            device_id: dto.device_id,
            device_type: dto.device_type,
            public_key_ed25519: dto.public_key_ed25519,
            firmware_version: dto.firmware_version,
            signature: dto.signature,
        }
    }
}

/// DTO for ingesting hardware telemetry via HTTP API.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct IngestTelemetryDto {
    #[garde(skip)]
    pub session_token: String,
    #[garde(skip)]
    pub frame: RobotTelemetryFrame,
}

impl ironic::OpenApiSchema for IngestTelemetryDto {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "IngestTelemetryDto",
            "required": ["sessionToken", "frame"],
            "properties": {
                "sessionToken": { "type": "string", "example": "rbt_tok_9b8a7c6d..." },
                "frame": { "$ref": "#/components/schemas/RobotTelemetryFrame" }
            }
        })
    }
}

/// DTO for dispatching a motion/task command to a hardware robot.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueCommandDto {
    #[garde(skip)]
    pub action: String,
    #[garde(skip)]
    pub parameters: serde_json::Value,
    #[garde(skip)]
    pub emergency_stop_override: Option<bool>,
}

impl ironic::OpenApiSchema for EnqueueCommandDto {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "EnqueueCommandDto",
            "required": ["action"],
            "properties": {
                "action": { "type": "string", "example": "move_base" },
                "parameters": { "type": "object" },
                "emergencyStopOverride": { "type": "boolean", "example": false }
            }
        })
    }
}

/// DTO for manually tripping the robot emergency stop circuit breaker.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, garde::Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct TripEmergencyStopDto {
    #[garde(skip)]
    pub reason: String,
}

/// Status response for generic robot operations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct RobotActionResponse {
    pub status: String,
    pub device_id: String,
}

#[controller("/api/v1/robots")]
#[derive(Injectable)]
pub struct RobotGatewayController {
    service: Arc<RobotGatewayService>,
}

#[routes]
impl RobotGatewayController {
    #[post("/register")]
    #[api(
        summary = "Register external hardware robot with Ed25519 handshake",
        tag = "Robots"
    )]
    #[body(json = DeviceHandshakeDto)]
    #[resp(200, "Device registration confirmation", json = DeviceRegistrationResponse)]
    pub async fn register(
        &self,
        #[body] dto: DeviceHandshakeDto,
    ) -> Result<Json<DeviceRegistrationResponse>, HttpError> {
        let resp = self.service.register_device(dto.into()).await?;
        Ok(Json(resp))
    }

    #[post("/:device_id/telemetry")]
    #[api(
        summary = "Ingest hardware telemetry and check emergency stop",
        tag = "Robots"
    )]
    #[body(json = IngestTelemetryDto)]
    #[resp(200, "Telemetry processed", json = RobotActionResponse)]
    pub async fn telemetry(
        &self,
        #[param] device_id: String,
        #[body] dto: IngestTelemetryDto,
    ) -> Result<Json<RobotActionResponse>, HttpError> {
        self.service
            .ingest_telemetry(&device_id, &dto.session_token, dto.frame)
            .await?;
        Ok(Json(RobotActionResponse {
            status: "ok".into(),
            device_id,
        }))
    }

    #[get("/:device_id/telemetry")]
    #[api(summary = "Get most recent telemetry frame from robot", tag = "Robots")]
    #[resp(200, "Latest telemetry frame", json = RobotTelemetryFrame)]
    pub async fn get_telemetry(
        &self,
        #[param] device_id: String,
    ) -> Result<Json<RobotTelemetryFrame>, HttpError> {
        let frame = self.service.get_telemetry(&device_id).await?;
        Ok(Json(frame))
    }

    #[post("/:device_id/commands")]
    #[api(summary = "Enqueue motion command for hardware robot", tag = "Robots")]
    #[body(json = EnqueueCommandDto)]
    #[resp(200, "Command enqueued", json = RobotActionResponse)]
    pub async fn enqueue_command(
        &self,
        #[param] device_id: String,
        #[body] dto: EnqueueCommandDto,
    ) -> Result<Json<RobotActionResponse>, HttpError> {
        let cmd = RobotCommandFrame {
            command_id: format!("cmd_{:x}", crate::infra::state::now_ms()),
            device_id: device_id.clone(),
            action: dto.action,
            parameters: dto.parameters,
            emergency_stop_override: dto.emergency_stop_override.unwrap_or(false),
            issued_at_ms: crate::infra::state::now_ms(),
        };
        self.service.enqueue_command(cmd).await?;
        Ok(Json(RobotActionResponse {
            status: "enqueued".into(),
            device_id,
        }))
    }

    #[post("/:device_id/emergency-stop")]
    #[api(
        summary = "Trip immediate emergency stop circuit breaker",
        tag = "Robots"
    )]
    #[body(json = TripEmergencyStopDto)]
    #[resp(200, "Emergency stop safety alert", json = RobotSafetyAlert)]
    pub async fn emergency_stop(
        &self,
        #[param] device_id: String,
        #[body] dto: TripEmergencyStopDto,
    ) -> Result<Json<RobotSafetyAlert>, HttpError> {
        let alert = self
            .service
            .trip_emergency_stop(&device_id, &dto.reason)
            .await?;
        Ok(Json(alert))
    }

    #[post("/:device_id/resume")]
    #[api(
        summary = "Reset emergency stop circuit breaker after inspection",
        tag = "Robots"
    )]
    #[resp(200, "Device resumed", json = RobotActionResponse)]
    pub async fn resume(
        &self,
        #[param] device_id: String,
    ) -> Result<Json<RobotActionResponse>, HttpError> {
        self.service.reset_emergency_stop(&device_id).await?;
        Ok(Json(RobotActionResponse {
            status: "resumed".into(),
            device_id,
        }))
    }

    #[get]
    #[api(
        summary = "List all registered robot devices and status",
        tag = "Robots"
    )]
    #[resp(200, "Registered devices list", json = Vec<RobotDeviceSummary>)]
    pub async fn list(&self) -> Result<Json<Vec<RobotDeviceSummary>>, HttpError> {
        let list = self.service.list_devices().await;
        Ok(Json(list))
    }
}

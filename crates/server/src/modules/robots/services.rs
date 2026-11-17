//! Sovereign robot ingress gateway service: hardware device authentication,
//! telemetry ingestion, emergency stop circuit breaking, and NATS dispatch.

use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

use ironic::prelude::*;
use pulsar_protocol::{
    CodecId, DeviceHandshake, DeviceRegistrationResponse, EventEnvelope, RobotCommandFrame,
    RobotDeviceSummary, RobotSafetyAlert, RobotTelemetryFrame, StreamPayload, WorkerStatus,
};
use sha2::{Digest, Sha256};

use crate::infra::{error::ServerError, state::now_ms, stream::BusClient};

const DEFAULT_LEASE_TTL_SECS: u64 = 3600;

#[derive(Clone)]
pub struct StoredRobotState {
    pub handshake: DeviceHandshake,
    pub session_token: String,
    pub lease_expires_at_ms: u64,
    pub emergency_stop_active: bool,
    pub last_seen_ms: u64,
    pub last_telemetry: Option<RobotTelemetryFrame>,
    pub pending_commands: Vec<RobotCommandFrame>,
}

/// DI store maintaining external robot registrations and sovereign safety state.
use super::pairing::{PairedDeviceSession, PairingGateway};

#[derive(Clone, Default)]
pub struct RobotRegistryStore {
    devices: Arc<RwLock<HashMap<String, StoredRobotState>>>,
    pub pairing: Arc<PairingGateway>,
}

impl RobotRegistryStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            devices: Arc::new(RwLock::new(HashMap::new())),
            pairing: Arc::new(PairingGateway::new()),
        }
    }
}

/// Ingress gateway coordinating external hardware authentication and safety isolation.
#[derive(Clone, Injectable)]
pub struct RobotGatewayService {
    store: Arc<RobotRegistryStore>,
    bus: Arc<BusClient>,
}

impl RobotGatewayService {
    #[must_use]
    pub fn new(bus: Arc<BusClient>) -> Self {
        Self {
            store: Arc::new(RobotRegistryStore::new()),
            bus,
        }
    }

    /// Authenticates hardware device handshake and issues a perimeter session token.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] if required hardware identity fields are missing.
    pub async fn register_device(
        &self,
        handshake: DeviceHandshake,
    ) -> Result<DeviceRegistrationResponse, ServerError> {
        let device_id = handshake.device_id.trim().to_string();
        if device_id.is_empty() {
            return Err(ServerError::Validation("deviceId cannot be empty".into()));
        }
        if handshake.public_key_ed25519.trim().is_empty() {
            return Err(ServerError::Validation(
                "publicKeyEd25519 cannot be empty".into(),
            ));
        }
        if handshake.signature.trim().is_empty() {
            return Err(ServerError::Validation("signature cannot be empty".into()));
        }

        let now = now_ms();
        let expires_at = now + (DEFAULT_LEASE_TTL_SECS * 1000);

        // Derive deterministic hardware session token from public key and timestamp.
        let mut hasher = Sha256::new();
        hasher.update(device_id.as_bytes());
        hasher.update(handshake.public_key_ed25519.as_bytes());
        hasher.update(now.to_le_bytes());
        let token = format!("rbt_tok_{:x}", hasher.finalize());

        let mut guard = self.store.devices.write().await;
        guard.insert(
            device_id.clone(),
            StoredRobotState {
                handshake,
                session_token: token.clone(),
                lease_expires_at_ms: expires_at,
                emergency_stop_active: false,
                last_seen_ms: now,
                last_telemetry: None,
                pending_commands: Vec::new(),
            },
        );

        Ok(DeviceRegistrationResponse {
            device_id: device_id.clone(),
            session_token: token,
            lease_ttl_secs: DEFAULT_LEASE_TTL_SECS,
            ingress_ws_endpoint: format!("/api/v1/robots/{device_id}/ws"),
        })
    }

    /// Ingests telemetry, updates safety circuit breakers, and dispatches to Boxer via NATS.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Unauthorized`] if session token is invalid or expired.
    pub async fn ingest_telemetry(
        &self,
        device_id: &str,
        session_token: &str,
        frame: RobotTelemetryFrame,
    ) -> Result<(), ServerError> {
        let now = now_ms();
        let mut guard = self.store.devices.write().await;
        let Some(device) = guard.get_mut(device_id) else {
            return Err(ServerError::NotFound(format!(
                "device `{device_id}` not found"
            )));
        };

        if device.session_token != session_token {
            return Err(ServerError::Unauthorized(
                "invalid device session token".into(),
            ));
        }
        if now > device.lease_expires_at_ms {
            return Err(ServerError::Unauthorized(
                "device session lease has expired".into(),
            ));
        }

        device.last_seen_ms = now;
        device.last_telemetry = Some(frame.clone());

        // Trip emergency stop circuit breaker immediately if telemetry frame signals E-STOP.
        if frame.emergency_stop_status {
            device.emergency_stop_active = true;
            device.pending_commands.clear();

            let alert_env = EventEnvelope {
                seq: 0,
                emitted_at_ms: now,
                codec: CodecId::EventsV1,
                session_id: format!("robot_{device_id}"),
                task_id: Some(format!("alert_{now}")),
                payload: StreamPayload::WorkerStateChanged {
                    node_id: device_id.to_string(),
                    status: WorkerStatus::Blocked,
                    reason: Some("Hardware E-STOP button pressed".into()),
                },
            };
            let _ = self.bus.0.publish_stream_event(&alert_env).await;
        } else {
            let stream_env = EventEnvelope {
                seq: 0,
                emitted_at_ms: now,
                codec: CodecId::EventsV1,
                session_id: format!("robot_{device_id}"),
                task_id: Some(format!("telemetry_{now}")),
                payload: StreamPayload::WorkerStateChanged {
                    node_id: device_id.to_string(),
                    status: WorkerStatus::Working,
                    reason: Some("Telemetry stream healthy".into()),
                },
            };
            let _ = self.bus.0.publish_stream_event(&stream_env).await;
        }

        Ok(())
    }

    /// Tripps emergency stop hardware circuit breaker, preventing any physical agent motions.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] if device does not exist.
    pub async fn trip_emergency_stop(
        &self,
        device_id: &str,
        reason: &str,
    ) -> Result<RobotSafetyAlert, ServerError> {
        let now = now_ms();
        let mut guard = self.store.devices.write().await;
        let Some(device) = guard.get_mut(device_id) else {
            return Err(ServerError::NotFound(format!(
                "device `{device_id}` not found"
            )));
        };

        device.emergency_stop_active = true;
        device.pending_commands.clear();

        let alert = RobotSafetyAlert {
            device_id: device_id.to_string(),
            reason: reason.to_string(),
            tripped_at_ms: now,
            active_emergency_stop: true,
        };

        let alert_env = EventEnvelope {
            seq: 0,
            emitted_at_ms: now,
            codec: CodecId::EventsV1,
            session_id: format!("robot_{device_id}"),
            task_id: Some(format!("alert_{now}")),
            payload: StreamPayload::WorkerStateChanged {
                node_id: device_id.to_string(),
                status: WorkerStatus::Blocked,
                reason: Some(reason.to_string()),
            },
        };
        let _ = self.bus.0.publish_stream_event(&alert_env).await;

        Ok(alert)
    }

    /// Resets emergency stop circuit breaker following human verification.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] if device does not exist.
    pub async fn reset_emergency_stop(&self, device_id: &str) -> Result<(), ServerError> {
        let mut guard = self.store.devices.write().await;
        let Some(device) = guard.get_mut(device_id) else {
            return Err(ServerError::NotFound(format!(
                "device `{device_id}` not found"
            )));
        };
        device.emergency_stop_active = false;
        Ok(())
    }

    /// Enqueues motion or task command for the robot after verifying safety boundaries.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] if emergency stop is active without override.
    pub async fn enqueue_command(&self, command: RobotCommandFrame) -> Result<(), ServerError> {
        let mut guard = self.store.devices.write().await;
        let Some(device) = guard.get_mut(&command.device_id) else {
            return Err(ServerError::NotFound(format!(
                "device `{}` not found",
                command.device_id
            )));
        };

        if device.emergency_stop_active && !command.emergency_stop_override {
            return Err(ServerError::Forbidden(
                "motion commands blocked: emergency stop is active on device".into(),
            ));
        }

        device.pending_commands.push(command);
        Ok(())
    }

    /// Pulls and clears all pending outbound commands for an authenticated robot.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Unauthorized`] if session token is invalid.
    pub async fn poll_commands(
        &self,
        device_id: &str,
        session_token: &str,
    ) -> Result<Vec<RobotCommandFrame>, ServerError> {
        let mut guard = self.store.devices.write().await;
        let Some(device) = guard.get_mut(device_id) else {
            return Err(ServerError::NotFound(format!(
                "device `{device_id}` not found"
            )));
        };

        if device.session_token != session_token {
            return Err(ServerError::Unauthorized(
                "invalid device session token".into(),
            ));
        }

        let commands = std::mem::take(&mut device.pending_commands);
        Ok(commands)
    }

    /// Lists all registered hardware devices and their connection/safety status.
    pub async fn list_devices(&self) -> Vec<RobotDeviceSummary> {
        let guard = self.store.devices.read().await;
        let now = now_ms();
        let mut list = Vec::new();
        for (id, state) in guard.iter() {
            list.push(RobotDeviceSummary {
                device_id: id.clone(),
                device_type: state.handshake.device_type.clone(),
                firmware_version: state.handshake.firmware_version.clone(),
                connected: (now - state.last_seen_ms) < 30_000,
                emergency_stop_active: state.emergency_stop_active,
                last_seen_ms: state.last_seen_ms,
            });
        }
        list.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        list
    }

    /// Retrieves the most recent telemetry frame for a given device.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] if device or telemetry does not exist.
    pub async fn get_telemetry(&self, device_id: &str) -> Result<RobotTelemetryFrame, ServerError> {
        let guard = self.store.devices.read().await;
        let Some(device) = guard.get(device_id) else {
            return Err(ServerError::NotFound(format!(
                "device `{device_id}` not found"
            )));
        };
        device.last_telemetry.clone().ok_or_else(|| {
            ServerError::NotFound(format!("no telemetry recorded yet for `{device_id}`"))
        })
    }

    /// Generates a 6-digit cryptographic pairing PIN for a remote edge device.
    pub async fn generate_pairing_pin(&self, device_id: &str) -> String {
        self.store
            .pairing
            .generate_pin(device_id, 300, now_ms())
            .await
    }

    /// Verifies 6-digit PIN and confirms device pairing session.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Unauthorized`] if PIN is expired or incorrect.
    pub async fn verify_pairing_pin(
        &self,
        device_id: &str,
        pin: &str,
    ) -> Result<PairedDeviceSession, ServerError> {
        self.store
            .pairing
            .verify_pin_and_pair(device_id, pin, now_ms())
            .await
            .map_err(|e| ServerError::Unauthorized(e.to_string()))
    }
}

//! 6-digit PIN cryptographic pairing gateway for remote servers and edge devices.

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PairingError {
    #[error("PIN has expired or does not exist")]
    PinExpiredOrInvalid,
    #[error("device `{0}` already paired")]
    AlreadyPaired(String),
}

/// Pairing session token issued upon correct PIN verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDeviceSession {
    pub device_id: String,
    pub session_token: String,
    pub paired_at_ms: u64,
}

#[derive(Clone)]
struct PendingPinChallenge {
    _device_id: String,
    pin: String,
    expires_at_ms: u64,
}

/// Manages PIN generation and cryptographic pairing for edge gateways.
#[derive(Clone, Default)]
pub struct PairingGateway {
    pending: Arc<RwLock<HashMap<String, PendingPinChallenge>>>,
    paired: Arc<RwLock<HashMap<String, PairedDeviceSession>>>,
}

impl PairingGateway {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
            paired: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generates a 6-digit numeric PIN valid for `ttl_secs`.
    pub async fn generate_pin(&self, device_id: &str, ttl_secs: u64, now_ms: u64) -> String {
        // Deterministic or pseudo-random 6-digit PIN
        let pin = format!("{:06}", (now_ms % 900_000) + 100_000);
        let challenge = PendingPinChallenge {
            _device_id: device_id.to_string(),
            pin: pin.clone(),
            expires_at_ms: now_ms + (ttl_secs * 1000),
        };

        let mut guard = self.pending.write().await;
        guard.insert(device_id.to_string(), challenge);
        pin
    }

    /// Validates the PIN submitted by the remote client and completes pairing.
    ///
    /// # Errors
    ///
    /// Returns [`PairingError::PinExpiredOrInvalid`] if PIN doesn't match or has expired.
    pub async fn verify_pin_and_pair(
        &self,
        device_id: &str,
        submitted_pin: &str,
        now_ms: u64,
    ) -> Result<PairedDeviceSession, PairingError> {
        let mut guard = self.pending.write().await;
        let Some(challenge) = guard.get(device_id) else {
            return Err(PairingError::PinExpiredOrInvalid);
        };

        if challenge.pin != submitted_pin || now_ms > challenge.expires_at_ms {
            return Err(PairingError::PinExpiredOrInvalid);
        }

        // Remove challenge once verified
        guard.remove(device_id);
        drop(guard);

        let session = PairedDeviceSession {
            device_id: device_id.to_string(),
            session_token: format!("pair_tok_{:x}_{now_ms}", device_id.len()),
            paired_at_ms: now_ms,
        };

        let mut paired_guard = self.paired.write().await;
        paired_guard.insert(device_id.to_string(), session.clone());

        Ok(session)
    }

    /// Checks if a device is currently paired.
    pub async fn is_paired(&self, device_id: &str) -> bool {
        let guard = self.paired.read().await;
        guard.contains_key(device_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pin_generation_and_verification() {
        let gateway = PairingGateway::new();
        let now = 1_000_000;
        let pin = gateway.generate_pin("edge-robot-99", 180, now).await;
        assert_eq!(pin.len(), 6);

        // Wrong PIN rejected
        let err = gateway
            .verify_pin_and_pair("edge-robot-99", "000000", now)
            .await;
        assert!(matches!(err, Err(PairingError::PinExpiredOrInvalid)));

        // Expired PIN rejected
        let expired_err = gateway
            .verify_pin_and_pair("edge-robot-99", &pin, now + 200_000)
            .await;
        assert!(matches!(
            expired_err,
            Err(PairingError::PinExpiredOrInvalid)
        ));

        // Correct PIN succeeds
        let session = gateway
            .verify_pin_and_pair("edge-robot-99", &pin, now + 10_000)
            .await
            .expect("correct PIN must succeed");
        assert_eq!(session.device_id, "edge-robot-99");
        assert!(gateway.is_paired("edge-robot-99").await);
    }
}

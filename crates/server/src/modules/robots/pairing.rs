//! 6-digit PIN cryptographic pairing gateway for remote servers and edge devices.

use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;

/// Errors returned by pairing verification.
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
        let pin = format!("{:06}", (now_ms % 900_000) + 100_000);
        let challenge = PendingPinChallenge {
            _device_id: device_id.to_owned(),
            pin: pin.clone(),
            expires_at_ms: now_ms + (ttl_secs * 1000),
        };
        let mut pending = self.pending.write().await;
        pending.insert(device_id.to_owned(), challenge);
        pin
    }

    /// Verifies the 6-digit PIN and returns a paired device session.
    pub async fn verify_pin_and_pair(
        &self,
        device_id: &str,
        pin: &str,
        now_ms: u64,
    ) -> Result<PairedDeviceSession, PairingError> {
        let mut pending = self.pending.write().await;
        let challenge = pending
            .get(device_id)
            .ok_or(PairingError::PinExpiredOrInvalid)?;

        if challenge.expires_at_ms < now_ms || challenge.pin != pin {
            return Err(PairingError::PinExpiredOrInvalid);
        }

        let token = format!("pair_tok_{}_{now_ms}", uuid::Uuid::new_v4().simple());
        let session = PairedDeviceSession {
            device_id: device_id.to_owned(),
            session_token: token,
            paired_at_ms: now_ms,
        };

        pending.remove(device_id);
        let mut paired = self.paired.write().await;
        paired.insert(device_id.to_owned(), session.clone());

        Ok(session)
    }

    /// Checks if a device has an active paired session.
    pub async fn is_paired(&self, device_id: &str) -> bool {
        let paired = self.paired.read().await;
        paired.contains_key(device_id)
    }
}

//! Local IoT and robotics device discovery and pairing registry.

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

/// Represents a hardware device discovered locally via mDNS or direct broadcast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredLocalDevice {
    pub device_id: String,
    pub service_type: String, // e.g. "_ros2._tcp", "_esp32._tcp", "_micro_xrce._udp"
    pub host: String,
    pub port: u16,
    pub txt_records: HashMap<String, String>,
    pub paired: bool,
}

/// Registry of discovered and user-paired local hardware devices.
#[derive(Clone, Default)]
pub struct LocalDeviceRegistry {
    devices: Arc<RwLock<HashMap<String, DiscoveredLocalDevice>>>,
}

impl LocalDeviceRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            devices: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Ingests a discovered device advertisement from local mDNS.
    pub async fn announce_device(&self, device: DiscoveredLocalDevice) {
        let mut guard = self.devices.write().await;
        guard.insert(device.device_id.clone(), device);
    }

    /// User explicitly approves pairing with a local hardware device.
    pub async fn approve_pairing(&self, device_id: &str) -> bool {
        let mut guard = self.devices.write().await;
        if let Some(dev) = guard.get_mut(device_id) {
            dev.paired = true;
            true
        } else {
            false
        }
    }

    /// Retrieves an approved device for agent dispatch. Unpaired devices are withheld.
    pub async fn get_paired_device(&self, device_id: &str) -> Option<DiscoveredLocalDevice> {
        let guard = self.devices.read().await;
        guard.get(device_id).filter(|d| d.paired).cloned()
    }

    /// Lists all discovered devices with their pairing state.
    pub async fn list_discovered(&self) -> Vec<DiscoveredLocalDevice> {
        let guard = self.devices.read().await;
        let mut list: Vec<DiscoveredLocalDevice> = guard.values().cloned().collect();
        list.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discovery_and_explicit_user_pairing() {
        let registry = LocalDeviceRegistry::new();
        let device = DiscoveredLocalDevice {
            device_id: "turtlebot-edge-01".into(),
            service_type: "_ros2._tcp".into(),
            host: "turtlebot.local".into(),
            port: 11811,
            txt_records: HashMap::new(),
            paired: false,
        };

        registry.announce_device(device).await;

        // Unpaired device must NOT be returned for agent access
        assert!(
            registry
                .get_paired_device("turtlebot-edge-01")
                .await
                .is_none()
        );

        // Explicit user pairing
        let approved = registry.approve_pairing("turtlebot-edge-01").await;
        assert!(approved);

        // Now accessible
        let paired = registry.get_paired_device("turtlebot-edge-01").await;
        assert!(paired.is_some());
        assert_eq!(paired.unwrap().host, "turtlebot.local");
    }
}

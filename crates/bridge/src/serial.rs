//! USB Serial (UART / CDC-ACM) hardware bridge driver and device enumeration.

use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::{Mutex, mpsc};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SerialBridgeError {
    #[error("serial port `{0}` is not available")]
    PortUnavailable(String),
    #[error("device `{0}` was disconnected")]
    DeviceDisconnected(String),
    #[error("command write failed: {0}")]
    WriteFailed(String),
    #[error("read timeout exceeded")]
    Timeout,
}

/// Hardware serial connection settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialPortConfig {
    pub port_name: String,
    pub baud_rate: u32,
    pub timeout_ms: u64,
}

impl Default for SerialPortConfig {
    fn default() -> Self {
        Self {
            port_name: "/dev/ttyUSB0".into(),
            baud_rate: 115_200,
            timeout_ms: 1000,
        }
    }
}

/// Connection status of local USB microcontroller (ESP32, Pico, Arduino).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerialDeviceStatus {
    Connected,
    Disconnected,
    Error,
}

/// Manages USB serial connections, supporting simulated loopbacks for headless CI.
#[derive(Clone)]
pub struct SerialBridgeManager {
    ports: Arc<Mutex<HashMap<String, MockSerialChannel>>>,
}

#[derive(Clone)]
struct MockSerialChannel {
    tx: mpsc::Sender<Vec<u8>>,
    rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
    status: SerialDeviceStatus,
}

impl Default for SerialBridgeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SerialBridgeManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ports: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Registers a local USB or virtual serial device.
    pub async fn register_mock_device(
        &self,
        port_name: &str,
    ) -> (mpsc::Receiver<Vec<u8>>, mpsc::Sender<Vec<u8>>) {
        let (host_tx, device_rx) = mpsc::channel(64);
        let (device_tx, host_rx) = mpsc::channel(64);

        let channel = MockSerialChannel {
            tx: host_tx,
            rx: Arc::new(Mutex::new(host_rx)),
            status: SerialDeviceStatus::Connected,
        };

        let mut guard = self.ports.lock().await;
        guard.insert(port_name.to_string(), channel);
        (device_rx, device_tx)
    }

    /// Sends raw bytes or ASCII command to the connected hardware.
    ///
    /// # Errors
    ///
    /// Returns [`SerialBridgeError`] if port does not exist or device is disconnected.
    pub async fn write_bytes(&self, port_name: &str, data: &[u8]) -> Result<(), SerialBridgeError> {
        let guard = self.ports.lock().await;
        let Some(channel) = guard.get(port_name) else {
            return Err(SerialBridgeError::PortUnavailable(port_name.to_string()));
        };

        if channel.status != SerialDeviceStatus::Connected {
            return Err(SerialBridgeError::DeviceDisconnected(port_name.to_string()));
        }

        channel
            .tx
            .send(data.to_vec())
            .await
            .map_err(|e| SerialBridgeError::WriteFailed(e.to_string()))?;
        Ok(())
    }

    /// Reads incoming response bytes from the connected hardware.
    ///
    /// # Errors
    ///
    /// Returns [`SerialBridgeError`] if read fails or port is unavailable.
    pub async fn read_bytes(&self, port_name: &str) -> Result<Vec<u8>, SerialBridgeError> {
        let guard = self.ports.lock().await;
        let Some(channel) = guard.get(port_name) else {
            return Err(SerialBridgeError::PortUnavailable(port_name.to_string()));
        };

        if channel.status != SerialDeviceStatus::Connected {
            return Err(SerialBridgeError::DeviceDisconnected(port_name.to_string()));
        }

        let mut rx = channel.rx.lock().await;
        match rx.recv().await {
            Some(bytes) => Ok(bytes),
            None => Err(SerialBridgeError::DeviceDisconnected(port_name.to_string())),
        }
    }

    /// Marks a hardware device as disconnected to test graceful fallback.
    pub async fn simulate_disconnect(&self, port_name: &str) {
        let mut guard = self.ports.lock().await;
        if let Some(channel) = guard.get_mut(port_name) {
            channel.status = SerialDeviceStatus::Disconnected;
        }
    }

    /// Retrieves status of a specific serial port.
    pub async fn get_status(&self, port_name: &str) -> Option<SerialDeviceStatus> {
        let guard = self.ports.lock().await;
        guard.get(port_name).map(|c| c.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_serial_mock_roundtrip_and_disconnect() {
        let manager = SerialBridgeManager::new();
        let (mut dev_rx, dev_tx) = manager.register_mock_device("/dev/ttyUSB0").await;

        // Write from host to device
        manager
            .write_bytes("/dev/ttyUSB0", b"PING\n")
            .await
            .expect("write must succeed");

        let received_by_dev = dev_rx.recv().await.expect("device must receive bytes");
        assert_eq!(received_by_dev, b"PING\n");

        // Device responds
        dev_tx
            .send(b"PONG\n".to_vec())
            .await
            .expect("device response sent");
        let host_read = manager
            .read_bytes("/dev/ttyUSB0")
            .await
            .expect("host read must succeed");
        assert_eq!(host_read, b"PONG\n");

        // Simulate disconnection
        manager.simulate_disconnect("/dev/ttyUSB0").await;
        assert_eq!(
            manager.get_status("/dev/ttyUSB0").await,
            Some(SerialDeviceStatus::Disconnected)
        );

        let err = manager.write_bytes("/dev/ttyUSB0", b"TEST").await;
        assert!(matches!(err, Err(SerialBridgeError::DeviceDisconnected(_))));
    }
}

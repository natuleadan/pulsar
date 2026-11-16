//! Hardware safety watchdog: Dead-man switch halting runaway robots on heartbeat loss.

use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, watch},
    time::Instant,
};

use crate::ros2::{Ros2TopicBridge, TwistMessage};

pub const DEFAULT_DEADMAN_TIMEOUT_MS: u64 = 200;

/// Dead-man switch monitoring agent heartbeats. If agent turns silent for > 200ms,
/// an emergency stop zero-velocity frame is automatically published to prevent crashes.
pub struct DeadManWatchdog {
    timeout: Duration,
    last_heartbeat: Arc<Mutex<Instant>>,
    bridge: Ros2TopicBridge,
    stop_tx: watch::Sender<bool>,
}

impl DeadManWatchdog {
    #[must_use]
    pub fn new(bridge: Ros2TopicBridge, timeout_ms: u64) -> Self {
        let (stop_tx, _) = watch::channel(false);
        Self {
            timeout: Duration::from_millis(timeout_ms),
            last_heartbeat: Arc::new(Mutex::new(Instant::now())),
            bridge,
            stop_tx,
        }
    }

    /// Records agent activity heartbeat, resetting the dead-man countdown.
    pub async fn ping_heartbeat(&self) {
        let mut guard = self.last_heartbeat.lock().await;
        *guard = Instant::now();
    }

    /// Checks if the heartbeat has timed out, automatically commanding zero-velocity stop.
    pub async fn evaluate_watchdog(&self) -> bool {
        let guard = self.last_heartbeat.lock().await;
        let elapsed = guard.elapsed();
        drop(guard);

        if elapsed > self.timeout {
            // Heartbeat lost! Emergency stop activated immediately.
            tracing::warn!("dead-man emergency switch tripped after {:?}", elapsed);
            self.bridge.publish_cmd_vel(TwistMessage::zero()).await;
            let _ = self.stop_tx.send(true);
            true
        } else {
            false
        }
    }

    /// Returns a watch receiver to subscribe to dead-man emergency stop events.
    #[must_use]
    pub fn subscribe_emergency(&self) -> watch::Receiver<bool> {
        self.stop_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dead_man_switch_200ms_timeout() {
        let bridge = Ros2TopicBridge::new();
        // Set motion command
        bridge.publish_cmd_vel(TwistMessage::planar(1.5, 0.5)).await;
        assert_eq!(bridge.get_last_cmd_vel().await.unwrap().linear.x, 1.5);

        // 50ms watchdog for fast test
        let watchdog = DeadManWatchdog::new(bridge.clone(), 50);

        // Within deadline, no timeout
        assert!(!watchdog.evaluate_watchdog().await);

        // Wait past deadline without heartbeat ping
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Watchdog trips!
        let tripped = watchdog.evaluate_watchdog().await;
        assert!(tripped, "dead-man switch must trip after elapsed timeout");

        // Velocity must be forced to zero!
        let stopped = bridge.get_last_cmd_vel().await.unwrap();
        assert_eq!(stopped.linear.x, 0.0);
        assert_eq!(stopped.angular.z, 0.0);
    }
}

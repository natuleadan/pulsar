//! ROS2 DDS and Micro-XRCE-DDS message adapters and serialization.

use pulsar_protocol::RobotCommandFrame;
use serde::{Deserialize, Serialize};

/// 3D vector for linear and angular robotics coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Vector3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// ROS2 `geometry_msgs/msg/Twist` representation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct TwistMessage {
    pub linear: Vector3,
    pub angular: Vector3,
}

impl TwistMessage {
    #[must_use]
    pub fn zero() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn planar(linear_x: f64, angular_z: f64) -> Self {
        Self {
            linear: Vector3 {
                x: linear_x,
                y: 0.0,
                z: 0.0,
            },
            angular: Vector3 {
                x: 0.0,
                y: 0.0,
                z: angular_z,
            },
        }
    }
}

/// Translates a generic [`RobotCommandFrame`] action into a standard ROS2 Twist message.
#[must_use]
pub fn command_frame_to_twist(frame: &RobotCommandFrame) -> Option<TwistMessage> {
    if frame.action == "cmd_vel" || frame.action == "move" {
        let linear_x = frame
            .parameters
            .get("linear")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        let angular_z = frame
            .parameters
            .get("angular")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        Some(TwistMessage::planar(linear_x, angular_z))
    } else {
        None
    }
}

/// Simulated ROS2 topic channel bridge connecting agent commands to robot nodes.
#[derive(Clone, Default)]
pub struct Ros2TopicBridge {
    last_published: std::sync::Arc<tokio::sync::Mutex<Option<TwistMessage>>>,
}

impl Ros2TopicBridge {
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_published: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    pub async fn publish_cmd_vel(&self, twist: TwistMessage) {
        let mut guard = self.last_published.lock().await;
        *guard = Some(twist);
    }

    pub async fn get_last_cmd_vel(&self) -> Option<TwistMessage> {
        let guard = self.last_published.lock().await;
        *guard
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_twist_serialization_and_bridge() {
        let bridge = Ros2TopicBridge::new();
        let twist = TwistMessage::planar(0.5, 0.2);

        bridge.publish_cmd_vel(twist).await;
        let last = bridge.get_last_cmd_vel().await.unwrap();

        assert_eq!(last.linear.x, 0.5);
        assert_eq!(last.angular.z, 0.2);

        // Zero velocity
        bridge.publish_cmd_vel(TwistMessage::zero()).await;
        let zero = bridge.get_last_cmd_vel().await.unwrap();
        assert_eq!(zero.linear.x, 0.0);
        assert_eq!(zero.angular.z, 0.0);
    }
}

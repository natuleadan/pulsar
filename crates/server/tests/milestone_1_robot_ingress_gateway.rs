//! Integration tests for Milestone 1: Sovereign Ingress Gateway for External Robots & IoT Hardware.

use std::sync::Arc;

use agents_protocol::{DeviceHandshake, RobotCommandFrame, RobotTelemetryFrame};
use agents_server::{infra::stream::BusClient, modules::robots::services::RobotGatewayService};

#[tokio::test]
async fn test_robot_handshake_registration_and_token_issuance() {
    let bus = Arc::new(BusClient::default());
    let service = RobotGatewayService::new(bus);

    let handshake = DeviceHandshake {
        device_id: "robot-quad-01".into(),
        device_type: "quadruped".into(),
        public_key_ed25519: "ed25519_pub_key_abcdef1234567890".into(),
        firmware_version: "2.1.0".into(),
        signature: "sig_mock_signature".into(),
    };

    let resp = service
        .register_device(handshake)
        .await
        .expect("registration should succeed");

    assert_eq!(resp.device_id, "robot-quad-01");
    assert!(resp.session_token.starts_with("rbt_tok_"));
    assert_eq!(resp.lease_ttl_secs, 3600);
    assert_eq!(resp.ingress_ws_endpoint, "/api/v1/robots/robot-quad-01/ws");

    // Invalid handshake rejection
    let invalid_handshake = DeviceHandshake {
        device_id: "   ".into(),
        device_type: "quadruped".into(),
        public_key_ed25519: "".into(),
        firmware_version: "1.0.0".into(),
        signature: "".into(),
    };
    assert!(service.register_device(invalid_handshake).await.is_err());
}

#[tokio::test]
async fn test_robot_telemetry_ingest_and_circuit_breaker() {
    let bus = Arc::new(BusClient::default());
    let service = RobotGatewayService::new(bus);

    let handshake = DeviceHandshake {
        device_id: "robot-arm-02".into(),
        device_type: "robotic_arm".into(),
        public_key_ed25519: "ed25519_pub_arm_02".into(),
        firmware_version: "1.0.4".into(),
        signature: "sig_arm_valid".into(),
    };

    let reg = service.register_device(handshake).await.unwrap();

    // Normal telemetry
    let normal_frame = RobotTelemetryFrame {
        device_id: "robot-arm-02".into(),
        timestamp_ms: 1000,
        sensor_data: serde_json::json!({ "joint_angles": [0.0, 45.0, 90.0] }),
        emergency_stop_status: false,
        battery_percentage: Some(95.0),
    };

    service
        .ingest_telemetry("robot-arm-02", &reg.session_token, normal_frame)
        .await
        .expect("telemetry ingest should succeed");

    let latest = service.get_telemetry("robot-arm-02").await.unwrap();
    assert!(!latest.emergency_stop_status);

    // Enqueue command
    let cmd = RobotCommandFrame {
        command_id: "cmd_01".into(),
        device_id: "robot-arm-02".into(),
        action: "move_joint".into(),
        parameters: serde_json::json!({ "joint_idx": 1, "angle": 60.0 }),
        emergency_stop_override: false,
        issued_at_ms: 1100,
    };
    service.enqueue_command(cmd).await.unwrap();

    // Trip emergency stop via telemetry
    let estop_frame = RobotTelemetryFrame {
        device_id: "robot-arm-02".into(),
        timestamp_ms: 1200,
        sensor_data: serde_json::json!({ "collision_sensor": true }),
        emergency_stop_status: true,
        battery_percentage: Some(94.5),
    };

    service
        .ingest_telemetry("robot-arm-02", &reg.session_token, estop_frame)
        .await
        .expect("estop telemetry should be ingested");

    let latest_estop = service.get_telemetry("robot-arm-02").await.unwrap();
    assert!(latest_estop.emergency_stop_status);

    // Pending commands must be cleared by E-STOP
    let pending = service
        .poll_commands("robot-arm-02", &reg.session_token)
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "E-STOP must clear all pending motion commands"
    );

    // Enqueueing new motion command while E-STOP is active must be rejected
    let new_cmd = RobotCommandFrame {
        command_id: "cmd_02".into(),
        device_id: "robot-arm-02".into(),
        action: "rotate_gripper".into(),
        parameters: serde_json::json!({ "angle": 15.0 }),
        emergency_stop_override: false,
        issued_at_ms: 1300,
    };
    assert!(service.enqueue_command(new_cmd).await.is_err());
}

#[tokio::test]
async fn test_emergency_stop_manual_trip_and_resume() {
    let bus = Arc::new(BusClient::default());
    let service = RobotGatewayService::new(bus);

    let handshake = DeviceHandshake {
        device_id: "rover-mobile-03".into(),
        device_type: "mobile_base".into(),
        public_key_ed25519: "ed25519_pub_rover_03".into(),
        firmware_version: "3.0.1".into(),
        signature: "sig_rover_valid".into(),
    };

    let reg = service.register_device(handshake).await.unwrap();

    // Manual trip
    let alert = service
        .trip_emergency_stop("rover-mobile-03", "Operator pushed software E-STOP")
        .await
        .unwrap();
    assert_eq!(alert.device_id, "rover-mobile-03");
    assert!(alert.active_emergency_stop);

    // List devices reflects emergency stop
    let devices = service.list_devices().await;
    let rover = devices
        .iter()
        .find(|d| d.device_id == "rover-mobile-03")
        .unwrap();
    assert!(rover.emergency_stop_active);

    // Reset emergency stop
    service
        .reset_emergency_stop("rover-mobile-03")
        .await
        .unwrap();

    let devices_after = service.list_devices().await;
    let rover_after = devices_after
        .iter()
        .find(|d| d.device_id == "rover-mobile-03")
        .unwrap();
    assert!(!rover_after.emergency_stop_active);

    // Now motion commands succeed again
    let cmd = RobotCommandFrame {
        command_id: "cmd_03".into(),
        device_id: "rover-mobile-03".into(),
        action: "navigate_waypoint".into(),
        parameters: serde_json::json!({ "x": 10.0, "y": 20.0 }),
        emergency_stop_override: false,
        issued_at_ms: 1400,
    };
    assert!(service.enqueue_command(cmd).await.is_ok());

    let polled = service
        .poll_commands("rover-mobile-03", &reg.session_token)
        .await
        .unwrap();
    assert_eq!(polled.len(), 1);
    assert_eq!(polled[0].action, "navigate_waypoint");
}

#[tokio::test]
async fn test_pairing_gateway_pin_flow_and_session_issuance() {
    let bus = Arc::new(BusClient::default());
    let service = RobotGatewayService::new(bus);

    let pin = service.generate_pairing_pin("edge-quad-99").await;
    assert_eq!(pin.len(), 6);

    // Wrong PIN
    let err = service.verify_pairing_pin("edge-quad-99", "123456").await;
    assert!(err.is_err());

    // Correct PIN
    let session = service
        .verify_pairing_pin("edge-quad-99", &pin)
        .await
        .expect("correct PIN must pair");
    assert_eq!(session.device_id, "edge-quad-99");
    assert!(session.session_token.starts_with("pair_tok_"));
}

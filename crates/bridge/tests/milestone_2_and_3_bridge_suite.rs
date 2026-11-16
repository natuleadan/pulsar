//! Integration tests for Batch 7 Hitos 7.2 & 7.3: Local Device Bridge, ROS2, and LAN Shield.

use std::{
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use agents_bridge::{
    DeadManWatchdog, LocalDeviceRegistry, PairingGateway, Ros2TopicBridge, SerialBridgeError,
    SerialBridgeManager, SerialDeviceStatus, TwistMessage, is_private_lan_ip, validate_outbound_ip,
    validate_twist_limits,
};

#[tokio::test]
async fn test_zero_trust_private_lan_shield_blocks_private_subnets() {
    // 192.168.0.0/16 must be blocked
    let home_router = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
    assert!(is_private_lan_ip(&home_router));
    assert!(validate_outbound_ip(&home_router).is_err());

    // 10.0.0.0/8 must be blocked
    let corp_db = IpAddr::V4(Ipv4Addr::new(10, 10, 0, 5));
    assert!(is_private_lan_ip(&corp_db));
    assert!(validate_outbound_ip(&corp_db).is_err());

    // 172.16.0.0/12 must be blocked
    let vpn_host = IpAddr::V4(Ipv4Addr::new(172, 16, 50, 1));
    assert!(is_private_lan_ip(&vpn_host));
    assert!(validate_outbound_ip(&vpn_host).is_err());

    // Public IPs are permitted
    let cloud_target = IpAddr::V4(Ipv4Addr::new(157, 254, 174, 217));
    assert!(!is_private_lan_ip(&cloud_target));
    assert!(validate_outbound_ip(&cloud_target).is_ok());
}

#[tokio::test]
async fn test_usb_serial_microcontroller_roundtrip_and_disconnect() {
    let bridge = SerialBridgeManager::new();
    let (mut esp32_rx, esp32_tx) = bridge.register_mock_device("/dev/ttyACM0").await;

    // Send motion command over serial
    bridge
        .write_bytes("/dev/ttyACM0", b"SET_VEL:100,50\n")
        .await
        .expect("serial write must succeed");

    let received = esp32_rx
        .recv()
        .await
        .expect("microcontroller received command");
    assert_eq!(received, b"SET_VEL:100,50\n");

    // Microcontroller emits sensor telemetry
    esp32_tx
        .send(b"IMU_PITCH:12.4\n".to_vec())
        .await
        .expect("microcontroller emitted telemetry");

    let telemetry = bridge
        .read_bytes("/dev/ttyACM0")
        .await
        .expect("bridge read telemetry");
    assert_eq!(telemetry, b"IMU_PITCH:12.4\n");

    // Hardware disconnect simulation
    bridge.simulate_disconnect("/dev/ttyACM0").await;
    assert_eq!(
        bridge.get_status("/dev/ttyACM0").await,
        Some(SerialDeviceStatus::Disconnected)
    );

    let err = bridge.write_bytes("/dev/ttyACM0", b"STOP\n").await;
    assert!(matches!(err, Err(SerialBridgeError::DeviceDisconnected(_))));
}

#[tokio::test]
async fn test_ros2_dead_man_switch_and_zero_velocity() {
    let ros2 = Ros2TopicBridge::new();
    ros2.publish_cmd_vel(TwistMessage::planar(2.0, 0.8)).await;

    let watchdog = DeadManWatchdog::new(ros2.clone(), 50);

    // Initial state: not tripped
    assert!(!watchdog.evaluate_watchdog().await);

    // Keep alive with ping
    watchdog.ping_heartbeat().await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!watchdog.evaluate_watchdog().await);

    // Timeout: agent fails to ping for 60ms
    tokio::time::sleep(Duration::from_millis(60)).await;
    let tripped = watchdog.evaluate_watchdog().await;
    assert!(tripped, "dead-man switch must trigger on heartbeat loss");

    // Must have published zero-velocity command
    let current_vel = ros2.get_last_cmd_vel().await.unwrap();
    assert_eq!(current_vel.linear.x, 0.0);
    assert_eq!(current_vel.angular.z, 0.0);
}

#[tokio::test]
async fn test_pairing_gateway_and_local_discovery() {
    let registry = LocalDeviceRegistry::new();
    let pairing = PairingGateway::new();

    let pin = pairing.generate_pin("robot-edge-arm", 300, 50_000).await;
    assert_eq!(pin.len(), 6);

    // Correct verification
    let session = pairing
        .verify_pin_and_pair("robot-edge-arm", &pin, 50_100)
        .await
        .expect("valid PIN must pair");
    assert_eq!(session.device_id, "robot-edge-arm");

    // Approve local registry device
    assert!(!registry.approve_pairing("unknown-device").await);
}

#[test]
fn test_velocity_limit_checks() {
    assert!(validate_twist_limits(1.0, 0.5, 1.5, 1.0).is_ok());
    assert!(validate_twist_limits(2.0, 0.5, 1.5, 1.0).is_err());
}

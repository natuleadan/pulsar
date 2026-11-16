//! Zero-trust private LAN shield and robotics safety boundary validator.

use std::net::IpAddr;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BridgeSecurityError {
    #[error("access to private LAN subnet {0} is forbidden under zero-trust policy")]
    PrivateSubnetForbidden(String),
    #[error("linear velocity {0} m/s exceeds safe maximum limit {1} m/s")]
    LinearVelocityExceeded(String, String),
    #[error("angular velocity {0} rad/s exceeds safe maximum limit {1} rad/s")]
    AngularVelocityExceeded(String, String),
    #[error("geofence boundary breach at coordinate ({0}, {1})")]
    GeofenceBreach(String, String),
}

/// Checks if an IP address belongs to RFC 1918 private IPv4 subnets
/// (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16) or link-local (169.254.0.0/16).
#[must_use]
pub fn is_private_lan_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // 10.0.0.0/8
            if octets[0] == 10 {
                return true;
            }
            // 172.16.0.0/12 (172.16.0.0 - 172.31.255.255)
            if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                return true;
            }
            // 192.168.0.0/16
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }
            // 169.254.0.0/16 (Link-local)
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
            false
        }
        IpAddr::V6(ipv6) => {
            // IPv6 Unique Local Addresses (fc00::/7) or Link-local (fe80::/10)
            let segments = ipv6.segments();
            let first_byte = (segments[0] >> 8) as u8;
            (first_byte & 0xfe) == 0xfc || (segments[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Enforces the Zero-Trust Private LAN Shield, forbidding sandboxed execution
/// from initiating raw TCP or UDP connections to internal home or corporate subnets.
///
/// # Errors
///
/// Returns [`BridgeSecurityError::PrivateSubnetForbidden`] if target IP is in private address space.
pub fn validate_outbound_ip(ip: &IpAddr) -> Result<(), BridgeSecurityError> {
    if is_private_lan_ip(ip) {
        return Err(BridgeSecurityError::PrivateSubnetForbidden(ip.to_string()));
    }
    Ok(())
}

/// Validates velocity commands to prevent destructive or runaway robot kinetics.
///
/// # Errors
///
/// Returns [`BridgeSecurityError`] if linear or angular components exceed physical constraints.
pub fn validate_twist_limits(
    linear: f64,
    angular: f64,
    max_linear: f64,
    max_angular: f64,
) -> Result<(), BridgeSecurityError> {
    if linear.abs() > max_linear {
        return Err(BridgeSecurityError::LinearVelocityExceeded(
            format!("{linear:.2}"),
            format!("{max_linear:.2}"),
        ));
    }
    if angular.abs() > max_angular {
        return Err(BridgeSecurityError::AngularVelocityExceeded(
            format!("{angular:.2}"),
            format!("{max_angular:.2}"),
        ));
    }
    Ok(())
}

/// Validates 2D planar workspace geofence boundaries (e.g. bounding box limits).
///
/// # Errors
///
/// Returns [`BridgeSecurityError::GeofenceBreach`] if coordinate falls outside defined boundaries.
pub fn validate_geofence(
    x: f64,
    y: f64,
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
) -> Result<(), BridgeSecurityError> {
    if x < min_x || x > max_x || y < min_y || y > max_y {
        return Err(BridgeSecurityError::GeofenceBreach(
            format!("{x:.2}"),
            format!("{y:.2}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_rfc1918_private_subnets_detected() {
        assert!(is_private_lan_ip(&IpAddr::V4(Ipv4Addr::new(
            192, 168, 1, 50
        ))));
        assert!(is_private_lan_ip(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_private_lan_ip(&IpAddr::V4(Ipv4Addr::new(172, 20, 0, 1))));
        assert!(is_private_lan_ip(&IpAddr::V4(Ipv4Addr::new(
            169, 254, 1, 1
        ))));

        // Public IPs must not be classified as private LAN
        assert!(!is_private_lan_ip(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_private_lan_ip(&IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_private_lan_ip(&IpAddr::V4(Ipv4Addr::new(
            157, 254, 174, 217
        ))));
    }

    #[test]
    fn test_velocity_limit_guardrails() {
        assert!(validate_twist_limits(1.2, 0.5, 2.0, 1.0).is_ok());
        assert!(validate_twist_limits(2.5, 0.5, 2.0, 1.0).is_err());
        assert!(validate_twist_limits(1.0, 1.5, 2.0, 1.0).is_err());
    }

    #[test]
    fn test_geofence_containment() {
        assert!(validate_geofence(5.0, 5.0, 0.0, 10.0, 0.0, 10.0).is_ok());
        assert!(validate_geofence(-1.0, 5.0, 0.0, 10.0, 0.0, 10.0).is_err());
        assert!(validate_geofence(5.0, 12.0, 0.0, 10.0, 0.0, 10.0).is_err());
    }
}

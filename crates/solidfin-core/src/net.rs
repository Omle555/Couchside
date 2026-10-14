//! Network address helpers.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// True if `ip` is this PC or a device on a home/local network.
///
/// Accepts loopback, private IPv4 (`10/8`, `172.16/12`, `192.168/16`), IPv4 link-local
/// (`169.254/16`), IPv6 unique-local (`fc00::/7`) and IPv6 link-local (`fe80::/10`).
/// IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`) are checked as IPv4.
pub fn is_local_network(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_local_v4(v4),
            None => is_local_v6(v6),
        },
    }
}

fn is_local_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

fn is_local_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(addr: &str) -> bool {
        is_local_network(addr.parse().expect("valid test address"))
    }

    #[test]
    fn accepts_loopback_and_private_ipv4() {
        for addr in [
            "127.0.0.1",
            "127.255.255.254",
            "10.0.0.1",
            "10.255.255.255",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.1",
            "192.168.1.50",
            "169.254.10.20",
        ] {
            assert!(local(addr), "{addr} should be local");
        }
    }

    #[test]
    fn rejects_public_and_edge_ipv4() {
        for addr in [
            "8.8.8.8",
            "1.1.1.1",
            "172.15.255.255",
            "172.32.0.0",
            "192.167.255.255",
            "192.169.0.0",
            "11.0.0.1",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.251",
        ] {
            assert!(!local(addr), "{addr} should not be local");
        }
    }

    #[test]
    fn accepts_local_ipv6() {
        for addr in [
            "::1",
            "fc00::1",
            "fd12:3456:789a::1",
            "fe80::1",
            "febf:ffff::1",
            "::ffff:192.168.1.20",
            "::ffff:127.0.0.1",
        ] {
            assert!(local(addr), "{addr} should be local");
        }
    }

    #[test]
    fn rejects_public_ipv6() {
        for addr in [
            "2001:4860:4860::8888",
            "2a00:1450::1",
            "fec0::1",
            "fbff::1",
            "::",
            "ff02::fb",
            "::ffff:8.8.8.8",
        ] {
            assert!(!local(addr), "{addr} should not be local");
        }
    }
}

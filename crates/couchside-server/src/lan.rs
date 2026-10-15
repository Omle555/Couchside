//! Home-network access: the LAN-only guard and the addresses other devices can use.

use crate::api::ApiError;
use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use couchside_core::net::is_local_network;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Address used only to ask the OS which interface has the default route; nothing is sent to it.
const ROUTE_PROBE: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 9);

/// Rejects requests from addresses outside the local network with a 403.
pub async fn lan_only(
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    if is_local_network(client.ip()) {
        next.run(request).await
    } else {
        tracing::warn!(client = %client.ip(), "rejected request from outside the local network");
        ApiError::OutsideLocalNetwork.into_response()
    }
}

/// URLs other devices on the network can open, for a server listening on `bind`.
pub async fn network_urls(bind: IpAddr, port: u16) -> Vec<String> {
    if bind.is_loopback() {
        return Vec::new();
    }
    let mut hosts = Vec::new();
    if bind.is_unspecified() {
        match primary_address().await {
            Ok(ip) => hosts.push(url_host(ip)),
            Err(err) => tracing::debug!(error = %err, "could not find this PC's network address"),
        }
        if let Some(name) = computer_name() {
            hosts.push(name);
        }
    } else {
        hosts.push(url_host(bind));
    }
    hosts
        .into_iter()
        .map(|host| format!("http://{host}:{port}"))
        .collect()
}

/// This PC's name, which other Windows PCs on the network can usually resolve.
pub fn computer_name() -> Option<String> {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|var| std::env::var(var).ok())
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
}

/// The address of the interface the OS would use for outgoing traffic (no packets are sent).
pub async fn primary_address() -> std::io::Result<IpAddr> {
    let socket = tokio::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    socket.connect(ROUTE_PROBE).await?;
    Ok(socket.local_addr()?.ip())
}

/// `ip` formatted for the host part of a URL (IPv6 in brackets).
pub fn url_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loopback_bind_has_no_network_urls() {
        assert!(network_urls(Ipv4Addr::LOCALHOST.into(), 8096)
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn specific_bind_uses_that_address() {
        let ip: IpAddr = "192.168.1.20".parse().expect("valid");
        assert_eq!(
            network_urls(ip, 8096).await,
            vec!["http://192.168.1.20:8096".to_string()]
        );
        let ip: IpAddr = "fe80::1".parse().expect("valid");
        assert_eq!(
            network_urls(ip, 80).await,
            vec!["http://[fe80::1]:80".to_string()]
        );
    }
}

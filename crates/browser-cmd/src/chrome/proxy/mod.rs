//! The way to an upstream proxy: a loopback HTTP/CONNECT proxy Chrome's
//! requests go through, which is what lets Chrome use an authenticated HTTP or
//! SOCKS5 upstream — Chrome itself cannot — with the address resolved here
//! and pinned, and no silent direct-route fallback if the upstream fails.
//! Chrome is told to use it with `--proxy-server` or a PAC script, and nothing
//! in the page can see it.

mod forwarder;
mod relay;
mod upstream;

use std::{net::SocketAddr, time::Duration};

use crate::Result;
use crate::settings::upstream::UpstreamProxy;
use forwarder::proxy_connection;
use tokio::{net::TcpListener, task::JoinHandle};

/// Bytes the tunnels have carried both ways, for an operator paying the upstream by traffic.
pub(crate) static CARRIED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Debug)]
pub(crate) struct ProxyLimits {
    pub(crate) connections: usize,
    pub(crate) header_bytes: usize,
    pub(crate) request: Duration,
    pub(crate) connect: Duration,
    pub(crate) tunnel: Duration,
}

impl Default for ProxyLimits {
    fn default() -> Self {
        Self {
            connections: 256,
            header_bytes: 32 * 1024,
            request: Duration::from_secs(10),
            connect: Duration::from_secs(15),
            tunnel: Duration::from_secs(120),
        }
    }
}

/// A proxy script sending only `hosts` and their subdomains to the local
/// forwarder, with no direct fallback for them; everything else goes direct.
pub(crate) fn pac(hosts: &[String], address: SocketAddr) -> String {
    let matches: Vec<String> =
        hosts.iter().map(|host| format!("h=={host:?}||dnsDomainIs(h,\".{host}\")")).collect();
    let script = format!(
        "function FindProxyForURL(u,h){{return {}?\"PROXY {address}\":\"DIRECT\"}}",
        matches.join("||")
    );
    let encoded = percent_encoding::utf8_percent_encode(&script, percent_encoding::NON_ALPHANUMERIC);
    format!("data:application/x-ns-proxy-autoconfig,{encoded}")
}

pub(crate) async fn start(
    upstream: UpstreamProxy,
    limits: ProxyLimits,
) -> Result<(SocketAddr, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { break };
                    while connections.try_join_next().is_some() {}
                    if connections.len() >= limits.connections { drop(stream); continue; }
                    let (upstream, limits) = (upstream.clone(), limits.clone());
                    connections.spawn(async move { let _ = proxy_connection(stream, upstream, limits).await; });
                }
                _ = connections.join_next(), if !connections.is_empty() => {},
            }
        }
        // Dropping JoinSet aborts every owned connection, not only the listener.
    });
    Ok((address, task))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use upstream::read_proxy_header;

    pub(super) fn limits() -> ProxyLimits {
        ProxyLimits {
            connections: 4,
            header_bytes: 1024,
            request: Duration::from_millis(80),
            connect: Duration::from_millis(80),
            tunnel: Duration::from_secs(5),
        }
    }

    #[tokio::test]
    async fn stopping_the_proxy_closes_active_tunnels() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = UpstreamProxy::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let header = read_proxy_header(&mut stream, 1024).await.unwrap();
            assert!(header.starts_with("CONNECT 127.0.0.1:80 "), "{header}");
            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            stream.read(&mut [0; 1]).await.unwrap()
        });
        let (address, task) = start(upstream, limits()).await.unwrap();
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"CONNECT 127.0.0.1:80 HTTP/1.1\r\nHost: 127.0.0.1:80\r\n\r\n").await.unwrap();
        assert!(read_proxy_header(&mut client, 1024).await.unwrap().contains("200"));
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(tokio::time::timeout(Duration::from_secs(1), peer).await.unwrap().unwrap(), 0);
        assert_eq!(client.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

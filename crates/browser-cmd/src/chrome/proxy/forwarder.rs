//! One connection from Chrome: its request read, its destination resolved and
//! reached through the upstream, and its bytes handed to the relay.

use crate::chrome::proxy::ProxyLimits;
use crate::chrome::proxy::relay::relay;
use crate::chrome::proxy::upstream::connect_via_proxy;
use crate::settings::upstream::{UpstreamProxy, proxy_host};
use crate::{Error, Result};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub(super) async fn proxy_connection(
    mut client: TcpStream,
    upstream_proxy: UpstreamProxy,
    limits: ProxyLimits,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + limits.request;
    let mut request = Vec::with_capacity(4096);
    let header_end = loop {
        if request.len() >= limits.header_bytes {
            return proxy_refusal(&mut client, "431 Request Header Fields Too Large").await;
        }
        let mut chunk = [0_u8; 4096];
        let room = chunk.len().min(limits.header_bytes - request.len());
        let read = tokio::time::timeout_at(deadline, client.read(&mut chunk[..room]))
            .await
            .map_err(|_| Error::Tool("browser proxy request timed out".into()))??;
        if read == 0 {
            return Ok(());
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let header = std::str::from_utf8(&request[..header_end])
        .map_err(|_| Error::Tool("browser proxy received a non-HTTP request".into()))?;
    let first = header.lines().next().unwrap_or_default();
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or("HTTP/1.1");

    let tunnel = method.eq_ignore_ascii_case("CONNECT");
    let (host, port, response, forwarded_header) = if tunnel {
        let (host, port) = proxy_authority(target)?;
        (host, port, Some(&b"HTTP/1.1 200 Connection Established\r\n\r\n"[..]), Vec::new())
    } else {
        let url = url::Url::parse(target)
            .map_err(|_| Error::Tool("browser proxy accepts absolute HTTP request targets".into()))?;
        if url.scheme() != "http" {
            return proxy_refusal(&mut client, "400 Bad Request").await;
        }
        let host = proxy_host(&url)?;
        let port = url.port_or_known_default().unwrap_or(80);
        let path = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        let header = format!("{method} {path} {version}\r\n{}", origin_headers(header)).into_bytes();
        (host, port, None, header)
    };
    let Some(mut upstream) = connect_target(&host, port, &upstream_proxy, &limits).await? else {
        return proxy_refusal(&mut client, "502 Bad Gateway").await;
    };
    tokio::time::timeout(limits.request, async {
        if let Some(response) = response {
            client.write_all(response).await?;
        }
        if !forwarded_header.is_empty() {
            upstream.write_all(&forwarded_header).await?;
        }
        if request.len() > header_end {
            upstream.write_all(&request[header_end..]).await?;
        }
        Ok::<(), Error>(())
    })
    .await
    .map_err(|_| Error::Timeout("browser proxy handoff"))??;
    relay(&mut client, &mut upstream, limits.tunnel).await
}

async fn proxy_refusal(client: &mut TcpStream, status: &str) -> Result<()> {
    client
        .write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes())
        .await?;
    Ok(())
}

fn proxy_authority(target: &str) -> Result<(String, u16)> {
    let url = url::Url::parse(&format!("https://{target}"))
        .map_err(|_| Error::Tool("browser proxy received an invalid CONNECT authority".into()))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Tool("browser proxy received an invalid CONNECT authority".into()));
    }
    let port = url.port_or_known_default().unwrap_or(443);
    if port == 0 {
        return Err(Error::Tool("CONNECT destination port must be nonzero".into()));
    }
    Ok((proxy_host(&url)?, port))
}

/// Resolve here and hand the upstream an address, never a name: what the
/// page asked for is what is reached, and the upstream cannot answer for
/// something else.
async fn connect_target(
    host: &str,
    port: u16,
    upstream: &UpstreamProxy,
    limits: &ProxyLimits,
) -> Result<Option<TcpStream>> {
    let deadline = tokio::time::Instant::now() + limits.connect;
    tokio::time::timeout_at(deadline, async {
        for address in tokio::net::lookup_host((host, port)).await? {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout("proxy destination handshake"));
            }
            if let Ok(stream) = connect_via_proxy(upstream, address, remaining, limits.header_bytes).await {
                return Ok(Some(stream));
            }
        }
        Ok(None)
    })
    .await
    .map_err(|_| Error::Timeout("proxy destination handshake"))?
}

/// The headers an origin is given. Proxy credentials and connection
/// management stay here, and the connection closes after one exchange: the
/// relay copies bytes, so a second request Chrome sent on a kept-alive
/// connection would reach the first request's host unrewritten.
fn origin_headers(header: &str) -> String {
    const HOP: [&str; 4] = ["proxy-authorization", "proxy-connection", "connection", "keep-alive"];
    let mut out = String::new();
    for line in header.split("\r\n").skip(1).filter(|line| !line.is_empty()) {
        let name = line.split_once(':').map(|(name, _)| name.trim()).unwrap_or_default();
        if !HOP.iter().any(|hop| name.eq_ignore_ascii_case(hop)) {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    out.push_str("Connection: close\r\n\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::proxy::tests::limits;
    use crate::chrome::proxy::upstream::read_proxy_header;
    use std::time::Duration;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn absolute_http_requests_are_rewritten_once_before_the_tunnel_is_relayed() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_address = upstream_listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            let (mut stream, _) = upstream_listener.accept().await.unwrap();
            let connect = read_proxy_header(&mut stream, 1024).await.unwrap();
            assert!(connect.starts_with("CONNECT 127.0.0.1:8080 HTTP/1.1"), "{connect}");
            stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.unwrap();
            let request = read_proxy_header(&mut stream, 1024).await.unwrap();
            assert!(request.starts_with("GET /path?q=1 HTTP/1.1\r\n"), "{request}");
            assert!(request.contains("Authorization: origin-secret\r\n"), "{request}");
            assert!(!request.to_ascii_lowercase().contains("proxy-authorization"), "{request}");
            assert!(request.ends_with("Connection: close\r\n\r\n"), "{request}");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            stream.shutdown().await.unwrap();
        });

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let proxy = UpstreamProxy::parse(&format!("http://{upstream_address}")).unwrap();
        let task = tokio::spawn(proxy_connection(server, proxy, ProxyLimits::default()));
        let mut client = client;
        client
            .write_all(
                b"GET http://127.0.0.1:8080/path?q=1 HTTP/1.1\r\nHost: 127.0.0.1:8080\r\nAuthorization: origin-secret\r\nProxy-Authorization: upstream-secret\r\n\r\n",
            )
            .await
            .unwrap();
        client.shutdown().await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.ends_with(b"ok"), "{}", String::from_utf8_lossy(&response));
        task.await.unwrap().unwrap();
        upstream.await.unwrap();
    }

    #[test]
    fn explicit_ports_ipv6_and_proxy_credentials_are_preserved_or_filtered_correctly() {
        assert_eq!(UpstreamProxy::parse("http://proxy.example:80").unwrap().port, 80);
        assert_eq!(UpstreamProxy::parse("http://proxy.example").unwrap().port, 80);
        assert_eq!(UpstreamProxy::parse("socks5://[::1]:1080").unwrap().host, "::1");
        assert_eq!(proxy_authority("example.org:80").unwrap(), ("example.org".into(), 80));
        assert_eq!(proxy_authority("[::1]:443").unwrap(), ("::1".into(), 443));
        assert!(proxy_authority("user:pass@example.org:443").is_err());
        assert!(proxy_authority("example.org:443/path").is_err());
        let headers = origin_headers(
            "GET http://example.org/ HTTP/1.1\r\nHost: example.org\r\nProxy-Authorization: secret\r\nProxy-Connection: keep-alive\r\nAuthorization: origin-credential\r\n\r\n",
        );
        assert!(!headers.to_ascii_lowercase().contains("proxy-"));
        assert!(headers.contains("Authorization: origin-credential"));
        assert!(
            headers.ends_with("Connection: close\r\n\r\n") && !headers.contains("keep-alive"),
            "{headers}"
        );
        assert!(UpstreamProxy::parse("socks5h://proxy.example").is_err());
    }

    #[tokio::test]
    async fn incoming_progress_does_not_reset_the_header_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let proxy = UpstreamProxy::parse("http://127.0.0.1:1").unwrap();
        let task = tokio::spawn(proxy_connection(server, proxy, limits()));
        let writer = tokio::spawn(async move {
            for _ in 0..100 {
                if client.write_all(b"x").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let result = tokio::time::timeout(Duration::from_millis(500), task).await.unwrap().unwrap();
        assert!(result.is_err());
        writer.abort();
    }

    #[tokio::test]
    async fn destination_resolution_and_address_attempts_share_one_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            // Under load the client's budget may expire during resolution,
            // before it reaches this listener. Do not wait forever for a
            // connection the timeout correctly prevented.
            let Ok(accepted) = tokio::time::timeout(Duration::from_secs(1), listener.accept()).await else {
                return;
            };
            let (mut stream, _) = accepted.unwrap();
            let _ = tokio::time::timeout(Duration::from_secs(1), read_proxy_header(&mut stream, 1024)).await;
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), stream.read(&mut [0; 1]))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
        });
        let upstream = UpstreamProxy::parse(&format!("http://{address}")).unwrap();
        let limits = ProxyLimits {
            connections: 1,
            header_bytes: 1024,
            request: Duration::from_secs(1),
            connect: Duration::from_millis(80),
            tunnel: Duration::from_secs(1),
        };
        let result = tokio::time::timeout(
            Duration::from_millis(300),
            connect_target("localhost", 443, &upstream, &limits),
        )
        .await
        .unwrap();
        assert!(!matches!(result, Ok(Some(_))));
        peer.await.unwrap();
        assert!(UpstreamProxy::parse("http://localhost:0").is_err());
        assert!(proxy_authority("localhost:0").is_err());
    }
}

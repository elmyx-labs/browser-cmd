//! Reaching a destination through the upstream proxy: an HTTP CONNECT or a
//! SOCKS5 handshake, to an address already resolved.

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use crate::settings::upstream::{UpstreamKind, UpstreamProxy};
use crate::{Error, Result};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// How much of an upstream's refusal is quoted back: its status line, not the
/// error page a proxy may put behind it.
const REFUSAL_CHARS: usize = 160;

pub(super) async fn connect_via_proxy(
    proxy: &UpstreamProxy,
    target: SocketAddr,
    connect: Duration,
    header_cap: usize,
) -> Result<TcpStream> {
    tokio::time::timeout(connect, async {
        let mut stream = TcpStream::connect((&*proxy.host, proxy.port)).await?;
        match proxy.kind {
            UpstreamKind::Http => http_proxy_connect(&mut stream, proxy, target, header_cap).await?,
            UpstreamKind::Socks5 => socks5_connect(&mut stream, proxy, target).await?,
        }
        Ok(stream)
    })
    .await
    .map_err(|_| Error::Timeout("upstream proxy handshake"))?
}

async fn http_proxy_connect(
    stream: &mut TcpStream,
    proxy: &UpstreamProxy,
    target: SocketAddr,
    header_cap: usize,
) -> Result<()> {
    let authority = match target {
        SocketAddr::V4(address) => address.to_string(),
        SocketAddr::V6(address) => format!("[{}]:{}", address.ip(), address.port()),
    };
    let authorization = proxy.username.as_ref().map(|username| {
        let credentials = format!("{username}:{}", proxy.password.as_deref().unwrap_or_default());
        format!("Proxy-Authorization: Basic {}\r\n", crate::text::base64::base64(credentials.as_bytes()))
    });
    stream
        .write_all(
            format!(
                "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n{}Connection: keep-alive\r\n\r\n",
                authorization.as_deref().unwrap_or_default()
            )
            .as_bytes(),
        )
        .await?;
    let response = read_proxy_header(stream, header_cap).await?;
    let status = response.lines().next().unwrap_or_default();
    if status.split_whitespace().nth(1).is_none_or(|code| code != "200") {
        return Err(Error::Tool(format!(
            "upstream HTTP proxy refused tunnel: {}",
            crate::text::clip(status, REFUSAL_CHARS)
        )));
    }
    Ok(())
}

async fn socks5_connect(stream: &mut TcpStream, proxy: &UpstreamProxy, target: SocketAddr) -> Result<()> {
    let authenticated = proxy.username.is_some();
    stream.write_all(if authenticated { &[5, 1, 2] } else { &[5, 1, 0] }).await?;
    let mut greeting = [0_u8; 2];
    stream.read_exact(&mut greeting).await?;
    if greeting != [5, if authenticated { 2 } else { 0 }] {
        return Err(Error::Tool("upstream SOCKS5 proxy rejected authentication method".into()));
    }
    if authenticated {
        let username = proxy.username.as_deref().unwrap_or_default().as_bytes();
        let password = proxy.password.as_deref().unwrap_or_default().as_bytes();
        let mut request = vec![1, username.len() as u8];
        request.extend_from_slice(username);
        request.push(password.len() as u8);
        request.extend_from_slice(password);
        stream.write_all(&request).await?;
        let mut response = [0_u8; 2];
        stream.read_exact(&mut response).await?;
        if response != [1, 0] {
            return Err(Error::Tool("upstream SOCKS5 proxy rejected credentials".into()));
        }
    }
    let mut request = vec![5, 1, 0];
    match target.ip() {
        IpAddr::V4(address) => {
            request.push(1);
            request.extend_from_slice(&address.octets());
        }
        IpAddr::V6(address) => {
            request.push(4);
            request.extend_from_slice(&address.octets());
        }
    }
    request.extend_from_slice(&target.port().to_be_bytes());
    stream.write_all(&request).await?;
    let mut head = [0_u8; 4];
    stream.read_exact(&mut head).await?;
    if head[0] != 5 || head[1] != 0 {
        return Err(Error::Tool(format!("upstream SOCKS5 proxy refused tunnel with code {}", head[1])));
    }
    let address_bytes = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut length = [0_u8; 1];
            stream.read_exact(&mut length).await?;
            usize::from(length[0])
        }
        _ => return Err(Error::Tool("upstream SOCKS5 proxy returned an invalid address".into())),
    };
    let mut remainder = vec![0_u8; address_bytes + 2];
    stream.read_exact(&mut remainder).await?;
    Ok(())
}

pub(super) async fn read_proxy_header(stream: &mut TcpStream, cap: usize) -> Result<String> {
    let mut bytes = Vec::new();
    loop {
        if bytes.len() >= cap {
            return Err(Error::Tool("upstream HTTP proxy response header is too large".into()));
        }
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await?;
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return String::from_utf8(bytes)
                .map_err(|_| Error::Tool("upstream HTTP proxy returned a non-UTF-8 header".into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn authenticated_http_upstream_receives_a_pinned_connect_target() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let header = read_proxy_header(&mut stream, 32 * 1024).await.unwrap();
            assert!(header.starts_with("CONNECT 203.0.113.7:443 HTTP/1.1"), "{header}");
            assert!(header.contains("Proxy-Authorization: Basic dXNlcjpwYXNz\r\n"), "{header}");
            stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.unwrap();
        });
        let proxy = UpstreamProxy::parse(&format!("http://user:pass@{address}")).unwrap();
        let stream =
            connect_via_proxy(&proxy, "203.0.113.7:443".parse().unwrap(), Duration::from_secs(5), 32 * 1024)
                .await
                .unwrap();
        drop(stream);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn authenticated_socks5_upstream_receives_a_pinned_connect_target() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut greeting = [0_u8; 3];
            stream.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 2]);
            stream.write_all(&[5, 2]).await.unwrap();
            let mut auth = [0_u8; 11];
            stream.read_exact(&mut auth).await.unwrap();
            assert_eq!(&auth, b"\x01\x04user\x04pass");
            stream.write_all(&[1, 0]).await.unwrap();
            let mut connect = [0_u8; 10];
            stream.read_exact(&mut connect).await.unwrap();
            assert_eq!(connect, [5, 1, 0, 1, 203, 0, 113, 8, 1, 187]);
            stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await.unwrap();
        });
        let proxy = UpstreamProxy::parse(&format!("socks5://user:pass@{address}")).unwrap();
        let stream =
            connect_via_proxy(&proxy, "203.0.113.8:443".parse().unwrap(), Duration::from_secs(5), 32 * 1024)
                .await
                .unwrap();
        drop(stream);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn every_upstream_handshake_stage_is_bounded() {
        for stage in 0..4 {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                if stage == 0 {
                    read_proxy_header(&mut stream, 1024).await.unwrap();
                } else {
                    stream.read_exact(&mut [0; 3]).await.unwrap();
                    if stage >= 2 {
                        stream.write_all(&[5, 2]).await.unwrap();
                        stream.read_exact(&mut [0; 11]).await.unwrap();
                    }
                    if stage >= 3 {
                        stream.write_all(&[1, 0]).await.unwrap();
                        stream.read_exact(&mut [0; 10]).await.unwrap();
                    }
                }
                assert_eq!(
                    stream.read(&mut [0; 1]).await.unwrap(),
                    0,
                    "timed-out handshake must close its socket"
                );
            });
            let scheme = if stage == 0 { "http" } else { "socks5" };
            let proxy = UpstreamProxy::parse(&format!("{scheme}://user:pass@{address}")).unwrap();
            let result = tokio::time::timeout(
                Duration::from_secs(1),
                connect_via_proxy(
                    &proxy,
                    "203.0.113.1:443".parse().unwrap(),
                    Duration::from_millis(50),
                    1024,
                ),
            )
            .await
            .unwrap();
            assert!(matches!(result, Err(Error::Timeout(_))), "stage {stage}");
            peer.await.unwrap();
        }
    }
}

//! The CLI-daemon HTTP/1.1 policy: where the daemon listens, its limits, and
//! checks that keep a page in the driven Chrome from driving it.

use std::time::Duration;

use crate::{Error, Result};

pub(crate) const ENDPOINT: &str = "/call";
pub(crate) const SESSION_HEADER: &str = "browser-session";

const DEFAULT_ADDRESS: &str = "127.0.0.1:7788";

#[derive(Clone, Debug)]
pub(crate) struct Limits {
    /// Reading or writing one message, and a daemon starting up.
    pub(crate) io: Duration,
    /// One command, end to end.
    pub(crate) rpc: Duration,
    pub(crate) header_bytes: usize,
    pub(crate) request_bytes: usize,
    pub(crate) response_bytes: usize,
    pub(crate) connections: usize,
    pub(crate) sessions: usize,
    /// How long a session may sit unused before it is let go.
    pub(crate) idle: Duration,
}

impl Limits {
    pub(crate) fn from_env() -> Result<Self> {
        Ok(Self {
            io: Duration::from_secs(15),
            rpc: crate::env::seconds("RPC_SECS")?.unwrap_or(Duration::from_secs(600)),
            header_bytes: 32 * 1024,
            request_bytes: 8 * 1024 * 1024,
            response_bytes: 64 * 1024 * 1024,
            connections: 64,
            sessions: 256,
            idle: crate::env::seconds("IDLE_SECS")?.unwrap_or(Duration::from_secs(2 * 60 * 60)),
        })
    }
}

pub(crate) fn address() -> String {
    crate::env::var("ADDR").unwrap_or_else(|| DEFAULT_ADDRESS.to_owned())
}

/// A daemon off loopback requires it.
pub(crate) fn token() -> Result<Option<String>> {
    match crate::env::var("RPC_TOKEN") {
        None => Ok(None),
        Some(token) if token.bytes().all(|byte| byte.is_ascii_graphic()) => Ok(Some(token)),
        Some(_) => Err(Error::Config("BROWSER_CMD_RPC_TOKEN must be visible ASCII characters".into())),
    }
}

pub(crate) fn endpoint(address: &str) -> Result<url::Url> {
    let bad = || Error::Config("BROWSER_CMD_ADDR must be HOST:PORT with a nonzero port".into());
    let port = address
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .filter(|port| *port > 0)
        .ok_or_else(bad)?;
    let url = url::Url::parse(&format!("http://{address}")).map_err(|_| bad())?;
    if url.host().is_none() || !bare(&url) || url.port_or_known_default() != Some(port) {
        return Err(bad());
    }
    Ok(url)
}

pub(crate) fn loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    }
}

pub(crate) fn authorized(header: Option<&str>, expected: Option<&str>) -> bool {
    let Some(expected) = expected else { return true };
    let mut words = header.unwrap_or_default().split_whitespace();
    if !words.next().is_some_and(|word| word.eq_ignore_ascii_case("Bearer")) {
        return false;
    }
    let supplied = words.next().unwrap_or_default();
    if words.next().is_some() || supplied.len() != expected.len() {
        return false;
    }
    supplied.bytes().zip(expected.bytes()).fold(0_u8, |difference, (a, b)| difference | (a ^ b)) == 0
}

pub(crate) fn authority(value: &str, local: std::net::SocketAddr) -> Option<url::Url> {
    let url = url::Url::parse(&format!("http://{value}")).ok()?;
    if !bare(&url) || url.host().is_none() || url.port_or_known_default() != Some(local.port()) {
        return None;
    }
    if local.ip().is_loopback() && !local_origin(url.as_str()) {
        return None;
    }
    Some(url)
}

pub(crate) fn same_origin(origin: &str, host: &url::Url) -> bool {
    let Ok(url) = url::Url::parse(origin) else { return false };
    url.scheme() == "http"
        && bare(&url)
        && url.port_or_known_default() == host.port_or_known_default()
        && url.host() == host.host()
}

/// Origin is a URL authority, never a substring in an attacker-controlled string.
fn local_origin(origin: &str) -> bool {
    let Ok(url) = url::Url::parse(origin) else { return false };
    matches!(url.scheme(), "http" | "https") && bare(&url) && loopback(&url)
}

/// An authority and nothing else: no credentials, no path, query or fragment.
fn bare(url: &url::Url) -> bool {
    url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_require_a_nonzero_explicit_port_and_no_url_credentials_or_path() {
        for invalid in
            ["127.0.0.1:0", "localhost", "localhost:65536", "user:pass@localhost:7788", "localhost:7788/path"]
        {
            assert!(endpoint(invalid).is_err(), "{invalid}");
        }
        for valid in ["127.0.0.1:7788", "localhost:80", "[::1]:7788", "remote.example:7788"] {
            assert!(endpoint(valid).is_ok(), "{valid}");
        }
    }
}

#[cfg(test)]
mod origin_tests {
    use super::*;
    #[test]
    fn another_loopback_address_on_the_same_port_is_not_the_same_origin() {
        let host = url::Url::parse("http://127.0.0.1:7788").unwrap();
        assert!(same_origin("http://127.0.0.1:7788", &host));
        for origin in
            ["http://127.0.0.2:7788", "http://localhost:7788", "http://[::1]:7788", "http://127.0.0.1:3000"]
        {
            assert!(!same_origin(origin, &host), "{origin}");
        }
    }

    #[test]
    fn origins_are_exact_loopback_authorities() {
        for origin in [
            "https://localhost.evil.example",
            "http://127.0.0.1.evil.example",
            "https://evil.example/?x=://localhost",
            "http://user@localhost",
            "null",
            "file://localhost",
        ] {
            assert!(!local_origin(origin), "{origin}");
        }
        for origin in ["http://localhost:7788", "http://127.0.0.1", "http://[::1]:7788"] {
            assert!(local_origin(origin), "{origin}");
        }
    }
}

//! The upstream proxy an operator names in `BROWSER_CMD_PROXY`.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpstreamKind {
    Http,
    Socks5,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpstreamProxy {
    pub(crate) kind: UpstreamKind,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
}

impl UpstreamProxy {
    pub fn parse(value: &str) -> Result<Self> {
        let url = url::Url::parse(value)
            .map_err(|error| Error::Config(format!("invalid BROWSER_CMD_PROXY: {error}")))?;
        let kind = match url.scheme() {
            "http" => UpstreamKind::Http,
            "socks" | "socks5" => UpstreamKind::Socks5,
            // socks5h promises the upstream resolves names; this forwarder
            // resolves them itself and pins the address.
            "socks5h" => {
                return Err(Error::Config(
                    "BROWSER_CMD_PROXY socks5h is not supported: names are resolved locally; use socks5"
                        .into(),
                ));
            }
            scheme => {
                return Err(Error::Config(format!(
                    "BROWSER_CMD_PROXY scheme must be http or socks5, not {scheme}"
                )));
            }
        };
        if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
            return Err(Error::Config(
                "BROWSER_CMD_PROXY must not contain a path, query, or fragment".into(),
            ));
        }
        let host = proxy_host(&url)?;
        let port = url.port_or_known_default().unwrap_or(match kind {
            UpstreamKind::Http => 80,
            UpstreamKind::Socks5 => 1080,
        });
        if port == 0 {
            return Err(Error::Config("BROWSER_CMD_PROXY port must be nonzero".into()));
        }
        let decode = |value: &str| {
            percent_encoding::percent_decode_str(value)
                .decode_utf8()
                .map(|value| value.into_owned())
                .map_err(|_| Error::Config("BROWSER_CMD_PROXY credentials are not valid UTF-8".into()))
        };
        let username = (!url.username().is_empty()).then(|| decode(url.username())).transpose()?;
        let password = url.password().map(decode).transpose()?;
        if password.is_some() && username.is_none() {
            return Err(Error::Config("BROWSER_CMD_PROXY password requires a username".into()));
        }
        if matches!(kind, UpstreamKind::Socks5)
            && (username.as_ref().is_some_and(|value| value.len() > 255)
                || password.as_ref().is_some_and(|value| value.len() > 255))
        {
            return Err(Error::Config("SOCKS5 proxy credentials exceed 255 bytes".into()));
        }
        Ok(Self { kind, host, port, username, password })
    }
}

pub(crate) fn proxy_host(url: &url::Url) -> Result<String> {
    match url.host() {
        Some(url::Host::Domain(host)) if !host.is_empty() => Ok(host.to_owned()),
        Some(url::Host::Ipv4(host)) => Ok(host.to_string()),
        Some(url::Host::Ipv6(host)) => Ok(host.to_string()),
        _ => Err(Error::Tool("proxy URL has no host".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_proxy_configuration_supports_authenticated_http_and_socks5() {
        assert_eq!(
            UpstreamProxy::parse("http://user:p%40ss@proxy.example:3128").unwrap(),
            UpstreamProxy {
                kind: UpstreamKind::Http,
                host: "proxy.example".into(),
                port: 3128,
                username: Some("user".into()),
                password: Some("p@ss".into()),
            }
        );
        assert_eq!(UpstreamProxy::parse("socks5://127.0.0.1").unwrap().port, 1080);
        assert!(UpstreamProxy::parse("https://proxy.example").is_err());
        assert!(UpstreamProxy::parse("http://proxy.example/path").is_err());
    }
}

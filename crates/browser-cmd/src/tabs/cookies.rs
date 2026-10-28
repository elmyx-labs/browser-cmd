//! Cookie transfer for a clean context. Values never appear in diagnostics.
use crate::{Error, Result, chrome::cdp::Cdp};
use serde_json::{Value, json};

/// The host- and domain-scoped cookies of `url` in a context, the default one
/// when none, read before the active context changes.
pub(crate) async fn of(cdp: &Cdp, url: &url::Url, context: Option<&str>) -> Result<Vec<Value>> {
    let ask = match context {
        Some(context) => json!({"browserContextId":context}),
        None => json!({}),
    };
    select(&cdp.call(None, "Storage.getCookies", ask).await?, url)
}

pub(crate) fn select(reply: &Value, url: &url::Url) -> Result<Vec<Value>> {
    let host = url.host_str().ok_or_else(|| Error::Tool("cookie transfer needs a host".into()))?;
    let cookies = reply["cookies"]
        .as_array()
        .ok_or_else(|| Error::Protocol("browser returned no cookie list".into()))?;
    let mut selected = Vec::new();
    for cookie in cookies {
        let domain = cookie["domain"]
            .as_str()
            .filter(|domain| !domain.is_empty())
            .ok_or_else(|| Error::Protocol("browser cookie has no domain".into()))?;
        let matches = match domain.strip_prefix('.') {
            Some(domain) => host == domain || host.ends_with(&format!(".{domain}")),
            None => host == domain,
        };
        if matches {
            parameters(cookie)?;
            selected.push(cookie.clone());
        }
    }
    Ok(selected)
}

fn parameters(cookie: &Value) -> Result<Value> {
    let invalid = || Error::Protocol("browser returned an unsupported cookie record".into());
    for field in ["name", "value", "domain", "path"] {
        if !cookie[field].is_string() {
            return Err(invalid());
        }
    }
    for field in ["secure", "httpOnly", "session"] {
        if !cookie[field].is_boolean() {
            return Err(invalid());
        }
    }
    if cookie["partitionKeyOpaque"] == true {
        return Err(Error::Tool("an opaque partitioned cookie cannot be transferred faithfully".into()));
    }
    let mut params = serde_json::Map::new();
    // CookieParam fields, excluding read-only Cookie metadata (size/session).
    for field in [
        "name",
        "value",
        "domain",
        "path",
        "secure",
        "httpOnly",
        "sameSite",
        "expires",
        "priority",
        "sameParty",
        "sourceScheme",
        "sourcePort",
        "partitionKey",
    ] {
        if let Some(value) = cookie.get(field) {
            params.insert(field.into(), value.clone());
        }
    }
    if cookie["session"] == true {
        params.remove("expires");
    }
    let domain = cookie["domain"].as_str().unwrap();
    if !domain.starts_with('.') {
        // Giving Chrome a URL preserves host-only scope, including __Host- cookies.
        let scheme =
            if cookie["secure"] == true || cookie["sourceScheme"] == "Secure" { "https" } else { "http" };
        let url = url::Url::parse(&format!("{scheme}://{domain}/")).map_err(|_| invalid())?;
        if url.host_str() != Some(domain) || !url.username().is_empty() || url.password().is_some() {
            return Err(invalid());
        }
        params.remove("domain");
        params.insert("url".into(), json!(url.as_str()));
    }
    Ok(Value::Object(params))
}

pub(crate) async fn restore(cdp: &Cdp, context: &str, cookies: &[Value]) -> Result<()> {
    if cookies.is_empty() {
        return Ok(());
    }
    let params: Vec<Value> = cookies.iter().map(parameters).collect::<Result<_>>()?;
    cdp.call(None, "Storage.setCookies", json!({"browserContextId":context,"cookies":params})).await?;
    let actual = cdp.call(None, "Storage.getCookies", json!({"browserContextId":context})).await?;
    let actual = actual["cookies"]
        .as_array()
        .ok_or_else(|| Error::Protocol("cannot verify transferred cookies: cookie list missing".into()))?;
    for expected in cookies {
        let found = actual.iter().any(|candidate| {
            expected.as_object().is_some_and(|fields| {
                fields
                    .iter()
                    .filter(|(key, _)| key.as_str() != "size")
                    .all(|(key, value)| candidate.get(key) == Some(value))
            })
        });
        if !found {
            return Err(Error::Tool(
                "cookie transfer verification failed; at least one cookie or attribute was not preserved"
                    .into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cookie(domain: &str) -> Value {
        json!({"name":"session","value":"private-value","domain":domain,"path":"/api",
            "secure":true,"httpOnly":true,"session":true,"expires":-1})
    }
    #[test]
    fn selection_preserves_www_and_distinguishes_host_only_from_domain_cookies() {
        let url = url::Url::parse("https://www.example.test/").unwrap();
        let cookies = vec![
            cookie("www.example.test"),
            cookie("example.test"),
            cookie(".example.test"),
            cookie("other.example.test"),
            cookie(".unrelated.test"),
        ];
        assert_eq!(
            select(&json!({"cookies":cookies}), &url).unwrap(),
            vec![cookie("www.example.test"), cookie(".example.test")]
        );
    }
    #[test]
    fn host_only_parameters_preserve_scope_and_session_lifetime() {
        let params = parameters(&cookie("www.example.test")).unwrap();
        assert_eq!(params["url"], "https://www.example.test/");
        assert!(params.get("domain").is_none() && params.get("expires").is_none());
        assert_eq!(params["path"], "/api");
        assert_eq!(parameters(&cookie(".example.test")).unwrap()["domain"], ".example.test");
    }
    #[test]
    fn unreadable_lists_and_opaque_partitions_are_errors_without_cookie_values() {
        let url = url::Url::parse("https://www.example.test/").unwrap();
        assert!(select(&json!({}), &url).is_err());
        let mut opaque = cookie("www.example.test");
        opaque["partitionKeyOpaque"] = json!(true);
        let error = select(&json!({"cookies":[opaque]}), &url).unwrap_err().to_string();
        assert!(!error.contains("private-value"));
    }
}

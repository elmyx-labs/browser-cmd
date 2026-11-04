//! Page addresses: which ones this browser opens, and one with a challenge's parameters left behind.

use crate::{Error, Result};

/// A page address this browser will open: http or https, with a host.
pub(crate) fn web_url(input: &str) -> Result<url::Url> {
    let url = url::Url::parse(input).map_err(|error| Error::Tool(format!("invalid browser URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::Tool(format!(
            "browser navigation accepts only http and https, not {}",
            url.scheme()
        )));
    }
    if url.host_str().is_none() {
        return Err(Error::Tool("browser URL has no host".into()));
    }
    Ok(url)
}

/// The query parameter prefix a challenge adds to the address it stands at.
/// It belongs to that one attempt, so a clean retry leaves it behind.
const CHALLENGE_PARAMS: &str = "__cf_chl_";

pub(crate) fn without_challenge(mut url: url::Url) -> url::Url {
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(name, _)| !name.starts_with(CHALLENGE_PARAMS))
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !kept.is_empty() {
        url.query_pairs_mut().extend_pairs(kept);
    }
    url
}

/// Query parameters that only say how a visit came about, on any site and on
/// the sites that add their own; a name ending in `_` is a prefix.
const TRACKING: &[&str] = &[
    "utm_", "gclid", "gbraid", "wbraid", "dclid", "fbclid", "msclkid", "yclid", "mc_cid", "mc_eid", "igshid",
    "_gl",
];
const SITE_TRACKING: &[(&str, &[&str])] = &[
    (
        "google.",
        &[
            "ved", "ei", "sa", "usg", "oq", "gs_lcrp", "gs_lp", "sca_esv", "sca_upv", "sxsrf", "sclient",
            "uact", "iflsig", "rlz", "bih", "biw", "dpr", "aqs", "sourceid", "ie", "client",
        ],
    ),
    (
        "amazon.",
        &["ref", "ref_", "qid", "sr", "crid", "sprefix", "dib", "dib_tag", "content-id", "pd_rd_", "pf_rd_"],
    ),
    ("youtube.", &["si", "pp", "feature", "ab_channel"]),
];

/// An address as the agent is shown it: a search engine's redirect is the
/// address it leads to, and the parameters that only track the visit are gone.
/// The rest of the query is kept byte for byte, so the address still opens.
pub(crate) fn shown(address: &str) -> String {
    let Ok(mut url) = url::Url::parse(address) else { return address.to_owned() };
    let host = url.host_str().unwrap_or_default().to_owned();
    if host.contains("google.")
        && matches!(url.path(), "/url" | "/goto")
        && let Some(target) = url
            .query_pairs()
            .find(|(name, _)| name == "q" || name == "url")
            .and_then(|(_, to)| web_url(&to).ok())
    {
        return shown(target.as_str());
    }
    let site = SITE_TRACKING.iter().filter(|(site, _)| host.contains(site)).flat_map(|(_, names)| *names);
    let tracking = |name: &str| {
        TRACKING.iter().chain(site.clone()).any(|t| name == *t || (t.ends_with('_') && name.starts_with(t)))
    };
    let kept: Vec<&str> = url
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !tracking(pair.split('=').next().unwrap_or(pair)))
        .collect();
    let query = kept.join("&");
    url.set_query((!query.is_empty()).then_some(query.as_str()));
    url.into()
}

#[cfg(test)]
mod tests {
    use super::shown;

    #[test]
    fn tracking_goes_redirects_lead_on_and_the_rest_stays_as_it_was() {
        assert_eq!(shown("https://shop.test/a?utm_source=x&id=7&gclid=1"), "https://shop.test/a?id=7");
        assert_eq!(shown("https://shop.test/a?utm_source=x"), "https://shop.test/a");
        assert_eq!(
            shown("https://www.google.com/search?q=blue+kettle&sa=X&ved=0ah&ei=Zz"),
            "https://www.google.com/search?q=blue+kettle"
        );
        assert_eq!(shown("https://other.test/?sa=1&q=%2C"), "https://other.test/?sa=1&q=%2C");
        assert_eq!(
            shown("https://www.google.com/url?q=https://site.test/p%3Futm_medium%3Dx%26k%3D1&sa=U"),
            "https://site.test/p?k=1"
        );
        assert_eq!(shown("https://www.google.com/goto?url=CAESbw"), "https://www.google.com/goto?url=CAESbw");
        assert_eq!(shown("not a url"), "not a url");
    }
}

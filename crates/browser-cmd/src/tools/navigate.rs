//! Going to a page, back and forward, and saying where the tab actually landed.

use serde_json::Value;

use crate::chrome::tab::Tab;
use crate::page::navigation::{self, Landing, history};
use crate::session::{Session, after};
use crate::tabs::registry::same_page;
use crate::tabs::status::TabStatus;
use crate::tools::args::args;
use crate::web::web_url;
use crate::{Error, Result};

/// How long to let a dropped connection settle before asking again.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(1);
pub(crate) async fn navigate(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(
        args,
        Args {
            url: String,
            #[serde(default)]
            here: bool,
        }
    );
    let tab = session.current().await?;
    let step = match args.url.trim() {
        "back" => -1,
        "forward" => 1,
        _ => {
            let url = web_url(&args.url)?;
            // A kept tab holds evidence and a borrowed one is somebody else's:
            // going elsewhere from either is going there in a new tab, unless here.
            let kept = session
                .book
                .registry
                .of(&tab.target)
                .filter(|_| !args.here)
                .filter(|entry| {
                    entry.kept.as_deref().is_some_and(|kept| !same_page(kept, url.as_str())) || entry.borrowed
                })
                .map(|entry| (entry.name(), if entry.borrowed { "borrowed" } else { "kept" }));
            let tab = match kept {
                Some(_) => crate::tabs::open(session).await?,
                None => tab,
            };
            go(session, &tab, url.as_str()).await?;
            let landed = tab.url().await.ok();
            let mut action = arrival(&url, landed.as_deref());
            if let Some((kept, why)) = kept {
                let name = session.book.handle(&tab.target);
                action.push_str(&format!(". {kept} is {why}, so this opened in a new tab, {name}"));
            }
            return after(session, &tab, &action).await;
        }
    };
    if !history(&tab, step).await? {
        return Err(Error::Tool(format!("there is no history entry to go {} to", args.url.trim())));
    }
    session.document.forget();
    after(session, &tab, if step < 0 { "Went back" } else { "Went forward" }).await
}

pub(crate) async fn go(session: &mut Session, tab: &Tab, url: &str) -> Result<()> {
    for attempt in 0..=1 {
        session.document.forget();
        let wait = session.settings().waits.navigate;
        let reason = match navigation::navigate(tab, url, wait).await? {
            Landing::Committed => return Ok(()),
            Landing::Failed(reason) => reason,
            Landing::NoAnswer => {
                session.book.mark(&tab.target, TabStatus::Failed("timeout".into()));
                return Err(Error::Tool(format!(
                    "{url} did not start answering within {} s, so loading was stopped; try it once more, then \
                     another source",
                    wait.as_secs()
                )));
            }
        };
        if attempt == 1 || !unreached(&reason) {
            session.book.mark(&tab.target, TabStatus::Failed(reason.clone()));
            return Err(Error::Tool(format!("navigation failed: {reason}")));
        }
        tokio::time::sleep(SETTLE).await;
    }
    unreachable!("both outcomes of the last attempt return")
}

/// A connection that never happened is not a fact about the site, so it is
/// retried once; a bad name, certificate or cancelled load is reported as is.
fn unreached(reason: &str) -> bool {
    matches!(
        reason,
        "net::ERR_TUNNEL_CONNECTION_FAILED"
            | "net::ERR_PROXY_CONNECTION_FAILED"
            | "net::ERR_CONNECTION_RESET"
            | "net::ERR_CONNECTION_CLOSED"
            | "net::ERR_CONNECTION_ABORTED"
            | "net::ERR_CONNECTION_FAILED"
            | "net::ERR_EMPTY_RESPONSE"
            | "net::ERR_SOCKET_NOT_CONNECTED"
            | "net::ERR_NETWORK_CHANGED"
            | "net::ERR_TIMED_OUT"
    )
}

/// Answering "Navigated." to a request that was redirected elsewhere is how a
/// caller comes to repeat, unchanged, a request that never worked.
fn arrival(asked: &url::Url, landed: Option<&str>) -> String {
    let Some(landed) = landed.and_then(|at| url::Url::parse(at).ok()) else {
        return "Navigated".into();
    };
    let same_page = landed.scheme() == asked.scheme()
        && landed.host_str() == asked.host_str()
        && landed.path() == asked.path();
    // A query is parameters, and sites add their own; but parameters are also
    // where a search lives, and a site that drops the ones it was given has
    // not done what was asked. Losing them is worth saying; gaining some is
    // not.
    let dropped: Vec<String> = asked
        .query_pairs()
        .filter(|(name, value)| !landed.query_pairs().any(|(had, was)| had == *name && was == *value))
        .map(|(name, _)| name.into_owned())
        .collect();
    match (same_page, dropped.is_empty()) {
        (true, true) => "Navigated".into(),
        (true, false) => format!(
            "Navigated, but the tab is at {landed}, which does not carry what was asked for: {}",
            dropped.join(", ")
        ),
        (false, _) => format!("Navigated, but the tab is at {landed} rather than the requested {asked}"),
    }
}

#[cfg(test)]
mod arrival_tests {
    use super::{arrival, unreached};

    fn at(url: &str) -> url::Url {
        url::Url::parse(url).unwrap()
    }

    #[test]
    fn a_navigation_that_landed_elsewhere_says_so_and_one_that_did_not_stays_quiet() {
        let asked = at("https://www.booking.com/searchresults.html?ss=Melbourne");
        // What booking.com really does with a search string it will not take.
        let bounced = "https://www.booking.com/index.html?label=example&errorc_searchstring_not_found=ss";
        let said = arrival(&asked, Some(bounced));
        assert!(said.starts_with("Navigated, but the tab is at"), "{said}");
        assert!(said.contains("errorc_searchstring_not_found"), "{said}");
        assert!(said.contains("searchresults.html"), "the request is named too: {said}");

        // The site's own parameters are not a different page, and saying so
        // every time would teach a reader to stop looking.
        assert_eq!(
            arrival(&asked, Some("https://www.booking.com/searchresults.html?ss=Melbourne&aid=1234")),
            "Navigated"
        );
        // But a search that arrived without its search is not the page that
        // was asked for, whatever the path says.
        let lost = arrival(&asked, Some("https://www.booking.com/searchresults.html?aid=1234"));
        assert!(lost.contains("does not carry what was asked for: ss"), "{lost}");
        assert_eq!(arrival(&at("https://a.test"), Some("https://a.test/")), "Navigated");
        // A different host is a different page, whatever the path.
        assert!(arrival(&at("https://a.test/in"), Some("https://login.a.test/in")).contains("login.a.test"));
        // Nothing to compare against is nothing to say.
        assert_eq!(arrival(&asked, None), "Navigated");
        assert_eq!(arrival(&asked, Some("not a url")), "Navigated");
    }

    #[test]
    fn a_connection_that_never_happened_is_told_from_an_answer_about_the_site() {
        assert!(unreached("net::ERR_TUNNEL_CONNECTION_FAILED"));
        assert!(unreached("net::ERR_CONNECTION_RESET"));
        assert!(unreached("net::ERR_TIMED_OUT"));
        assert!(!unreached("net::ERR_NAME_NOT_RESOLVED"));
        assert!(!unreached("net::ERR_CERT_AUTHORITY_INVALID"));
        assert!(!unreached("net::ERR_BLOCKED_BY_CLIENT"));
        assert!(!unreached("net::ERR_ABORTED"));
        assert!(!unreached(""));
    }
}

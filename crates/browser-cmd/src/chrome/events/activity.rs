//! Whether a tab's page is still moving, by observing session: navigations
//! committed, frames loading, same-site requests in flight. Settling reads it.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde_json::Value;

/// How long an in-flight same-site document, fetch or XHR request holds a page
/// unsettled. Past this it is a long poll or a stream, not a page loading.
const NETWORK_BUDGET: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub(super) struct Activity {
    tabs: HashMap<String, Moving>,
}

#[derive(Default)]
struct Moving {
    /// Committed navigations and frame membership changes.
    navigation: u64,
    loading: Loading,
    /// Same-site requests in flight, by request id; only seen once the
    /// Network domain is enabled.
    requests: HashMap<String, tokio::time::Instant>,
}

#[derive(Default)]
struct Loading {
    frames: HashSet<String>,
    requested: HashMap<String, tokio::time::Instant>,
}

impl Loading {
    /// No frame loading, and no navigation asked for recently enough to still be starting.
    fn quiet(&self, interval: Duration) -> bool {
        self.frames.is_empty() && self.requested.values().all(|at| at.elapsed() >= interval)
    }
}

impl Activity {
    fn tab(&mut self, session: &str) -> &mut Moving {
        self.tabs.entry(session.into()).or_default()
    }

    pub(super) fn event(&mut self, session: &str, method: &str, params: &Value) {
        self.loading(session, method, params);
        self.request(session, method, params);
        if matches!(
            method,
            "Page.frameNavigated"
                | "Page.navigatedWithinDocument"
                | "Page.frameAttached"
                | "Page.frameDetached"
        ) {
            let tab = self.tab(session);
            tab.navigation = tab.navigation.saturating_add(1);
        }
    }

    pub(super) fn detach(&mut self, session: &str) {
        self.tabs.remove(session);
    }

    fn loading(&mut self, session: &str, method: &str, params: &Value) {
        let Some(frame) = params["frameId"].as_str().or_else(|| params["frame"]["id"].as_str()) else {
            return;
        };
        match method {
            "Page.frameRequestedNavigation" if params["disposition"] == "currentTab" => {
                self.tab(session).loading.requested.insert(frame.into(), tokio::time::Instant::now());
            }
            "Page.frameStartedLoading" => {
                self.tab(session).loading.frames.insert(frame.into());
            }
            // After commit, DOM readiness/quiet govern settling; slow images must
            // not keep every action waiting for the entire network load.
            "Page.frameNavigated"
            | "Page.navigatedWithinDocument"
            | "Page.frameStoppedLoading"
            | "Page.frameDetached" => {
                if let Some(tab) = self.tabs.get_mut(session) {
                    tab.loading.frames.remove(frame);
                    tab.loading.requested.remove(frame);
                }
            }
            _ => {}
        }
    }

    fn request(&mut self, session: &str, method: &str, params: &Value) {
        let Some(id) = params["requestId"].as_str() else { return };
        match method {
            "Network.requestWillBeSent"
                if matches!(params["type"].as_str(), Some("Document" | "XHR" | "Fetch"))
                    && same_site(params["request"]["url"].as_str(), params["documentURL"].as_str()) =>
            {
                let requests = &mut self.tab(session).requests;
                requests.retain(|_, at| at.elapsed() < NETWORK_BUDGET);
                requests.entry(id.into()).or_insert_with(tokio::time::Instant::now);
            }
            "Network.loadingFinished" | "Network.loadingFailed" => {
                if let Some(tab) = self.tabs.get_mut(session) {
                    tab.requests.remove(id);
                }
            }
            _ => {}
        }
    }

    pub(super) fn network_quiet(&self, session: &str) -> bool {
        self.tabs
            .get(session)
            .is_none_or(|tab| tab.requests.values().all(|at| at.elapsed() >= NETWORK_BUDGET))
    }

    pub(super) fn navigation_epoch(&self, session: &str) -> u64 {
        self.tabs.get(session).map_or(0, |tab| tab.navigation)
    }

    pub(super) fn navigation_quiet(&self, session: &str, interval: Duration) -> bool {
        self.tabs.get(session).is_none_or(|tab| tab.loading.quiet(interval))
    }
}

/// Whether two URLs share a site, approximated as the last two labels of the
/// host: no public-suffix list is carried, so `a.co.uk` and `b.co.uk` match.
fn same_site(url: Option<&str>, document: Option<&str>) -> bool {
    let site = |url: Option<&str>| {
        let url = url::Url::parse(url?).ok()?;
        let labels: Vec<&str> = url.host_str()?.rsplitn(3, '.').take(2).collect();
        Some(labels.join("."))
    };
    site(url).is_some_and(|url| site(document) == Some(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_requested_navigation_that_never_started_stops_holding_the_page() {
        let mut activity = Activity::default();
        activity.event(
            "s",
            "Page.frameRequestedNavigation",
            &serde_json::json!({"frameId":"main","disposition":"currentTab"}),
        );
        assert!(!activity.navigation_quiet("s", Duration::from_secs(1)));
        activity
            .tab("s")
            .loading
            .requested
            .insert("main".into(), tokio::time::Instant::now() - Duration::from_secs(2));
        assert!(
            activity.navigation_quiet("s", Duration::from_secs(1)),
            "cancelled requests must not remain loading forever"
        );
    }

    #[test]
    fn only_young_same_site_page_requests_hold_the_network_unsettled() {
        let mut activity = Activity::default();
        let mut sent = |id: &str, kind: &str, url: &str| {
            let params = serde_json::json!({"requestId":id,"type":kind,"request":{"url":url},"documentURL":"https://www.shop.test/cart"});
            activity.event("s", "Network.requestWillBeSent", &params);
            activity.network_quiet("s")
        };
        assert!(sent("image", "Image", "https://cdn.shop.test/a.png"));
        assert!(
            sent("tracker", "XHR", "https://ads.example/collect"),
            "images and other sites do not hold a page"
        );
        assert!(!sent("api", "Fetch", "https://api.shop.test/cart"));
        assert!(activity.network_quiet("other"));
        activity.event("s", "Network.loadingFinished", &serde_json::json!({"requestId":"api"}));
        assert!(activity.network_quiet("s"));
        activity.event("s", "Network.requestWillBeSent", &serde_json::json!({"requestId":"poll","type":"XHR","request":{"url":"https://www.shop.test/poll"},"documentURL":"https://www.shop.test/cart"}));
        *activity.tab("s").requests.get_mut("poll").unwrap() -= NETWORK_BUDGET;
        assert!(activity.network_quiet("s"), "a long poll past the budget does not hold the page");
    }
}

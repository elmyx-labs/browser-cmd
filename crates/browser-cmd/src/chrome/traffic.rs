//! What a page fetched for itself — the JSON behind a map, a chart, a feed, a
//! socket's messages — kept per tab for `network` to read back. The data an
//! app draws from is a cleaner account of a canvas than its pixels. Seen only
//! on tabs with the Network domain on.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::{Value, json};

use crate::Result;
use crate::chrome::tab::Tab;

/// How many exchanges one tab keeps, newest last.
const RING: usize = 200;

/// How much of one socket message is kept: a socket's messages are gone once
/// received, so the text is all there is to read back.
const MESSAGE_CHARS: usize = 8000;

/// One response or socket message the page received.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Exchange {
    /// The browser's request id, which `Network.getResponseBody` takes.
    pub(crate) id: String,
    pub(crate) url: String,
    /// XHR, Fetch, EventSource or WebSocket.
    pub(crate) kind: String,
    pub(crate) status: u64,
    pub(crate) mime: String,
    /// Bytes on the wire, once loaded.
    pub(crate) bytes: Option<u64>,
    /// A socket message's text, which has no body to fetch later.
    pub(crate) message: Option<String>,
}

/// Keep this tab's exchanges from now on, again whenever it is attached anew,
/// and say its failed requests when `noted`.
pub(crate) async fn record(tab: &Tab, noted: bool) -> Result<()> {
    tab.call("Network.enable", json!({})).await?;
    let mut events = tab.cdp.events();
    events.watch_network(&tab.session, noted);
    events.traffic.asked.insert(tab.target.clone());
    Ok(())
}

/// Leave unfetched whatever the tab asks for at an address matching one of `patterns`.
pub(crate) async fn block(tab: &Tab, patterns: &[&str]) -> Result<()> {
    tab.call("Network.enable", json!({})).await?;
    tab.call("Network.setBlockedURLs", json!({ "urls": patterns })).await.map(drop)
}

/// A response body as text, `None` when it is not text; an error when the
/// browser no longer holds it.
pub(crate) async fn body(tab: &Tab, id: &str) -> Result<Option<String>> {
    let fetched = tab.call("Network.getResponseBody", json!({ "requestId": id })).await?;
    let text = fetched["body"].as_str().unwrap_or_default();
    if fetched["base64Encoded"] != true {
        return Ok(Some(text.to_owned()));
    }
    Ok(crate::text::base64::unbase64(text).and_then(|bytes| String::from_utf8(bytes).ok()))
}

#[derive(Default)]
pub(crate) struct Traffic {
    /// The tabs being recorded, by session.
    tabs: HashMap<String, TabState>,
    /// The pages asked to be recorded, by target, for every session attached to them while they are open.
    pub(crate) asked: HashSet<String>,
}

#[derive(Default)]
struct TabState {
    exchanges: VecDeque<Exchange>,
    /// Open sockets' addresses, by request id: their messages carry none.
    sockets: HashMap<String, String>,
}

impl Traffic {
    pub(crate) fn record(&mut self, session: &str) {
        self.tabs.entry(session.into()).or_default();
    }

    pub(crate) fn recording(&self, session: &str) -> bool {
        self.tabs.contains_key(session)
    }

    pub(crate) fn exchanges(&self, session: &str) -> Vec<Exchange> {
        self.tabs.get(session).into_iter().flat_map(|tab| &tab.exchanges).cloned().collect()
    }

    pub(crate) fn detach(&mut self, session: &str) {
        self.tabs.remove(session);
    }

    pub(crate) fn event(&mut self, session: &str, method: &str, params: &Value) {
        let Some(tab) = self.tabs.get_mut(session) else { return };
        let id = params["requestId"].as_str().unwrap_or_default();
        match method {
            "Network.responseReceived"
                if matches!(params["type"].as_str(), Some("XHR" | "Fetch" | "EventSource")) =>
            {
                let response = &params["response"];
                tab.push(Exchange {
                    id: id.into(),
                    url: response["url"].as_str().unwrap_or_default().into(),
                    kind: params["type"].as_str().unwrap_or_default().into(),
                    status: response["status"].as_u64().unwrap_or_default(),
                    mime: response["mimeType"].as_str().unwrap_or_default().into(),
                    bytes: None,
                    message: None,
                });
            }
            "Network.loadingFinished" => {
                if let Some(exchange) = tab.exchanges.iter_mut().rfind(|e| e.id == id) {
                    exchange.bytes = params["encodedDataLength"].as_f64().map(|bytes| bytes as u64);
                }
            }
            "Network.webSocketCreated" => {
                tab.sockets.insert(id.into(), params["url"].as_str().unwrap_or_default().into());
            }
            "Network.webSocketClosed" => {
                tab.sockets.remove(id);
            }
            "Network.webSocketFrameReceived" => {
                let payload = params["response"]["payloadData"].as_str().unwrap_or_default();
                let text = params["response"]["opcode"].as_u64() == Some(1);
                let url = tab.sockets.get(id).cloned().unwrap_or_default();
                tab.push(Exchange {
                    id: id.into(),
                    url,
                    kind: "WebSocket".into(),
                    status: 101,
                    mime: if text { "text" } else { "binary" }.into(),
                    bytes: Some(payload.len() as u64),
                    message: text.then(|| crate::text::clip(payload, MESSAGE_CHARS)),
                });
            }
            _ => {}
        }
    }
}

impl TabState {
    fn push(&mut self, exchange: Exchange) {
        if self.exchanges.len() >= RING {
            self.exchanges.pop_front();
        }
        self.exchanges.push_back(exchange);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_a_recording_tabs_data_responses_and_socket_text_are_kept_and_bounded() {
        let mut traffic = Traffic::default();
        let received = |id: &str, kind: &str| json!({"requestId":id,"type":kind,"response":{"url":"https://a.test/api","status":200,"mimeType":"application/json"}});
        traffic.event("s", "Network.responseReceived", &received("early", "XHR"));
        traffic.record("s");
        traffic.event("s", "Network.responseReceived", &received("image", "Image"));
        traffic.event("s", "Network.responseReceived", &received("api", "Fetch"));
        traffic.event("s", "Network.loadingFinished", &json!({"requestId":"api","encodedDataLength":512.0}));
        traffic.event("s", "Network.webSocketCreated", &json!({"requestId":"ws","url":"wss://a.test/live"}));
        traffic.event(
            "s",
            "Network.webSocketFrameReceived",
            &json!({"requestId":"ws","response":{"opcode":1,"payloadData":"{\"price\":3}"}}),
        );
        let kept = traffic.exchanges("s");
        assert_eq!(kept.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["api", "ws"]);
        assert_eq!(kept[0].bytes, Some(512));
        assert_eq!(
            (kept[1].url.as_str(), kept[1].message.as_deref()),
            ("wss://a.test/live", Some("{\"price\":3}"))
        );
        for n in 0..RING + 5 {
            traffic.event("s", "Network.responseReceived", &received(&n.to_string(), "XHR"));
        }
        assert_eq!(traffic.exchanges("s").len(), RING);
        traffic.detach("s");
        assert!(!traffic.recording("s") && traffic.exchanges("s").is_empty());
    }
}

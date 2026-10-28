//! A tab's frames and the documents they hold, and the epoch that tells a
//! read made within one main document from one a navigation crossed. Document
//! stability is checked across capture; it does not imply an atomic snapshot
//! of a live DOM.

use crate::Error;
use crate::Result;
use serde_json::{Value, json};

use crate::chrome::tab::Tab;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Document {
    frame_id: String,
    loader_id: String,
    url: String,
}

/// How many times a tab's main document has changed: whatever was read
/// between `of` and a `check` that passes was read from one document.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Epoch(u64);

impl Epoch {
    pub(crate) fn of(tab: &Tab) -> Self {
        Self(tab.cdp.events().navigation_epoch(&tab.session))
    }

    pub(crate) fn check(self, tab: &Tab) -> Result<()> {
        if self == Self::of(tab) { Ok(()) } else { Err(Error::DocumentChanged) }
    }
}

/// One frame inventory shared by state, text and observations. Identity is
/// checked at capture boundaries, while DOM changes inside those boundaries
/// remain possible and are reported as non-atomic evidence.
pub(crate) struct FrameSet {
    pub(crate) tree: Value,
    pub(crate) epoch: Epoch,
}

impl FrameSet {
    pub(crate) async fn capture(tab: &Tab) -> Result<Self> {
        let epoch = Epoch::of(tab);
        let tree = tab.call("Page.getFrameTree", json!({})).await?;
        Ok(Self { tree, epoch })
    }

    /// Confirm the captured documents still exist. `all_frames=false` retains
    /// the narrower text(ref=...) contract, which does not read child frames.
    pub(crate) async fn validate(&self, tab: &Tab, all_frames: bool) -> Result<()> {
        if all_frames {
            let current = tab.call("Page.getFrameTree", json!({})).await?;
            if self.documents()? != documents(&current)? {
                return Err(Error::DocumentChanged);
            }
        }
        self.epoch.check(tab)
    }

    pub(crate) fn documents(&self) -> Result<Vec<Document>> {
        documents(&self.tree)
    }
}

pub(crate) fn walk(tree: &Value) -> Vec<(&Value, Option<usize>)> {
    let mut frames = Vec::new();
    let mut pending = vec![(tree, None)];
    while let Some((node, parent)) = pending.pop() {
        let at = frames.len();
        frames.push((node, parent));
        pending.extend(
            node["childFrames"].as_array().into_iter().flatten().rev().map(|child| (child, Some(at))),
        );
    }
    frames
}

pub(crate) fn documents(tree: &Value) -> Result<Vec<Document>> {
    let mut result = walk(&tree["frameTree"])
        .into_iter()
        .map(|(node, _)| {
            let string = |key: &str| {
                node["frame"][key]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| Error::Tool(format!("frame identity is missing {key}")))
            };
            Ok(Document { frame_id: string("id")?, loader_id: string("loaderId")?, url: string("url")? })
        })
        .collect::<Result<Vec<_>>>()?;
    result.sort_by(|a, b| a.frame_id.cmp(&b.frame_id));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    /// A URL round trip can leave the returned tree identical. The epoch must
    /// therefore be checked after, rather than before, the final awaited read.
    #[tokio::test]
    async fn final_tree_read_cannot_hide_an_intervening_navigation() {
        use futures_util::{SinkExt, StreamExt};
        use std::{rc::Rc, time::Duration};
        use tokio_tungstenite::tungstenite::Message;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for read in 0..2 {
                let request = socket.next().await.unwrap().unwrap().into_text().unwrap();
                let request: Value = serde_json::from_str(&request).unwrap();
                assert_eq!(request["method"], "Page.getFrameTree");
                if read == 1 {
                    for url in ["about:blank#away", "about:blank"] {
                        let event = json!({"sessionId":"s", "method":"Page.navigatedWithinDocument", "params":{"frameId":"main", "url":url}});
                        socket.send(Message::Text(event.to_string().into())).await.unwrap();
                    }
                }
                let response = json!({"id":request["id"],"result":{"frameTree":{"frame":{"id":"main","loaderId":"l","url":"about:blank"}}}});
                socket.send(Message::Text(response.to_string().into())).await.unwrap();
            }
        });
        let cdp = Rc::new(
            crate::chrome::cdp::Cdp::connect(
                &format!("ws://{address}"),
                Duration::from_secs(1),
                crate::settings::Dialogs::Accept,
            )
            .await
            .unwrap(),
        );
        let tab = Tab::fixture(cdp, "target".into(), "s".into());
        let frames = FrameSet::capture(&tab).await.unwrap();
        assert!(matches!(frames.validate(&tab, true).await, Err(Error::DocumentChanged)));
        server.await.unwrap();
    }

    #[test]
    fn document_identity_includes_child_navigation_and_url_changes() {
        let a = json!({"frameTree":{"frame":{"id":"main","loaderId":"a","url":"https://example.org"},
            "childFrames":[{"frame":{"id":"child","loaderId":"b","url":"https://example.org/frame"}}]}});
        let mut b = a.clone();
        b["frameTree"]["childFrames"][0]["frame"]["loaderId"] = json!("c");
        assert_ne!(documents(&a).unwrap(), documents(&b).unwrap());
        assert!(documents(&json!({})).is_err());
    }
}

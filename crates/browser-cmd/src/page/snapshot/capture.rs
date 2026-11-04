//! The accessibility tree of each frame a tab reaches, and the document each frame holds.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::page::frames::walk;
use crate::{Error, Result};

pub(crate) struct Capture {
    pub(crate) frames: Vec<(String, String, Vec<Value>)>,
    pub(crate) bindings: HashMap<String, Vec<(String, u64)>>,
    pub(crate) complete: bool,
}

pub(crate) async fn capture(tab: &Tab, tree: &Value) -> Result<Capture> {
    tab.cdp.events().register_frames(&tab.session, &tree["frameTree"]);
    let mut capture = Capture { frames: Vec::new(), bindings: HashMap::new(), complete: true };
    let mut paths: Vec<Option<Vec<(String, u64)>>> = Vec::new();
    for (node, parent) in walk(&tree["frameTree"]) {
        let main = parent.is_none();
        let Some(mut ancestors) = parent.map_or(Some(Vec::new()), |parent| paths[parent].clone()) else {
            paths.push(None);
            continue;
        };
        let frame = &node["frame"];
        let id = frame["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| Error::Protocol("frame identity missing".into()))?;
        let epoch = tab.cdp.events().frame_document(&tab.session, id).ok_or(Error::DocumentChanged)?;
        ancestors.push((id.to_owned(), epoch));
        capture.bindings.insert(id.to_owned(), ancestors.clone());
        let url = frame["url"].as_str().unwrap_or_default().to_owned();
        let params = if main { json!({}) } else { json!({ "frameId": frame["id"] }) };
        // A frame from another site lives in another process, which this
        // tab's session does not reach; its controls can still be clicked
        // by x and y off a screenshot.
        match tab.call("Accessibility.getFullAXTree", params).await {
            Ok(tree) => {
                capture.frames.push((
                    id.to_owned(),
                    url,
                    tree["nodes"].as_array().cloned().unwrap_or_default(),
                ));
                paths.push(Some(ancestors));
            }
            Err(_) if main => {
                return Err(Error::Tool(
                    "the page has no accessibility tree yet — it may still be loading, or it draws to a \
                     canvas (a map, a game, a viewer). Wait a moment and retry, or read it another way: \
                     text, screenshot, or eval."
                        .into(),
                ));
            }
            Err(_) => {
                capture.complete = false;
                capture.frames.push((
                    id.to_owned(),
                    format!("{url} (cross-origin: not readable here)"),
                    Vec::new(),
                ));
                paths.push(None);
            }
        }
    }
    Ok(capture)
}

//! The document as the browser's DOM agent holds it: the whole tree, the node
//! under a point, and the files an input is handed.

use std::path::Path;

use serde_json::{Value, json};

use crate::Result;
use crate::chrome::tab::Tab;

/// The whole tree, through shadow roots and into frames.
pub(crate) async fn document(tab: &Tab) -> Result<Value> {
    tab.call("DOM.getDocument", json!({ "depth": -1, "pierce": true })).await
}

/// The node a click at a point in viewport coordinates would land on.
pub(crate) async fn node_at(tab: &Tab, (x, y): (f64, f64)) -> Result<Option<i64>> {
    let hit =
        tab.call("DOM.getNodeForLocation", json!({ "x": x.round() as i64, "y": y.round() as i64 })).await?;
    Ok(hit["backendNodeId"].as_i64())
}

pub(crate) async fn set_files(tab: &Tab, node: i64, path: &Path) -> Result<()> {
    tab.call("DOM.setFileInputFiles", json!({ "files": [path], "backendNodeId": node })).await?;
    Ok(())
}

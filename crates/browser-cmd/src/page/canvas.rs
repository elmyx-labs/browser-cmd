//! What a page draws on its canvases, which no tree or text holds: where they are, whether WebGL draws, and the text.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::Result;
use crate::chrome::tab::Tab;
use crate::page::js::{CANVASES, DRAWN_TEXT, WEBGL_RENDERER};
use crate::page::world::World;

/// The share of the view canvases must cover for their pixels to be what
/// settles the page, and for the agent to be shown them unasked.
pub(crate) const LARGE_CANVAS: f64 = 0.3;

/// Where the viewport's canvases are; what they are, which scrolling does not
/// change; and how much of the view they cover.
pub(crate) async fn canvases(world: &mut World, tab: &Tab) -> Option<(String, String, f64)> {
    let found = world.call(tab, CANVASES, Vec::new()).await.ok()?;
    let shown = found["canvases"].as_array().filter(|list| !list.is_empty())?;
    let coverage = found["coverage"].as_f64().unwrap_or(0.0);
    let name = |c: &Value| c["selector"].as_str().map(|s| format!(" {s}")).unwrap_or_default();
    let list = |each: &dyn Fn(&Value) -> String| shown.iter().map(each).collect::<Vec<_>>().join(", ");
    let place = list(&|c| format!("{}x{} at {},{}{}", c["width"], c["height"], c["x"], c["y"], name(c)));
    let kinds = list(&|c| format!("{}x{}{}", c["width"], c["height"], name(c)));
    Some((place, kinds, coverage))
}

/// This browser's WebGL renderer, `null` when it has no WebGL, asked once per
/// process: it is the machine's GPU, or its absence, that answers.
pub(crate) async fn webgl_renderer(world: &mut World, tab: &Tab) -> Result<Value> {
    static KNOWN: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    if let Some(known) = KNOWN.get() {
        return Ok(known.clone());
    }
    let renderer = world.call(tab, WEBGL_RENDERER, Vec::new()).await?;
    Ok(KNOWN.get_or_init(|| renderer).clone())
}

/// Record what every document the tab loads from now on draws as text: a
/// chart's labels and tooltips, a map's place names. Recording wraps the
/// page's own canvas functions in the page's own world, where a script can
/// notice, so it is the operator's choice (BROWSER_CMD_CANVAS_TEXT).
pub(crate) async fn record(tab: &Tab) -> Result<()> {
    tab.call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": DRAWN_TEXT })).await?;
    Ok(())
}

/// The text the view's canvases newly draw since `said`, each with its
/// viewport x,y; `said` then holds what they draw now. On a new document
/// everything drawn is new.
pub(crate) async fn drawn(tab: &Tab, said: &mut HashSet<String>, new_document: bool) -> Option<Vec<String>> {
    let read = tab
        .call(
            "Runtime.evaluate",
            json!({ "expression": "window[Symbol.for('browser-cmd.drawn')]?.() ?? []", "returnByValue": true }),
        )
        .await
        .ok()?;
    let now: Vec<String> = read["result"]["value"]
        .as_array()?
        .iter()
        .map(|item| format!("{} {},{}", json!(item["text"]), item["x"], item["y"]))
        .collect();
    if new_document {
        said.clear();
    }
    let mut seen = HashSet::new();
    let fresh: Vec<String> =
        now.iter().filter(|item| !said.contains(*item) && seen.insert(*item)).cloned().collect();
    *said = now.into_iter().collect();
    Some(fresh)
}

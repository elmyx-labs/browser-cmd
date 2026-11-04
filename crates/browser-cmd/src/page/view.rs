//! Where the viewport is and what it shows: its size, a picture of it in CSS
//! pixels, and how far the thing under a point can scroll.

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::input::rect::Rect;
use crate::page::js::{REACH, VIEWPORT};
use crate::page::world::World;
use crate::{Error, Result};

/// The viewport's size in CSS pixels.
pub(crate) async fn viewport(world: &mut World, tab: &Tab) -> Result<(f64, f64)> {
    let view = world.call(tab, VIEWPORT, Vec::new()).await?;
    let dimension = |key: &str| {
        view[key].as_f64().filter(|value| value.is_finite() && *value > 0.0).ok_or_else(|| {
            Error::Tool("viewport dimensions are unavailable; take a fresh observation".into())
        })
    };
    Ok((dimension("width")?, dimension("height")?))
}

/// Which quad of an element's box: `Border` for what a person sees of it,
/// `Content` for where to aim inside it.
pub(crate) enum Quad {
    Content,
    Border,
}

/// An element's box in the top frame's coordinates — which, unlike a frame's
/// own `getBoundingClientRect`, already carries the offset of every iframe it
/// sits inside. The quad is the one source: the reply's `width`/`height` are
/// the unrotated box and disagree with it whenever a transform applies. `None`
/// when the element has no box.
pub(crate) async fn box_of(tab: &Tab, node: i64, quad: Quad) -> Result<Option<Rect>> {
    let model = tab.call("DOM.getBoxModel", json!({ "backendNodeId": node })).await?;
    let quad = match quad {
        Quad::Content => "content",
        Quad::Border => "border",
    };
    Ok(Rect::around(&model["model"][quad]))
}

/// What a picture covers: the viewport, the whole page, or one box in
/// viewport coordinates — an element, a section — however far it reaches.
pub(crate) enum Area {
    Viewport,
    Page,
    Box(Rect),
}

/// A PNG of an area, `zoom` image pixels to a CSS pixel: on a screen of pixel
/// ratio 2 the raw capture is twice the size, and a point read off it would
/// land a click twice as far as meant unless asked for.
pub(crate) async fn capture(world: &mut World, tab: &Tab, area: Area, zoom: f64) -> Result<String> {
    let view = world.call(tab, VIEWPORT, Vec::new()).await?;
    let ratio = view["ratio"].as_f64().filter(|ratio| ratio.is_finite() && *ratio > 0.0).unwrap_or(1.0);
    let whole = !matches!(area, Area::Viewport);
    let mut clip = match area {
        Area::Page => {
            let size = &tab.call("Page.getLayoutMetrics", json!({})).await?["cssContentSize"];
            json!({ "x": 0, "y": 0, "width": size["width"], "height": size["height"] })
        }
        Area::Viewport => {
            json!({ "x": view["left"], "y": view["top"], "width": view["width"], "height": view["height"] })
        }
        Area::Box(rect) => {
            let offset = |key: &str| view[key].as_f64().unwrap_or_default();
            json!({
                "x": rect.left + offset("left"), "y": rect.top + offset("top"),
                "width": rect.width, "height": rect.height,
            })
        }
    };
    clip["scale"] = json!(zoom / ratio);
    let shot = tab
        .call(
            "Page.captureScreenshot",
            json!({ "format": "png", "captureBeyondViewport": whole, "clip": clip }),
        )
        .await?;
    shot["data"]
        .as_str()
        .filter(|data| !data.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Error::Tool("the browser returned no image".into()))
}

/// Where the thing under a point can travel, and where in it we are: the page
/// when no point is given. An action that says it scrolled 700 px and moved
/// nothing reads as progress; this is what turns "scrolled" into "at the bottom
/// already" or "1400 of 3200".
pub(crate) async fn reach(world: &mut World, tab: &Tab, at: Option<(f64, f64)>) -> Value {
    let (x, y) = at.map_or((Value::Null, Value::Null), |(x, y)| (json!(x), json!(y)));
    world.call(tab, REACH, vec![x, y]).await.unwrap_or(Value::Null)
}

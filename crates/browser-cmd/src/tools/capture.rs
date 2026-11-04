//! Keeping a record of the page: a screenshot and the evidence for a value.

use crate::chrome::tab::Tab;
use crate::input::rect::Rect;
use crate::page::js::{EVIDENCE, QUERY_SELECTOR};
use crate::page::view::{Area, Quad, box_of};
use crate::tools::aim::locate;
use serde_json::{Value, json};

use crate::session::Session;
use crate::tools::args::args;
use crate::{Error, Image, Result};

/// How far a picture may be enlarged: past this a small label is blur, not detail.
const MAX_ZOOM: f64 = 4.0;

/// An element by ref or selector is pictured whole however far past the window
/// it reaches; a viewport region is enlarged by `zoom` for small print.
pub(crate) async fn screenshot(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(
        args,
        Args {
            #[serde(default)]
            full_page: bool,
            r#ref: Option<String>,
            selector: Option<String>,
            region: Option<[f64; 4]>,
            zoom: Option<f64>,
        }
    );
    let zoom = args.zoom.unwrap_or(1.0);
    if !(1.0..=MAX_ZOOM).contains(&zoom) {
        return Err(Error::Tool(format!("zoom is a number from 1 to {MAX_ZOOM}")));
    }
    let tab = session.current().await?;
    if let Some([left, top, width, height]) = args.region {
        if args.full_page || args.r#ref.is_some() || args.selector.is_some() {
            return Err(Error::Tool("screenshot takes one of full, ref, selector or a region".into()));
        }
        let (view_width, view_height) =
            crate::page::view::viewport(&mut session.document.world, &tab).await?;
        let rect = Rect { left, top, width, height }
            .visible(view_width, view_height)
            .filter(|rect| rect.width >= 1.0 && rect.height >= 1.0)
            .ok_or_else(|| Error::Tool("the region is not inside the viewport".into()))?;
        let data =
            crate::page::view::capture(&mut session.document.world, &tab, Area::Box(rect), zoom).await?;
        let path = keep(session, data)?;
        let (x, y, w, h) = (rect.left, rect.top, rect.width, rect.height);
        return Ok(format!(
            "URL: {}\nScreenshot of the {w:.0}x{h:.0} region at {x:.0},{y:.0} of the viewport, enlarged \
             {zoom}x, follows; a point px,py in it is the viewport's {x:.0}+px/{zoom}, {y:.0}+py/{zoom}. \
             Saved to {}",
            crate::web::shown(&tab.url().await?),
            path.display()
        ));
    }
    let (node, what) = match (args.full_page, &args.r#ref, &args.selector) {
        (_, None, None) => (None, None),
        (false, Some(reference), None) => (Some(session.node(reference)?), Some(reference.clone())),
        (false, None, Some(selector)) => {
            let found =
                session.document.world.node_result(&tab, None, QUERY_SELECTOR, vec![json!(selector)]).await?;
            let node = found.node.ok_or_else(|| {
                Error::Tool(
                    found.detail.unwrap_or_else(|| format!("nothing on the page matches {selector:?}")),
                )
            })?;
            (Some(node), Some(format!("selector {selector:?}")))
        }
        _ => return Err(Error::Tool("screenshot takes one of full, ref or selector".into())),
    };
    let area = match node {
        // Brought into view first, so what draws itself on arrival is drawn.
        Some(node) => {
            locate(session, &tab, node).await?;
            Area::Box(
                box_of(&tab, node, Quad::Border)
                    .await?
                    .ok_or_else(|| Error::Tool("the element has no box to picture".into()))?,
            )
        }
        None if zoom != 1.0 => return Err(Error::Tool("zoom takes a region, a ref or a selector".into())),
        None if args.full_page => Area::Page,
        None => Area::Viewport,
    };
    let clip = match &area {
        Area::Box(rect) => Some((rect.width, rect.height)),
        _ => None,
    };
    let data = crate::page::view::capture(&mut session.document.world, &tab, area, zoom).await?;
    let path = keep(session, data)?;
    let (width, height) = crate::page::view::viewport(&mut session.document.world, &tab).await?;
    // A picture whose page is unknown is evidence for nothing.
    let at = crate::web::shown(&tab.url().await?);
    let raw = raw_document(session, &tab).await.map_or_else(String::new, |warning| format!("{warning}\n"));
    // Only a viewport shot's pixels are where a click lands; the model acts on what is said.
    let said = match (what, clip) {
        (Some(what), Some((w, h))) => format!(
            "Screenshot of {what}, {w:.0}x{h:.0} enlarged {zoom}x, follows; its pixels are not viewport coordinates, so \
             click by ref or by the viewport's x and y."
        ),
        _ if args.full_page => format!(
            "Screenshot of the whole page follows, {width} wide and taller than the {width}x{height} \
             window; its pixels are not viewport coordinates, so scroll to what you want before \
             clicking by point."
        ),
        _ if session.document.told.insert("screenshot") => format!(
            "Screenshot of the {width}x{height} viewport follows; x and y for clicks are its pixels from \
             the top left."
        ),
        _ => format!("Viewport screenshot {width}x{height} follows; its pixels are click x,y."),
    };
    Ok(format!("{raw}URL: {at}\n{said} Saved to {}", path.display()))
}

/// Show a picture to the model and keep it as a file.
fn keep(session: &mut Session, data: String) -> Result<std::path::PathBuf> {
    let bytes = crate::text::base64::unbase64(&data)
        .ok_or_else(|| Error::Tool("the browser returned no image".into()))?;
    let path = session.output.save_shot(session.host.settings(), &bytes)?;
    session.output.picture = Some(Image { media_type: "image/png".into(), data });
    Ok(path)
}

/// Which of the given strings a person would see in the viewport now, and
/// where; for each one not in view, whether and where else the page has it.
/// Facts about the rendered document only, never whether an answer is right.
pub(crate) async fn evidence(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args { values: Vec<String> });
    if args.values.is_empty() {
        return Err(Error::Tool("evidence needs at least one string to look for".into()));
    }
    let tab = session.current().await?;
    let found = session.document.world.call(&tab, EVIDENCE, vec![json!(args.values)]).await?;
    let (width, height) = crate::page::view::viewport(&mut session.document.world, &tab).await?;
    let raw = raw_document(session, &tab).await.map_or_else(String::new, |warning| format!("{warning}\n"));
    let mut out = format!("{raw}Evidence in the {width}x{height} viewport of {}:", tab.url().await?);
    for item in found.as_array().into_iter().flatten() {
        out.push_str(&format!("\n- {}: {}", json!(item["value"]), finding(item, height)));
    }
    out.push_str(
        "\nSearched: the rendered text and field values of the top document; frames, shadow roots, \
                   images and canvases are not read.",
    );
    Ok(out)
}

fn finding(item: &Value, height: f64) -> String {
    let count = |key: &str| item[key].as_u64().unwrap_or_default();
    let (total, visible, fields) = (count("total"), count("visible"), count("fields"));
    let fields = match fields {
        0 => String::new(),
        n => format!("; in the value of {n} form field{}", if n == 1 { "" } else { "s" }),
    };
    let first = &item["first"];
    let nearest = &item["nearest"];
    let said = if visible > 0 {
        format!(
            "visible at {},{} in <{}>; {visible} of {total} matches in view",
            first["x"],
            first["y"],
            first["tag"].as_str().unwrap_or_default()
        )
    } else if nearest.is_object() {
        let y = nearest["y"].as_f64().unwrap_or_default();
        let place = if y < 0.0 {
            "above the viewport"
        } else if y > height {
            "below the viewport"
        } else {
            "in the viewport but not visible"
        };
        format!(
            "not visible in the viewport; {total} match{} on the page, nearest at page y {} ({place}) in <{}>",
            if total == 1 { "" } else { "es" },
            nearest["pageY"],
            nearest["tag"].as_str().unwrap_or_default()
        )
    } else if total > 0 {
        format!("in the page text {total} time{} but not rendered", if total == 1 { "" } else { "s" })
    } else {
        "not in the page text".into()
    };
    format!("{said}{fields}")
}

/// A warning when the tab shows a raw document — JSON, plain text, XML —
/// rather than a rendered page, which a picture of is poor evidence of what a
/// person would see.
async fn raw_document(session: &mut Session, tab: &Tab) -> Option<String> {
    let document = session.document.world.call(tab, crate::page::js::DOCUMENT, Vec::new()).await.ok()?;
    let kind = document["contentType"].as_str()?;
    if kind.contains("json") {
        return Some("[warning] evidence page is raw JSON; open a rendered page for a screenshot".into());
    }
    (!(matches!(kind, "text/html" | "application/xhtml+xml" | "application/pdf")
        || kind.starts_with("image/")))
    .then(|| {
        format!("[warning] evidence page is a raw {kind} document; open a rendered page for a screenshot")
    })
}

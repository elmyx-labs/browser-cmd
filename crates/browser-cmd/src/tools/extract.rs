//! Named fields off the page as data, from one screen or down a virtualized list.

use crate::tools::collection;
use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::page::js::EXTRACT;
use crate::session::Session;
use crate::tools::args::args;
use crate::{Error, Result};

/// Fields by selector as JSON: cheaper than reading the page, and checkable.
/// A field may name an attribute after `@`: `a@href`, `img@src`, `input@value`.
pub(crate) async fn extract(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args, #[serde(deny_unknown_fields)] {
        fields: std::collections::BTreeMap<String, String>,
        each: Option<String>,
        limit: Option<usize>,
        #[serde(default)]
        all: bool,
        identity: Option<String>,
    });
    if args.fields.is_empty() {
        return Err(Error::Tool("extract needs at least one field".into()));
    }
    if args.limit == Some(0)
        || args.each.as_deref().is_some_and(|s| s.trim().is_empty())
        || args.identity.as_deref().is_some_and(|s| s.trim().is_empty())
    {
        return Err(Error::Tool(
            "extract selectors and identity must be nonempty; limit must be positive".into(),
        ));
    }
    if (args.all || args.identity.is_some()) && args.each.is_none() {
        return Err(Error::Tool("all and identity require each".into()));
    }
    let limit = args
        .limit
        .unwrap_or(session.settings().limits.extract_rows)
        .min(session.settings().limits.extract_rows);
    let spec = json!({"fields":args.fields, "each":args.each, "identity":args.identity,
        "limit":session.settings().limits.extract_rows});
    let tab = session.current().await?;
    if let Some(each) = args.each.as_deref().filter(|_| args.all) {
        return extract_all(session, &tab, &spec, each, limit).await;
    }
    let mut rows = collection::Rows::default();
    let full = extract_pass(session, &tab, &spec, args.identity.is_some(), limit, &mut rows).await?;
    let matched = rows.matched();
    collection::render(
        &rows,
        json!({
            "mode":"snapshot", "scope":"current_dom", "matched":matched,
            "termination":full.unwrap_or("snapshot"),
            "complete":full.is_none() && matched == rows.values.len(), "identity":args.identity,
        }),
        session.settings().limits.page_text,
    )
}

/// Adds the rows the page shows now; answers why the walk should stop, or
/// `None` to keep going.
async fn extract_pass(
    session: &mut Session,
    tab: &Tab,
    spec: &Value,
    keyed: bool,
    limit: usize,
    rows: &mut collection::Rows,
) -> Result<Option<&'static str>> {
    let batch: collection::Batch =
        serde_json::from_value(session.document.world.call(tab, EXTRACT, vec![spec.clone()]).await?)?;
    rows.append(&batch, keyed, limit)?;
    Ok(if batch.found > batch.rows.len() {
        Some("batch_limit")
    } else if rows.values.len() >= limit {
        Some("row_limit")
    } else {
        None
    })
}

/// Observe a virtualized list, telling rows apart by the caller's identity or
/// else by their field values.
/// Stalling is a termination reason, never proof of whole-list coverage.
async fn extract_all(
    session: &mut Session,
    tab: &Tab,
    spec: &Value,
    each: &str,
    limit: usize,
) -> Result<String> {
    // One near-screenful at a time, overlapping, so a recycled list never
    // skips the rows between two stops the way a big jump would; the
    // screenful is the box that clips the rows, not the whole window.
    let list = session.document.world.call(tab, crate::page::js::LIST, vec![json!(each)]).await?;
    let client = list["height"]
        .as_f64()
        .filter(|height| *height > 0.0)
        .ok_or_else(|| Error::Tool("cannot determine the list viewport height".into()))?;
    let (px, py) = (list["x"].as_f64().unwrap_or_default(), list["y"].as_f64().unwrap_or_default());
    session.hand.move_to(tab, px, py).await?;
    let step = (client * (1.0 - session.settings().limits.extract_scroll_overlap))
        .min(f64::from(session.settings().limits.scroll_max));
    let ceiling = session.settings().limits.scroll_passes;
    let mut rows = collection::Rows::default();
    let mut termination = "pass_limit";
    let mut last = (i64::MIN, i64::MIN);
    let mut still = 0;
    let mut passes = 0;
    while passes < ceiling {
        let before = rows.values.len();
        let full = extract_pass(session, tab, spec, true, limit, &mut rows).await?;
        passes += 1;
        if let Some(reason) = full {
            termination = reason;
            break;
        }
        let reach = crate::page::view::reach(&mut session.document.world, tab, Some((px, py))).await;
        let here = (reach["top"].as_i64().unwrap_or(0), reach["max"].as_i64().unwrap_or(0));
        // Stop only when neither rows nor scroll position change: a grid that
        // virtualises by transform never moves its scrollTop as it renders.
        let grew = rows.values.len() > before;
        if !grew && here == last {
            still += 1;
            if still >= session.settings().limits.extract_stall_passes {
                termination = "stalled";
                break;
            }
        } else {
            still = 0;
        }
        last = here;
        if passes == ceiling {
            break;
        }
        session.hand.wheel(tab, 0.0, step).await?;
        session.settle(tab).await;
    }
    collection::render(
        &rows,
        json!({
            "mode":"scroll", "scope":"forward_from_current_position", "passes":passes,
            "termination":termination, "complete":false, "identity":spec["identity"],
        }),
        session.settings().limits.page_text,
    )
}

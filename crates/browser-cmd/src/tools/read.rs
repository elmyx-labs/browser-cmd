//! Reading the page: its state, its text, its pictures and one element's markup.
//! Page content is untrusted data.

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::page::js::MARKUP;
use crate::page::view::{Quad, box_of};
use crate::session::Session;
use crate::text::fence::{bounded, bounded_state, untrusted};
use crate::text::passages::passages;
use crate::tools::args::{Empty, args, parse};
use crate::{Error, Result};

pub(crate) async fn snapshot(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(
        args,
        Args {
            #[serde(default)]
            full: bool,
            /// Append the viewport centre of each ref, for reasoning about where
            /// things sit — which column, above or below, near an edge.
            #[serde(default)]
            coords: bool,
            /// One element and what is inside it: a dialog, a list, a form.
            r#ref: Option<String>,
        }
    );
    let tab = session.current().await?;
    let state = session.document.state(&tab, args.full, true).await?;
    // All of it when asked for all of it, or for one part of it.
    let whole = (args.full || args.r#ref.is_some()).then(|| state.tree.clone());
    let mut state = session.document.shown(state, whole);
    if let Some(reference) = &args.r#ref {
        state = within(&state, reference)?;
    }
    if args.coords {
        let positions = positions(session, &tab).await;
        if !positions.is_empty() {
            state.push_str(&format!("\n\nPositions (viewport centre x,y of each ref):\n{positions}"));
        }
    }
    // A long tree is kept whole in a file and its head shown: the file is
    // there to search, and the head is what fits beside everything else.
    let limits = &session.settings().limits;
    if state.len() > limits.state {
        let saved = crate::workspace::spill(session.settings(), "snapshot", "txt", 1, state.as_bytes());
        if let Ok((path, _)) = saved {
            let head = crate::text::head_lines(&state, limits.state);
            return Ok(bounded_state(
                format!(
                    "{head}\n[{} of {} bytes shown; the whole tree is in {}: grep it, or `snapshot @ref` for one \
                     part]",
                    head.len(),
                    state.len(),
                    path.display()
                ),
                limits.state + 512,
            ));
        }
    }
    Ok(bounded_state(state, limits.page_text))
}

/// The header of a state and the one element `reference` names, with
/// everything indented under it.
fn within(state: &str, reference: &str) -> Result<String> {
    let handle = format!("@{}", reference.trim().trim_start_matches('@'));
    let (header, tree) = state.split_once("\n\n").unwrap_or(("", state));
    let lines: Vec<&str> = tree.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.split(' ').any(|word| word == handle))
        .ok_or_else(|| Error::Tool(format!("{handle} is not in this page's tree; take a snapshot")))?;
    let indent = |line: &str| line.len() - line.trim_start().len();
    let depth = indent(lines[start]);
    let end = lines[start + 1..]
        .iter()
        .position(|line| indent(line) <= depth)
        .map_or(lines.len(), |n| start + 1 + n);
    Ok(format!("{header}\n\n{}", lines[start..end].join("\n")))
}

/// Box models are queried concurrently, so a page of controls costs about one
/// round trip, not one per element. Refs without a box are left out.
pub(crate) async fn positions(session: &mut Session, tab: &Tab) -> String {
    let handles = session.document.refs.handles();
    let boxes = futures_util::future::join_all(handles.into_iter().map(|(handle, node)| async move {
        let rect = box_of(tab, node, Quad::Content).await.ok()??;
        let (x, y) = rect.centre();
        Some(format!("@{handle} {},{}", x.round(), y.round()))
    }))
    .await;
    boxes.into_iter().flatten().collect::<Vec<_>>().join("  ")
}

/// From the top, from a byte offset, or the passages around a phrase: a bound
/// that says only "truncated" leaves the rest unreachable, which reads as absent.
pub(crate) async fn text(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args, #[serde(deny_unknown_fields)] {
        find: Option<String>,
        #[serde(default)]
        from: usize,
        /// Inline each link as [text](url), for reading a page with no
        /// clean selector to extract from.
        #[serde(default)]
        links: bool,
        /// One element to read instead of the whole page.
        r#ref: Option<String>,
        max_bytes: Option<usize>,
        /// Unadorned innerText remains available for exact/raw reads.
        #[serde(default = "crate::tools::read::context_default")]
        context: bool,
        /// The site's header, menus, footer and cookie notice as well.
        #[serde(default)]
        full: bool,
    });
    if args.max_bytes == Some(0) {
        return Err(Error::Config("max_bytes must be positive".into()));
    }
    let within = args.r#ref.as_deref().map(|reference| session.node(reference)).transpose()?;
    let find = args.find.as_deref().map(str::trim).filter(|phrase| !phrase.is_empty());
    // A phrase is looked for everywhere: its absence is a claim about the page.
    let skip = (!args.full && find.is_none() && within.is_none()).then_some("`text full=true` reads them");
    let tab = session.current().await?;
    let reading =
        crate::page::reading::read(&mut session.document.world, &tab, args.links, within, args.context, skip)
            .await?;
    let partial = reading.partial();
    let at = reading.sources.first().map(|source| crate::web::shown(&source.url)).unwrap_or_default();
    let text = match args.links {
        true => linked(&reading.render()),
        false => reading.render(),
    };
    // A page whose meaning is in its pictures reads as empty text. The
    // alt text is that meaning written down, and an agent that would have
    // to know to ask `images` as well usually does not.
    let alts = if within.is_none() { alt_texts(session, &tab, &text).await } else { None };
    let text = match alts {
        Some(described) => format!("{text}\n{described}"),
        None => text,
    };
    let text = text.as_str();
    let limits = &session.settings().limits;
    let limit = args.max_bytes.unwrap_or(limits.text).min(limits.page_text);
    let body = match find {
        Some(phrase) => passages(text, phrase, limits.passage, limit, args.context)?,
        None => {
            let from = crate::text::ceil(text, args.from);
            let context = if args.context {
                crate::text::passages::context_before(
                    text,
                    from,
                    crate::text::passages::context_budget(limit),
                )
            } else {
                String::new()
            };
            let shown = crate::text::head(&text[from..], limit.saturating_sub(context.len()));
            if shown.is_empty() && from < text.len() {
                return Err(Error::Config(
                    "max_bytes is too small for the next UTF-8 character; increase it".into(),
                ));
            }
            let end = from + shown.len();
            // The first cut page is saved whole, once; reading on uses `from=`.
            let spilled = (end < text.len() && from == 0 && args.max_bytes.is_none())
                .then(|| crate::workspace::spill(session.settings(), "text", "txt", 1, text.as_bytes()))
                .and_then(Result::ok);
            let more = match (end < text.len(), spilled) {
                (true, Some((path, _))) => format!(
                    "\n[bytes {from}-{end} of {}; text from={end} continues; the whole text is saved to {}]",
                    text.len(),
                    path.display()
                ),
                (true, None) => {
                    format!("\n[bytes {from}-{end} of {}; text from={end} continues]", text.len())
                }
                (false, _) if from > 0 => format!("\n[bytes {from}-{end} of {end}; the end]"),
                (false, _) => String::new(),
            };
            format!("{context}{shown}{more}")
        }
    };
    let coverage =
        if partial { "\nCoverage: partial; unreadable frames prevent establishing absence." } else { "" };
    let empty = if text.trim().is_empty() {
        "\nReading: no rendered text captured; loading or visual-only content is possible. Try wait or screenshot."
    } else {
        ""
    };
    Ok(untrusted(&format!("URL: {at}{coverage}{empty}\n\n{body}")))
}

/// Each inlined link's address as the agent is shown addresses.
fn linked(text: &str) -> String {
    let link = regex::Regex::new(r"\]\((https?://[^)\s]+)\)").expect("a valid pattern");
    link.replace_all(text, |found: &regex::Captures| format!("]({})", crate::web::shown(&found[1])))
        .into_owned()
}

/// The alt text of images whose words the page's own text does not
/// already carry. Only `text` takes these: what `wait` and `verify`
/// call absence is a claim about the text, and it stays that.
async fn alt_texts(session: &mut Session, tab: &Tab, text: &str) -> Option<String> {
    let limit = session.settings().limits.images;
    let images = session.document.world.call(tab, crate::page::js::IMAGES, vec![json!(limit)]).await.ok()?;
    let mut seen = std::collections::HashSet::new();
    let described: Vec<&str> = images
        .as_array()?
        .iter()
        .filter_map(|image| image["alt"].as_str())
        .filter(|alt| !alt.is_empty() && !text.contains(*alt) && seen.insert(*alt))
        .collect();
    (!described.is_empty()).then(|| format!("Image descriptions: {}", described.join(" · ")))
}

pub(crate) async fn images(session: &mut Session, args: Value) -> Result<String> {
    parse::<Empty>(args)?;
    let tab = session.current().await?;
    let limit = session.settings().limits.images;
    let images = session.document.world.call(&tab, crate::page::js::IMAGES, vec![json!(limit)]).await?;
    let text = serde_json::to_string_pretty(&images)?;
    Ok(bounded(text, session.settings().limits.page_text))
}

/// One element's markup, `levels` ancestors up: what the tree and text do not
/// carry, from which an `extract` selector is written.
pub(crate) async fn markup(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(
        args,
        Args {
            r#ref: String,
            #[serde(default)]
            levels: u32,
        }
    );
    let node = session.node(&args.r#ref)?;
    let tab = session.current().await?;
    let shown = session.document.world.call_on(&tab, node, MARKUP, vec![json!(args.levels)]).await?;
    let body = shown.as_str().unwrap_or_default().to_owned();
    Ok(bounded(body, session.settings().limits.markup))
}

fn context_default() -> bool {
    true
}

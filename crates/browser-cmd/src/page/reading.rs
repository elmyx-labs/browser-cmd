//! The page's words: the text of the document and of every frame this tab can
//! reach, with the frames it could not read named rather than skipped.

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::page::frames::{FrameSet, walk};
use crate::page::js::READ;
use crate::page::world::{World, call_in, unwrap_result};
use crate::{Error, Result};

pub(crate) struct CapturedText {
    pub frame_id: String,
    pub url: String,
    /// `None` for a frame that could not be read.
    pub text: Option<String>,
}

/// A phrase absent from readable bodies is undecidable when another frame
/// could not be read. Frame labels and URLs are metadata, never body text.
pub(crate) struct TextReading {
    pub sources: Vec<CapturedText>,
}

impl TextReading {
    /// Whether a frame went unread, so absence cannot be established.
    pub fn partial(&self) -> bool {
        self.sources.iter().any(|source| source.text.is_none())
    }

    pub fn contains(&self, phrase: &str) -> Option<bool> {
        if self.sources.iter().any(|source| source.text.as_ref().is_some_and(|text| text.contains(phrase))) {
            return Some(true);
        }
        match self.partial() {
            true => None,
            false => Some(false),
        }
    }

    pub fn render(&self) -> String {
        self.sources
            .iter()
            .enumerate()
            .map(|(index, source)| match &source.text {
                Some(text) if index == 0 => text.clone(),
                Some(text) => format!("--- frame: {} ---\n{text}", source.url),
                None => format!("--- unreadable frame: {} ({}) ---", source.frame_id, source.url),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The page's text, or one element's. `links` writes each link as
/// `[text](url)`; `context` keeps headings, table columns and selected values
/// with the words they label; `skip` leaves out the site's header, menus,
/// footer and cookie notice, naming them in a line that ends with it, the way
/// to read them. A failed primary read is an error, never absence.
pub(crate) async fn read(
    world: &mut World,
    tab: &Tab,
    links: bool,
    within: Option<i64>,
    context: bool,
    skip: Option<&str>,
) -> Result<TextReading> {
    let frames = FrameSet::capture(tab).await?;
    let tree = &frames.tree;
    let main = &tree["frameTree"]["frame"];
    let arguments = vec![json!(links), json!(context), json!(skip)];
    let value = match within {
        Some(node) => world.call_on(tab, node, READ, arguments.clone()).await?,
        None => world.call(tab, READ, arguments.clone()).await?,
    };
    let text = match value {
        Value::String(text) => Some(text),
        Value::Null => None, // e.g. an SVG or XML document has no HTML body
        _ => return Err(Error::Tool("page text is not a string".into())),
    };
    let mut sources = vec![CapturedText {
        frame_id: main["id"].as_str().ok_or_else(|| Error::Tool("page has no main frame".into()))?.to_owned(),
        url: main["url"].as_str().unwrap_or_default().to_owned(),
        text,
    }];
    // One element is read as itself; a frame inside it is another document.
    if within.is_none() {
        for (node, _) in walk(&tree["frameTree"]).into_iter().skip(1) {
            if let Some(id) = node["frame"]["id"].as_str() {
                sources.push(CapturedText {
                    frame_id: id.to_owned(),
                    url: node["frame"]["url"].as_str().unwrap_or_default().to_owned(),
                    text: frame_text(world, tab, id, &arguments).await,
                });
            }
        }
    }
    frames.validate(tab, within.is_none()).await?;
    Ok(TextReading { sources })
}

/// One subframe's text, read in an isolated world of its own so a cross-origin
/// frame in this process is reached; a frame in another process fails here and
/// is `None`.
async fn frame_text(world: &World, tab: &Tab, frame: &str, arguments: &[Value]) -> Option<String> {
    let context = world.context_in(tab, frame).await.ok()?;
    let answer = call_in(tab, context, READ, arguments.to_vec()).await.ok()?;
    unwrap_result(answer).ok()?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_content_is_unknown_but_visible_matches_remain_provable() {
        let reading = TextReading {
            sources: vec![
                CapturedText {
                    frame_id: "main".into(),
                    url: "https://metadata-only.example".into(),
                    text: Some("Available evidence".into()),
                },
                CapturedText { frame_id: "child".into(), url: "about:srcdoc".into(), text: None },
            ],
        };
        assert_eq!(reading.contains("Available evidence"), Some(true));
        assert_eq!(reading.contains("hidden evidence"), None);
        assert_eq!(reading.contains("metadata-only.example"), None);
        let empty = TextReading {
            sources: vec![CapturedText {
                frame_id: "main".into(),
                url: "about:blank".into(),
                text: Some(String::new()),
            }],
        };
        assert_eq!(empty.contains("absent"), Some(false));
    }
}

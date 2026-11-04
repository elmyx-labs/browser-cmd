//! A PDF the tab shows, saved once per load for the pdf tools to read.

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::session::Session;

/// A PDF viewer draws nothing an accessibility tree can read, so the bytes are
/// saved with the workspace's media for the pdf tools to read.
pub(crate) async fn saved_pdf(session: &mut Session, tab: &Tab, document: &Value) -> Option<String> {
    if document["contentType"] != "application/pdf" {
        return None;
    }
    let loader = session.document.loader.clone()?;
    if session.document.saved_pdf.as_deref() == Some(loader.as_str()) {
        return None;
    }
    let limit = session.settings().limits.document;
    let fetched = session.document.world.call(tab, crate::page::js::PDF, vec![json!(limit)]).await.ok()?;
    if let Some(over) = fetched["over"].as_u64() {
        return Some(format!(
            "This is a PDF of {over} bytes, more than the {limit} this browser will carry; \
             open it another way."
        ));
    }
    let bytes = crate::text::base64::unbase64(fetched["data"].as_str()?)?;
    let (path, _) = crate::workspace::spill(session.settings(), "document", "pdf", 1, &bytes).ok()?;
    session.document.saved_pdf = Some(loader);
    let relative = session.relative(&path);
    Some(format!(
        "This document is a PDF, not a page: the state below is the viewer, which shows nothing. \
         It is saved to {relative} ({} bytes); read it with the pdf tools, inspect then read.",
        bytes.len()
    ))
}

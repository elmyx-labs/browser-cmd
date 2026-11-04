//! What the page fetched for itself, read back: the data behind a map, a
//! chart or a feed, which says more than the pixels drawn from it.

use serde_json::Value;

use crate::chrome::traffic::{self, Exchange};
use crate::session::Session;
use crate::text::fence::untrusted;
use crate::tools::args::args;
use crate::{Error, Result};

/// How many exchanges a listing shows, newest last.
const LISTED: usize = 40;
/// How many bodies one call reads, newest first.
const BODIES: usize = 3;

pub(crate) async fn network(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args, #[serde(deny_unknown_fields)] {
        find: Option<String>,
        #[serde(default)]
        body: bool,
        max_bytes: Option<usize>,
    });
    if args.max_bytes == Some(0) {
        return Err(Error::Config("max_bytes must be positive".into()));
    }
    let tab = session.current().await?;
    if !tab.cdp.events().traffic.recording(&tab.session) {
        traffic::record(&tab, false).await?;
        return Ok("Recording this tab's data requests from now on; what the page fetched before was not \
                   kept. Load the page again (navigate to it) or act on it, then run network again."
            .into());
    }
    let find = args.find.unwrap_or_default().to_lowercase();
    let matching: Vec<Exchange> = tab
        .cdp
        .events()
        .traffic
        .exchanges(&tab.session)
        .into_iter()
        .filter(|exchange| exchange.url.to_lowercase().contains(&find))
        .collect();
    let at = crate::web::shown(&tab.url().await?);
    if matching.is_empty() {
        return Ok(untrusted(&format!(
            "URL: {at}\nNo data responses{} recorded on this tab yet: act on the page or wait, then look again.",
            if find.is_empty() { String::new() } else { format!(" with {find:?} in their address") }
        )));
    }
    if !args.body {
        let shown = &matching[matching.len().saturating_sub(LISTED)..];
        let lines: Vec<String> = shown.iter().map(line).collect();
        return Ok(untrusted(&format!(
            "URL: {at}\n{} data responses, the last {} shown, newest last:\n{}\n`network FIND body=true` reads \
             the newest bodies whose address has FIND.",
            matching.len(),
            shown.len(),
            lines.join("\n")
        )));
    }
    let limits = &session.settings().limits;
    let each = args.max_bytes.unwrap_or(limits.text).min(limits.page_text / BODIES);
    let mut out = format!("URL: {at}");
    for exchange in matching.iter().rev().take(BODIES) {
        out.push_str(&format!("\n\n{}\n", line(exchange)));
        let whole = match &exchange.message {
            Some(message) => message.clone(),
            None => match traffic::body(&tab, &exchange.id).await {
                Ok(Some(text)) => text,
                Ok(None) => {
                    out.push_str("[a binary body, not text]");
                    continue;
                }
                Err(_) => {
                    out.push_str("[the browser no longer holds this body]");
                    continue;
                }
            },
        };
        let shown = crate::text::head(&whole, each);
        out.push_str(shown);
        if shown.len() < whole.len() {
            let saved = crate::workspace::spill(session.settings(), "network", "txt", 1, whole.as_bytes());
            out.push_str(&match saved {
                Ok((path, _)) => format!(
                    "\n[{} of {} bytes; the whole body is saved to {}]",
                    shown.len(),
                    whole.len(),
                    path.display()
                ),
                Err(_) => format!("\n[{} of {} bytes]", shown.len(), whole.len()),
            });
        }
    }
    Ok(untrusted(&out))
}

fn line(exchange: &Exchange) -> String {
    let bytes = exchange.bytes.map_or_else(|| "loading".into(), |bytes| format!("{bytes} B"));
    format!("{} {} {} {bytes} {}", exchange.status, exchange.kind, exchange.mime, exchange.url)
}

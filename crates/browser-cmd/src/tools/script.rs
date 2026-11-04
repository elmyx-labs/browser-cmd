//! The escape hatches: a caller's own script, the browser log, a raw CDP call.

use serde_json::{Value, json};

use crate::session::Session;
use crate::text::fence::{bounded, untrusted};
use crate::tools::args::args;
use crate::{Error, Result};

pub(crate) async fn eval(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args { expression: String });
    let tab = session.current().await?;
    let value = session.document.caller_world.evaluate(&tab, &args.expression).await?;
    Ok(bounded(crate::text::json(&value), session.settings().limits.page_text))
}

/// A rolling window of the browser log, newest last.
pub(crate) async fn console(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(
        args,
        Args {
            #[serde(default)]
            errors: bool,
        }
    );
    // Diagnostics must not launch, attach, or recover a browser. Keep the
    // failed connection's captured logs available until an actual action
    // changes the session.
    let Some(tab) = session.book.active.as_ref() else {
        return Ok(untrusted("No active tab; no browser log entries captured."));
    };
    let lines: Vec<String> = tab
        .cdp
        .events()
        .logs(&tab.session)
        .into_iter()
        .filter(|entry| !args.errors || entry["level"] == "error")
        .map(|entry| {
            let url = entry["url"]
                .as_str()
                .filter(|u| !u.is_empty())
                .map_or_else(String::new, |u| format!(" ({u})"));
            format!(
                "[{}/{}] {}{url}",
                entry["level"].as_str().unwrap_or("info"),
                entry["source"].as_str().unwrap_or("other"),
                entry["text"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    let body = if lines.is_empty() {
        "This tab has no captured browser log entries.".to_owned()
    } else {
        lines.join("\n")
    };
    Ok(untrusted(&body))
}

/// Runs on the tab's own session, so a page domain (DOM, Page, Input, Network) is in scope.
pub(crate) async fn cdp(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(
        args,
        Args {
            method: String,
            #[serde(default)]
            params: Value,
        }
    );
    let params = match args.params {
        Value::Null => json!({}),
        value @ Value::Object(_) => value,
        _ => return Err(Error::Tool("CDP params must be an object".into())),
    };
    let tab = session.current().await?;
    let value = tab.call(&args.method, params).await?;
    let text = serde_json::to_string_pretty(&value)?;
    Ok(bounded(text, session.settings().limits.page_text))
}

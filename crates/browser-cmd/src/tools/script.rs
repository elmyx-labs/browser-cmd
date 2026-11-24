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
    Ok(whole_or_saved(crate::text::json(&value), session.settings()))
}

/// A long value is kept whole in a file and its head shown, as a long tree is:
/// a bound that says only "truncated" leaves the rest out of reach.
fn whole_or_saved(value: String, settings: &crate::Settings) -> String {
    let limit = settings.limits.page_text;
    if value.len() <= limit {
        return untrusted(&value);
    }
    let Ok((path, _)) = crate::workspace::spill(settings, "eval", "txt", 1, value.as_bytes()) else {
        return bounded(value, limit);
    };
    let head = crate::text::head(&value, limit);
    untrusted(&format!(
        "{head}\n[{} of {} bytes shown; the whole value is in {}]",
        head.len(),
        value.len(),
        path.display()
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_value_is_saved_whole_and_a_short_one_is_shown_as_it_is() {
        let temp = tempfile::tempdir().unwrap();
        let settings = crate::Settings::new(temp.path(), temp.path());
        let short = whole_or_saved("[1,2]".into(), &settings);
        assert!(short.contains("[1,2]") && !short.contains("bytes shown"), "{short}");

        let long = format!("\"{}\"", "x".repeat(settings.limits.page_text + 10));
        let told = whole_or_saved(long.clone(), &settings);
        let path = told.split("the whole value is in ").nth(1).unwrap().split(']').next().unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), long);
        assert!(told.contains(&format!("of {} bytes shown", long.len())), "{told}");
        assert!(!told.contains("output truncated"), "{told}");
    }
}

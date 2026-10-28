//! Closing a step: waiting for the page to take a shape, and verifying what
//! must now be true.

use serde_json::Value;

use crate::page::frames::Epoch;
use crate::session::{Session, after};
use crate::text::fence::bounded_state;
use crate::tools::args::{OneOrMany, args};
use crate::{Error, Result};

/// The longest a `wait` may take, so it ends inside the daemon's command
/// deadline with an answer rather than an unknown outcome.
const WAIT_CAP: std::time::Duration = std::time::Duration::from_secs(90);

/// Unlike `wait`, looks once: a claim that does not hold is an error naming
/// what was there instead, so a long task splits into retryable steps.
pub(crate) async fn verify(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args, #[serde(deny_unknown_fields)] {
        #[serde(default)]
        text: OneOrMany,
        #[serde(default)]
        gone: OneOrMany,
        url: Option<String>,
        r#ref: Option<String>,
    });
    let (wanted, absent) = (args.text.all(), args.gone.all());
    if wanted.is_empty() && absent.is_empty() && args.url.is_none() && args.r#ref.is_none() {
        return Err(Error::Tool("verify needs something to check: text, gone, url or ref".into()));
    }
    if wanted.iter().chain(absent.iter()).any(|text| text.trim().is_empty())
        || args.url.as_deref().is_some_and(|url| url.trim().is_empty())
        || args.r#ref.as_deref().is_some_and(|reference| reference.trim().is_empty())
    {
        return Err(Error::Tool("verify conditions must not be empty".into()));
    }
    let tab = session.current().await?;
    // The handles a ref is checked against belong to the state as it is
    // now, not to the one before the action being verified.
    session.settle(&tab).await;
    let epoch = Epoch::of(&tab);
    let state = bounded_state(
        {
            let state = session.document.state(&tab, false, true).await?;
            session.document.shown(state, None)
        },
        session.settings().limits.state,
    );
    let body = if !wanted.is_empty() || !absent.is_empty() {
        Some(crate::page::reading::read(&mut session.document.world, &tab, false, None, false, None).await?)
    } else {
        None
    };
    let at = tab.url().await?;
    epoch.check(&tab)?;

    // Every phrase is its own condition, so one checkpoint names exactly which
    // did not hold. The page was read once, and only if a phrase asked for it.
    let mut failed = Vec::new();
    if let Some(body) = &body {
        for text in &wanted {
            let found = body
                .contains(text)
                .ok_or_else(|| Error::Tool(format!("cannot verify {text:?}: some frames are unreadable")))?;
            if !found {
                failed.push(format!("{text:?} is not on the page"));
            }
        }
        for gone in &absent {
            let found = body.contains(gone).ok_or_else(|| {
                Error::Tool(format!("cannot establish absence of {gone:?}: some frames are unreadable"))
            })?;
            if found {
                failed.push(format!("{gone:?} is still on the page"));
            }
        }
    }
    if let Some(url) = args.url.as_deref()
        && !at.contains(url)
    {
        failed.push(format!("the URL is {at:?}, which does not contain {url:?}"));
    }
    if let Some(target) = args.r#ref.as_deref()
        && let Err(reason) = session.resolve(target, &[])
    {
        failed.push(reason.to_string());
    }
    if !failed.is_empty() {
        return Err(Error::Tool(format!("the page is not as expected: {}.\n{state}", failed.join("; "))));
    }
    let checked = [
        (!wanted.is_empty()).then(|| format!("{wanted:?} on the page")),
        (!absent.is_empty()).then(|| format!("{absent:?} gone")),
        args.url.map(|url| format!("the URL contains {url:?}")),
        args.r#ref.map(|target| format!("{target} is here")),
    ];
    let checked: Vec<_> = checked.into_iter().flatten().collect();
    Ok(format!("Verified: {}.\n{state}", checked.join(", ")))
}

pub(crate) async fn wait(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args, #[serde(deny_unknown_fields)] {
        text: Option<String>,
        gone: Option<String>,
        url: Option<String>,
        seconds: Option<f64>,
    });
    if [args.text.as_deref(), args.gone.as_deref(), args.url.as_deref()]
        .into_iter()
        .flatten()
        .any(|value| value.trim().is_empty())
    {
        return Err(Error::Tool("wait conditions must not be empty".into()));
    }
    let limit = args
        .seconds
        .map(std::time::Duration::try_from_secs_f64)
        .transpose()
        .map_err(|_| Error::Tool("wait seconds must be finite and nonnegative".into()))?
        .unwrap_or(session.settings().waits.wait)
        .min(WAIT_CAP);
    let tab = session.current().await?;
    if args.text.is_none() && args.gone.is_none() && args.url.is_none() {
        tokio::time::sleep(limit.min(session.settings().waits.wait)).await;
        return after(session, &tab, "Waited").await;
    }
    let host = session.host.clone();
    let found = crate::page::settle::wait_for(
        &mut session.document.world,
        &tab,
        &host.settings().waits,
        args.text.as_deref(),
        args.gone.as_deref(),
        args.url.as_deref(),
        limit,
    )
    .await?;
    if !found {
        return Err(Error::Tool(format!(
            "the requested conditions were not observed within {}s; take a fresh observation to inspect the current state",
            limit.as_secs()
        )));
    }
    after(session, &tab, "The page changed as expected").await
}

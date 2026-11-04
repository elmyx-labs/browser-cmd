//! A clean retry: the same address from a fresh browser context, carrying the
//! site's cookies only when asked.

use serde_json::Value;

use crate::Result;
use crate::session::{Session, after};
use crate::tools::args::args;

/// The same URL again from a context with nothing in it: no cookies, no
/// cache, no reputation the persistent profile may have earned.
pub(crate) async fn retry_clean(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(
        args,
        Args {
            #[serde(default)]
            keep_signin: bool,
        }
    );
    let source = session.current().await?;
    let (tab, kept) = crate::tabs::context::retry(session, &source, args.keep_signin).await?;
    let how = match kept {
        0 => "Retried in a clean isolated browser context".to_owned(),
        1 => "Retried in a clean isolated browser context, carrying 1 cookie for this site".to_owned(),
        many => {
            format!("Retried in a clean isolated browser context, carrying {many} cookies for this site")
        }
    };
    after(session, &tab, &how).await
}

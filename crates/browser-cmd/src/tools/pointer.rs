//! The pointer: clicking, hovering and dragging.

use serde_json::Value;

use crate::session::{Session, after};
use crate::tools::aim::{covered_by, point};
use crate::tools::args::{Target, args, parse};
use crate::tools::submission::{Submit, guard_submission};
use crate::{Error, Result};

pub(crate) async fn click(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(
        args,
        Args {
            #[serde(flatten)]
            target: Target,
            #[serde(default)]
            double: bool,
            /// A right-button click, to open a page's own context menu.
            #[serde(default)]
            right: bool,
            /// Send the same form a second time on purpose.
            #[serde(default)]
            again: bool,
        }
    );
    args.target.validate()?;
    let tab = session.current().await?;
    if let Some(reference) = args.target.r#ref.as_deref()
        && crate::page::snapshot::is_handle(reference)
    {
        session.node(reference)?;
    }
    if session.hand.may_wander() {
        let (width, height) = crate::page::view::viewport(&mut session.document.world, &tab).await?;
        session.hand.wander(&tab, width, height).await?;
    }
    // The node a ref names, so the click can confirm it is still there once
    // the pointer arrives — a point (x, y) names nothing to confirm against.
    let (x, y) = point(session, &tab, &args.target, "click").await?;
    let node = args.target.r#ref.as_deref().and_then(|reference| session.node(reference).ok());
    session.hand.move_to(&tab, x, y).await?;
    if let Some(node) = node
        && let Some(cover) = covered_by(session, &tab, node, x, y).await?
    {
        return Err(Error::Tool(format!(
            "the target moved before the click landed — {cover} is under the cursor now; \
             take a fresh snapshot and try again"
        )));
    }
    if args.right {
        session.hand.secondary(&tab, x, y).await?;
    } else {
        // All targeting and cover checks succeeded. Reserve immediately
        // before dispatch; a failed aim must not count as a submission.
        guard_submission(session, &tab, Submit::Click { node, at: (x, y) }, args.again).await?;
        session.hand.press(&tab, x, y, if args.double { 2 } else { 1 }).await?;
    }
    session.hand.think().await;
    let did = if args.right {
        "Right-clicked"
    } else if args.double {
        "Double-clicked"
    } else {
        "Clicked"
    };
    after(session, &tab, did).await
}

pub(crate) async fn hover(session: &mut Session, args: Value) -> Result<String> {
    let target: Target = parse(args)?;
    target.validate()?;
    let tab = session.current().await?;
    let (x, y) = point(session, &tab, &target, "hover over").await?;
    session.hand.move_to(&tab, x, y).await?;
    session.hand.pause(crate::input::hand::tempo::BETWEEN_MOVES).await;
    after(session, &tab, "Hovered").await
}

pub(crate) async fn drag(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args { from: Target, to: Target });
    args.from.validate()?;
    args.to.validate()?;
    let tab = session.current().await?;
    let from = point(session, &tab, &args.from, "drag").await?;
    let to = point(session, &tab, &args.to, "drag to").await?;
    session.hand.drag(&tab, from, to).await?;
    session.hand.think().await;
    let name = |target: &Target, (x, y): (f64, f64)| {
        target.r#ref.clone().unwrap_or_else(|| format!("{x:.0}, {y:.0}"))
    };
    after(session, &tab, &format!("Dragged {} to {}", name(&args.from, from), name(&args.to, to))).await
}

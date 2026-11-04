//! The keyboard: typing into fields, choosing options and pressing keys.

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::input::hand::Hand;
use crate::page::js::{CLEAR_EDITABLE, EDITABLE, FIELD_VALUE, SELECT_OPTION, SET_VALUE};
use crate::session::{Session, after};
use crate::tools::aim::aim;
use crate::tools::args::args;
use crate::tools::submission::{Submit, guard_submission};
use crate::{Error, Result};

/// How much of a field's reading a mismatch quotes.
const READING_CHARS: usize = 200;

pub(crate) async fn type_text(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args {
        r#ref: Option<String>,
        text: String,
        #[serde(default)]
        submit: bool,
        #[serde(default)]
        again: bool,
    });
    let tab = session.current().await?;
    let reads = match args.r#ref.as_deref() {
        Some(reference) => enter(session, &tab, reference, &args.text)
            .await?
            .map_or_else(String::new, |now| format!("; field now reads {now:?}")),
        None => {
            insert(session, &tab, &args.text).await?;
            format!("; {} characters", args.text.chars().count())
        }
    };
    if args.submit {
        let node = args.r#ref.as_deref().and_then(|reference| session.field(reference).ok());
        guard_submission(session, &tab, Submit::Enter { node }, args.again).await?;
        crate::input::keys::press(&tab, &mut session.hand, "Enter").await?;
    }
    let done = if args.submit { "Typed and submitted" } else { "Typed" };
    let combobox = args.r#ref.as_deref().filter(|reference| {
        let node = session.field(reference).ok();
        !args.submit && node.and_then(|node| session.document.refs.role_for(node)) == Some("combobox")
    });
    let hint = combobox.map_or_else(String::new, |reference| {
        format!(
            "\nTyped text is not a choice: choose {reference} \"TEXT\" picks one of its suggestions and \
             confirms it took."
        )
    });
    Ok(after(session, &tab, &format!("{done}{reads}")).await? + &hint)
}

pub(crate) async fn select(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args { r#ref: String, option: String });
    let node = session.resolve(&args.r#ref, &["combobox", "listbox"])?;
    let tab = session.current().await?;
    let chosen = session.document.world.call_on(&tab, node, SELECT_OPTION, vec![json!(args.option)]).await?;
    if let Some(error) = chosen["error"].as_str() {
        let options = chosen["options"].as_array().map(|options| {
            format!(
                "; the options are {}",
                options.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
            )
        });
        return Err(Error::Tool(format!(
            "cannot select in {}: {error}{}",
            args.r#ref,
            options.unwrap_or_default()
        )));
    }
    after(session, &tab, &format!("Selected {:?}", chosen["chosen"].as_str().unwrap_or_default())).await
}

pub(crate) async fn press(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(
        args,
        Args {
            key: String,
            #[serde(default)]
            again: bool,
        }
    );
    let tab = session.current().await?;
    // The key itself, whatever chord or spelling it was named in.
    let (_, key) = crate::input::keys::parse_key(&args.key)?;
    match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => {
            guard_submission(session, &tab, Submit::Enter { node: None }, args.again).await?
        }
        "space" | " " => guard_submission(session, &tab, Submit::Space, args.again).await?,
        _ => {}
    }
    crate::input::keys::press(&tab, &mut session.hand, &args.key).await?;
    after(session, &tab, &format!("Pressed {}", args.key)).await
}

/// Text into whatever has focus, as typing would put it there: for an editor
/// the tree gives no ref — a canvas document, a sheet, a frame's rich text —
/// once a click has put the caret in it. Each line after the first is entered
/// with the Enter key, which is how such editors start a new paragraph.
async fn insert(session: &mut Session, tab: &Tab, text: &str) -> Result<()> {
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            crate::input::keys::press(tab, &mut session.hand, "Enter").await?;
        }
        if !line.is_empty() {
            Hand::insert(tab, line).await?;
        }
    }
    Ok(())
}

/// Replace one field's text. What it reads afterwards comes back when a mask,
/// a length limit or a script rewrote it: reported, not refused.
pub(crate) async fn enter(
    session: &mut Session,
    tab: &Tab,
    reference: &str,
    text: &str,
) -> Result<Option<String>> {
    let node = session.field(reference)?;
    if session.document.world.call_on(tab, node, EDITABLE, Vec::new()).await? != Value::Bool(true) {
        return Err(Error::Tool(format!(
            "cannot type into {reference}: it does not take text — it is a button or a display \
             element, not an input. If it opens a field (a search icon, a combobox), click it \
             first, then type; otherwise target a textbox, searchbox, or combobox by @ref."
        )));
    }
    // A date, time, colour or range field takes no keystrokes (a colour
    // click opens a native picker the renderer waits on), so the value is set
    // and announced as a person's choice would be.
    let given =
        session.document.world.call_on(tab, node, SET_VALUE, vec![Value::String(text.to_owned())]).await?;
    match given.as_str() {
        None => {}
        Some("") => return Ok(reads(session, tab, node, text).await),
        Some(wants) => {
            return Err(Error::Tool(format!("cannot put {text:?} into {reference}: it takes {wants}")));
        }
    }
    // Focus by clicking, as a person does.
    let (x, y) = aim(session, tab, node)
        .await
        .map_err(|error| Error::Tool(format!("cannot type into {reference}: {error}")))?;
    session.hand.click(tab, x, y).await?;
    // Clear what is there before typing — nothing when it is empty, where
    // Backspace takes a multi-select's last choice away. A contenteditable is
    // cleared with a script: selecting it and pressing Backspace hangs the key
    // event in headless Chrome, and typing over a selection appends rather
    // than replaces there. An input keeps the keystroke clear, which a
    // framework controlling its value sees, where setting .value by script
    // would not.
    let held = session.document.world.call_on(tab, node, FIELD_VALUE, Vec::new()).await?;
    let empty = held.as_str().is_none_or(str::is_empty);
    if !empty && session.document.world.call(tab, CLEAR_EDITABLE, Vec::new()).await? != Value::Bool(true) {
        crate::input::keys::press(
            tab,
            &mut session.hand,
            if cfg!(target_os = "macos") { "Meta+a" } else { "Control+a" },
        )
        .await?;
        crate::input::keys::press(tab, &mut session.hand, "Backspace").await?;
    }
    session.hand.type_text(tab, text).await?;
    Ok(reads(session, tab, node, text).await)
}

/// The field's value when it differs from `text`.
async fn reads(session: &mut Session, tab: &Tab, node: i64, text: &str) -> Option<String> {
    let value = session.document.world.call_on(tab, node, FIELD_VALUE, Vec::new()).await;
    let now = value.ok()?.as_str()?.trim_end_matches('\n').to_owned();
    (now != text).then(|| crate::text::clip(&now, READING_CHARS))
}

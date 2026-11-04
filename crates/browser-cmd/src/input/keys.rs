//! The keys: what a character or a named key is to Chrome, and pressing one.

use serde_json::{Value, json};

use crate::chrome::tab::Tab;
use crate::input::hand::Hand;
use crate::{Error, Result};

/// The modifier bits of a CDP key event.
pub(crate) const ALT: i64 = 1;
pub(crate) const CONTROL: i64 = 2;
pub(crate) const META: i64 = 4;
pub(crate) const SHIFT: i64 = 8;

pub(crate) fn key_event(kind: &str, modifiers: i64, key: &str, code: &str, virtual_key: i64) -> Value {
    json!({
        "type": kind, "modifiers": modifiers, "key": key, "code": code,
        "windowsVirtualKeyCode": virtual_key, "nativeVirtualKeyCode": virtual_key,
    })
}

/// The key a printable ASCII character sits on: its name, its physical code,
/// its virtual key, and whether shift is held.
pub(crate) fn key_for_char(character: char) -> Option<(String, String, i64, bool)> {
    let key = character.to_string();
    if character.is_ascii_alphabetic() {
        let upper = character.to_ascii_uppercase();
        return Some((key, format!("Key{upper}"), upper as i64, character.is_ascii_uppercase()));
    }
    if character.is_ascii_digit() {
        return Some((key, format!("Digit{character}"), character as i64, false));
    }
    if character == ' ' {
        return Some((key, "Space".into(), 32, false));
    }
    let (code, virtual_key, shift) = match character {
        '.' => ("Period", 190, false),
        ',' => ("Comma", 188, false),
        '-' => ("Minus", 189, false),
        '_' => ("Minus", 189, true),
        '/' => ("Slash", 191, false),
        '?' => ("Slash", 191, true),
        ':' => ("Semicolon", 186, true),
        ';' => ("Semicolon", 186, false),
        '@' => ("Digit2", 50, true),
        '!' => ("Digit1", 49, true),
        '=' => ("Equal", 187, false),
        '+' => ("Equal", 187, true),
        '\'' => ("Quote", 222, false),
        '"' => ("Quote", 222, true),
        _ => return None,
    };
    Some((key, code.into(), virtual_key, shift))
}

/// Press a named key or chord such as `Enter` or `Control+a`.
pub(crate) async fn press(tab: &Tab, hand: &mut Hand, requested: &str) -> Result<()> {
    let (modifiers, key_name) = parse_key(requested)?;
    let (key, code, virtual_key, text, shift) = key_definition(&key_name)?;
    let kind = if text.is_some() && modifiers == 0 { "keyDown" } else { "rawKeyDown" };
    let held = if shift { modifiers | SHIFT } else { modifiers };
    let mut down = key_event(kind, held, &key, &code, virtual_key);
    if let Some(text) = text {
        down["text"] = Value::String(text);
    }
    // A shortcut reaches an editor as the editing command it stands for; the
    // chord alone is a keystroke Chrome may not map, and a headless one on
    // macOS does not. This is what a real keyboard does through the OS.
    let meta_or_control = modifiers & (CONTROL | META) != 0 && modifiers & (ALT | SHIFT) == 0;
    if meta_or_control {
        let command = match key_name.to_ascii_lowercase().as_str() {
            "a" => Some("selectAll"),
            "c" => Some("copy"),
            "v" => Some("paste"),
            "x" => Some("cut"),
            "z" => Some("undo"),
            _ => None,
        };
        if let Some(command) = command {
            down["commands"] = json!([command]);
        }
    }
    tab.call("Input.dispatchKeyEvent", down).await?;
    hand.hold_key().await;
    tab.call("Input.dispatchKeyEvent", key_event("keyUp", held, &key, &code, virtual_key)).await?;
    Ok(())
}

pub(crate) fn parse_key(requested: &str) -> Result<(i64, String)> {
    let mut modifiers = 0;
    let mut key = None;
    for part in requested.split('+').map(str::trim).filter(|part| !part.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "alt" | "option" => modifiers |= ALT,
            "control" | "ctrl" => modifiers |= CONTROL,
            "meta" | "command" | "cmd" => modifiers |= META,
            "shift" => modifiers |= SHIFT,
            _ if key.is_none() => key = Some(part.to_owned()),
            _ => return Err(Error::Tool(format!("browser key chord has more than one key: {requested}"))),
        }
    }
    let key = key.ok_or_else(|| Error::Tool("press needs a key".into()))?;
    Ok((modifiers, key))
}

/// A named key's name, code, virtual key and text, and whether shift is held for it.
fn key_definition(key: &str) -> Result<(String, String, i64, Option<String>, bool)> {
    let mut characters = key.chars();
    if let (Some(character), None) = (characters.next(), characters.next())
        && let Some((key, code, virtual_key, shift)) = key_for_char(character.to_ascii_lowercase())
    {
        return Ok((key.clone(), code, virtual_key, Some(key), shift));
    }
    let (key, code, virtual_key, text) = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => ("Enter".into(), "Enter".into(), 13, Some("\r".into())),
        "tab" => ("Tab".into(), "Tab".into(), 9, Some("\t".into())),
        "escape" | "esc" => ("Escape".into(), "Escape".into(), 27, None),
        "backspace" => ("Backspace".into(), "Backspace".into(), 8, None),
        "delete" => ("Delete".into(), "Delete".into(), 46, None),
        "arrowup" | "up" => ("ArrowUp".into(), "ArrowUp".into(), 38, None),
        "arrowdown" | "down" => ("ArrowDown".into(), "ArrowDown".into(), 40, None),
        "arrowleft" | "left" => ("ArrowLeft".into(), "ArrowLeft".into(), 37, None),
        "arrowright" | "right" => ("ArrowRight".into(), "ArrowRight".into(), 39, None),
        "home" => ("Home".into(), "Home".into(), 36, None),
        "end" => ("End".into(), "End".into(), 35, None),
        "pageup" => ("PageUp".into(), "PageUp".into(), 33, None),
        "pagedown" => ("PageDown".into(), "PageDown".into(), 34, None),
        "space" => (" ".into(), "Space".into(), 32, Some(" ".into())),
        _ => {
            return Err(Error::Tool(format!(
                "unsupported browser key {key:?}. Keys are Enter, Escape, Tab, Backspace, Delete, the arrows \
                 (ArrowUp/Down/Left/Right), Home, End, PageUp, PageDown, Space, or a single character like \
                 \"a\" or \"/\". To type text use the type tool, not press."
            )));
        }
    };
    Ok((key, code, virtual_key, text, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_chords_are_named_the_way_chrome_names_them() {
        assert_eq!(parse_key("Control+a").unwrap(), (2, "a".into()));
        assert!(parse_key("Control+a+b").is_err());
        assert_eq!(key_for_char('A').unwrap(), ("A".into(), "KeyA".into(), 65, true));
        assert_eq!(key_for_char('7').unwrap(), ("7".into(), "Digit7".into(), 55, false));
        assert_eq!(key_for_char('@').unwrap().1, "Digit2");
        assert!(key_for_char('é').is_none(), "an accented letter comes through the input method");
        assert_eq!(key_definition("A").unwrap(), ("a".into(), "KeyA".into(), 65, Some("a".into()), false));
        assert_eq!(key_definition("?").unwrap(), ("?".into(), "Slash".into(), 191, Some("?".into()), true));
        assert_eq!(key_definition("/").unwrap().1, "Slash");
        assert_eq!(key_definition(".").unwrap().1, "Period");
        assert!(key_definition("~").is_err());
    }
}

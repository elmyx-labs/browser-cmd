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
    // Native key codes use a platform-specific namespace; Windows VK values
    // must not be forwarded as macOS/Linux scan codes. Renderer input uses
    // the explicit Windows VK, DOM key and physical code, as CDP supports.
    json!({
        "type": kind, "modifiers": modifiers, "key": key, "code": code,
        "windowsVirtualKeyCode": virtual_key,
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
        '>' => ("Period", 190, true),
        ',' => ("Comma", 188, false),
        '<' => ("Comma", 188, true),
        '-' => ("Minus", 189, false),
        '_' => ("Minus", 189, true),
        '/' => ("Slash", 191, false),
        '?' => ("Slash", 191, true),
        ':' => ("Semicolon", 186, true),
        ';' => ("Semicolon", 186, false),
        '@' => ("Digit2", 50, true),
        '#' => ("Digit3", 51, true),
        '$' => ("Digit4", 52, true),
        '%' => ("Digit5", 53, true),
        '^' => ("Digit6", 54, true),
        '&' => ("Digit7", 55, true),
        '*' => ("Digit8", 56, true),
        '(' => ("Digit9", 57, true),
        ')' => ("Digit0", 48, true),
        '[' => ("BracketLeft", 219, false),
        '{' => ("BracketLeft", 219, true),
        ']' => ("BracketRight", 221, false),
        '}' => ("BracketRight", 221, true),
        '\\' => ("Backslash", 220, false),
        '|' => ("Backslash", 220, true),
        '`' => ("Backquote", 192, false),
        '~' => ("Backquote", 192, true),
        '!' => ("Digit1", 49, true),
        '=' => ("Equal", 187, false),
        '+' => ("Equal", 187, true),
        '\'' => ("Quote", 222, false),
        '"' => ("Quote", 222, true),
        _ => return None,
    };
    Some((key, code.into(), virtual_key, shift))
}

/// US keyboard text with Shift held; characters already shifted stay so.
fn shifted_char(character: char) -> char {
    if character.is_ascii_alphabetic() {
        return character.to_ascii_uppercase();
    }
    "`1234567890-=[]\\;',./"
        .chars()
        .zip("~!@#$%^&*()_+{}|:\"<>?".chars())
        .find_map(|(plain, shifted)| (plain == character).then_some(shifted))
        .unwrap_or(character)
}

/// Press a named key or chord such as `Enter` or `Control+a`.
pub(crate) async fn press(tab: &Tab, hand: &mut Hand, requested: &str) -> Result<()> {
    let (modifiers, key_name) = parse_key(requested)?;
    let (mut key, code, virtual_key, mut text, shift) = key_definition(&key_name)?;
    if modifiers & SHIFT != 0 && key.len() == 1 {
        key = shifted_char(key.chars().next().unwrap()).to_string();
        text = Some(key.clone());
    }
    let kind = if text.is_some() && modifiers & !SHIFT == 0 { "keyDown" } else { "rawKeyDown" };
    let held = if shift { modifiers | SHIFT } else { modifiers };
    let mut down = key_event(kind, held, &key, &code, virtual_key);
    if kind == "keyDown"
        && let Some(text) = text
    {
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
    dispatch_keystroke(tab, hand, down).await
}

/// Dispatch modifier transitions as well as the key itself. Release any keys
/// already pressed even if a later CDP call fails, so a failed chord does not
/// leave subsequent input with a stuck modifier.
pub(crate) async fn dispatch_keystroke(tab: &Tab, hand: &mut Hand, down: Value) -> Result<()> {
    let modifiers = down["modifiers"].as_i64().unwrap_or(0);
    let mut held = 0;
    let mut pressed = Vec::new();
    let result: Result<()> = async {
        for (bit, key, code, virtual_key) in [
            (CONTROL, "Control", "ControlLeft", 17),
            (ALT, "Alt", "AltLeft", 18),
            (META, "Meta", "MetaLeft", 91),
            (SHIFT, "Shift", "ShiftLeft", 16),
        ] {
            if modifiers & bit != 0 {
                held |= bit;
                // Remember before awaiting: a transport error may happen after
                // Chrome has received the key-down.
                pressed.push((bit, key, code, virtual_key));
                tab.call("Input.dispatchKeyEvent", key_event("rawKeyDown", held, key, code, virtual_key))
                    .await?;
            }
        }
        let mut up = down.clone();
        up["type"] = "keyUp".into();
        for field in ["text", "unmodifiedText", "commands"] {
            up.as_object_mut().unwrap().remove(field);
        }
        let key_result = tab.call("Input.dispatchKeyEvent", down).await;
        hand.hold_key().await;
        let released = tab.call("Input.dispatchKeyEvent", up).await;
        key_result?;
        released?;
        Ok(())
    }
    .await;
    let mut release_error = None;
    for (bit, key, code, virtual_key) in pressed.into_iter().rev() {
        held &= !bit;
        if let Err(error) =
            tab.call("Input.dispatchKeyEvent", key_event("keyUp", held, key, code, virtual_key)).await
        {
            release_error.get_or_insert(error);
        }
    }
    result?;
    match release_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub(crate) fn parse_key(requested: &str) -> Result<(i64, String)> {
    let mut modifiers = 0;
    let mut key = requested.ends_with('+').then(|| "+".to_owned());
    let requested_parts = if key.is_some() { &requested[..requested.len() - 1] } else { requested };
    for part in requested_parts.split('+').map(str::trim).filter(|part| !part.is_empty()) {
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
    fn renderer_keys_do_not_invent_platform_native_codes() {
        let event = key_event("rawKeyDown", SHIFT, "Shift", "ShiftLeft", 16);
        assert!(event.get("nativeVirtualKeyCode").is_none());
        assert_eq!(event["windowsVirtualKeyCode"], 16);
        assert_eq!(event["key"], "Shift");
        assert_eq!(event["code"], "ShiftLeft");
    }

    #[test]
    fn every_printable_us_key_has_a_physical_mapping() {
        for character in ' '..='~' {
            assert!(key_for_char(character).is_some(), "{character:?}");
        }
        for character in "~!@#$%^&*()_+{}|:\"<>?".chars() {
            let (_, code, virtual_key, shift) = key_for_char(character).unwrap();
            assert!(shift, "{character}");
            let plain = (' '..='~')
                .find(|candidate| {
                    key_for_char(*candidate).is_some_and(|(_, c, v, s)| !s && c == code && v == virtual_key)
                })
                .unwrap();
            assert_eq!(shifted_char(plain), character);
        }
    }

    #[test]
    fn keys_and_chords_are_named_the_way_chrome_names_them() {
        assert_eq!(parse_key("Control+a").unwrap(), (2, "a".into()));
        assert_eq!(parse_key("+").unwrap(), (0, "+".into()));
        assert_eq!(parse_key("Control++").unwrap(), (CONTROL, "+".into()));
        assert!(parse_key("Control+a+b").is_err());
        assert_eq!(key_for_char('A').unwrap(), ("A".into(), "KeyA".into(), 65, true));
        assert_eq!(key_for_char('7').unwrap(), ("7".into(), "Digit7".into(), 55, false));
        assert_eq!(key_for_char('@').unwrap().1, "Digit2");
        assert!(key_for_char('é').is_none(), "an accented letter comes through the input method");
        assert_eq!(key_definition("A").unwrap(), ("a".into(), "KeyA".into(), 65, Some("a".into()), false));
        assert_eq!(key_definition("?").unwrap(), ("?".into(), "Slash".into(), 191, Some("?".into()), true));
        assert_eq!(key_definition("/").unwrap().1, "Slash");
        assert_eq!(key_definition(".").unwrap().1, "Period");
        assert_eq!(key_definition("~").unwrap().1, "Backquote");
    }
}

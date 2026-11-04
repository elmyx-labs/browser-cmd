//! The environment: `BROWSER_CMD_*` variables, read strictly — a value that does
//! not parse is a configuration error, never a silent default — the config
//! files behind them, and whether a window can be shown.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use crate::{Error, Result};

/// The variable's value, when it is set to something other than blanks; else
/// the config files' value under the same name, lowercased.
pub(crate) fn var(name: &str) -> Option<String> {
    std::env::var(format!("BROWSER_CMD_{name}"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| config().as_ref().ok()?.get(name).cloned())
}

pub(crate) fn flag(name: &str) -> Result<Option<bool>> {
    var(name)
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(Error::Config(format!("BROWSER_CMD_{name}={value:?} is not a yes or a no"))),
        })
        .transpose()
}

pub(crate) fn parse<T: std::str::FromStr>(name: &str) -> Result<Option<T>> {
    var(name)
        .map(|value| {
            value.parse().map_err(|_| Error::Config(format!("BROWSER_CMD_{name}={value:?} does not parse")))
        })
        .transpose()
}

/// Positive finite seconds that fit the clock.
pub(crate) fn seconds(name: &str) -> Result<Option<Duration>> {
    parse::<f64>(name)?
        .map(|value| {
            Duration::try_from_secs_f64(value)
                .ok()
                .filter(|duration| {
                    !duration.is_zero() && std::time::Instant::now().checked_add(*duration).is_some()
                })
                .ok_or_else(|| Error::Config(format!("BROWSER_CMD_{name} must be positive finite seconds")))
        })
        .transpose()
}

/// A malformed config file, reported at startup so that no command runs on a
/// silent default.
pub(crate) fn check() -> Result<()> {
    config().as_ref().map(|_| ()).map_err(|error| Error::Config(error.clone()))
}

/// The config files' values by variable name, read once: `~/.browser-cmd/config.toml`,
/// then the workspace's `.browser-cmd/config.toml` over it.
fn config() -> &'static std::result::Result<HashMap<String, String>, String> {
    static CONFIG: OnceLock<std::result::Result<HashMap<String, String>, String>> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let home = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".browser-cmd"));
        let workspace = std::env::current_dir().ok().map(|dir| repository(&dir).join(".browser-cmd"));
        let mut values = HashMap::new();
        for path in home.into_iter().chain(workspace).map(|dir| dir.join("config.toml")) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                values.extend(parse_config(&text).map_err(|line| format!("{}: {line}", path.display()))?);
            }
        }
        Ok(values)
    })
}

/// The git work tree around a directory, else the directory itself.
pub(crate) fn repository(dir: &Path) -> PathBuf {
    dir.ancestors().find(|at| at.join(".git").exists()).unwrap_or(dir).to_owned()
}

/// `key = value` lines, `#` comments, a value quoted or bare: the TOML a
/// settings file needs, and nothing a table or an array would add.
fn parse_config(text: &str) -> std::result::Result<Vec<(String, String)>, String> {
    let mut values = Vec::new();
    for (at, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let malformed = || format!("line {}: expected key = \"value\"", at + 1);
        let (key, value) = line.split_once('=').ok_or_else(malformed)?;
        let key = key.trim();
        if key.is_empty()
            || !key.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(malformed());
        }
        let value = match value.trim().strip_prefix('"') {
            Some(quoted) => quoted.split_once('"').map(|(value, _)| value).ok_or_else(malformed)?,
            None => value.split('#').next().unwrap_or_default().trim(),
        };
        values.push((key.to_ascii_uppercase(), value.to_owned()));
    }
    Ok(values)
}

/// Whether a window can be shown. A real window is the strongest single
/// signal of a real user, and on a desktop it costs nothing.
pub(crate) fn display_available() -> bool {
    if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
        return true;
    }
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_file_is_flat_keys_with_quoted_or_bare_values_and_comments() {
        let parsed = parse_config(
            "# the browser\nchrome = \"mine\" # a comment\nheadless = true\npace = 0.5\n\nwindow=\"1440,900\"\n",
        )
        .unwrap();
        let expected = [("CHROME", "mine"), ("HEADLESS", "true"), ("PACE", "0.5"), ("WINDOW", "1440,900")];
        assert_eq!(parsed, expected.map(|(key, value)| (key.to_owned(), value.to_owned())));
        for malformed in ["[browser]", "chrome", "Chrome = \"mine\"", "chrome = \"mine", "= 1"] {
            assert!(parse_config(malformed).is_err(), "{malformed}");
        }
    }
}

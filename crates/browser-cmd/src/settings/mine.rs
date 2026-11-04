//! The Chrome a person uses every day. Chrome 144 opens a DevTools port for
//! its default profile once Remote Debugging is turned on in `chrome://inspect`,
//! notes the port in that profile, and asks the person before letting a
//! connection in.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Its DevTools WebSocket endpoint, from the port and path noted in the
/// default profile: in this mode Chrome serves no `/json` discovery, only that path.
pub(crate) fn endpoint() -> Result<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let profile = if cfg!(target_os = "macos") {
        home.map(|home| home.join("Library/Application Support/Google/Chrome"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(|local| PathBuf::from(local).join("Google/Chrome/User Data"))
    } else {
        home.map(|home| home.join(".config/google-chrome"))
    };
    noted_in(&profile.ok_or_else(|| Error::Config("cannot tell where your Chrome keeps its profile".into()))?)
}

fn noted_in(profile: &Path) -> Result<String> {
    std::fs::read_to_string(profile.join("DevToolsActivePort"))
        .ok()
        .and_then(|noted| {
            let mut lines = noted.lines();
            let port: u16 = lines.next()?.trim().parse().ok()?;
            let path = lines.next()?.trim().strip_prefix('/')?.to_owned();
            Some(format!("ws://127.0.0.1:{port}/{path}"))
        })
        .ok_or_else(|| {
            Error::Config(
                "your Chrome has no DevTools port open: in it, open chrome://inspect/#remote-debugging and \
                 turn on Remote Debugging (Chrome 144 or later), then run this again"
                    .into(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_persons_chrome_is_found_by_the_port_it_noted_or_the_person_is_told_what_to_turn_on() {
        let temp = tempfile::tempdir().unwrap();
        let told = noted_in(temp.path()).expect_err("no port noted").to_string();
        assert!(told.contains("chrome://inspect/#remote-debugging"), "{told}");
        std::fs::write(temp.path().join("DevToolsActivePort"), "9333\n/devtools/browser/abc\n").unwrap();
        assert_eq!(noted_in(temp.path()).unwrap(), "ws://127.0.0.1:9333/devtools/browser/abc");
    }
}

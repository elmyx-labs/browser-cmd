//! Finding a stable Google Chrome on this machine, and where a running one listens.
//!
//! Chromium and the "for Testing" builds are what automation tools ship, and a
//! site that has seen a thousand of them recognises the next one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::{Error, Result};

/// Where a macOS install keeps the executable inside its bundle, and what
/// Spotlight is asked to find that bundle by.
const BUNDLE: &str = "Google Chrome.app/Contents/MacOS/Google Chrome";
const BUNDLE_QUERY: &str = "kMDItemCFBundleIdentifier == 'com.google.Chrome'";

/// How much of a `/json/version` answer is read, head and body each.
const VERSION_BYTES: usize = 64 * 1024;

/// The usual place first, because it costs no subprocess and does not depend
/// on Spotlight, which a server may have switched off and which answers
/// nothing under load; then anything named chrome on `PATH`; then, on macOS,
/// wherever Spotlight says a bundle is, for an install somewhere unusual.
pub(crate) fn preferred_stable_chrome() -> Option<PathBuf> {
    standard_locations().into_iter().chain(chrome_on_path()).chain(spotlight_bundles()).find(is_stable_chrome)
}

fn standard_locations() -> Vec<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        [Some(PathBuf::from("/Applications")), home().map(|home| home.join("Applications"))]
            .into_iter()
            .flatten()
            .map(|directory| directory.join(BUNDLE))
            .collect()
    } else if cfg!(target_os = "windows") {
        ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|root| PathBuf::from(root).join("Google/Chrome/Application/chrome.exe"))
            .collect()
    } else {
        ["/usr/bin/google-chrome", "/usr/bin/google-chrome-stable", "/opt/google/chrome/chrome"]
            .iter()
            .map(PathBuf::from)
            .collect()
    }
}

fn chrome_on_path() -> Vec<PathBuf> {
    let Some(path) = std::env::var_os("PATH") else { return Vec::new() };
    std::env::split_paths(&path)
        .filter_map(|directory| std::fs::read_dir(directory).ok())
        .flatten()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
            (name.contains("chrome") && !name.contains("driver")).then_some(path)
        })
        .collect()
}

/// The Chrome bundles Spotlight knows about, for an install in neither the
/// standard place nor on `PATH`. Empty wherever Spotlight is not the index.
fn spotlight_bundles() -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let Some(found) = output(std::process::Command::new("mdfind").arg(BUNDLE_QUERY)) else {
        return Vec::new();
    };
    found
        .lines()
        .filter(|bundle| !bundle.is_empty())
        .filter_map(|bundle| {
            let plist = Path::new(bundle).join("Contents/Info.plist");
            let executable = output(
                std::process::Command::new("plutil")
                    .args(["-extract", "CFBundleExecutable", "raw", "-o", "-"])
                    .arg(&plist),
            )?;
            let executable = executable.trim();
            (!executable.is_empty()).then(|| Path::new(bundle).join("Contents/MacOS").join(executable))
        })
        .collect()
}

/// On Windows `chrome.exe --version` opens a browser instead of printing, so the
/// channel is read from the install path: Beta, Dev and Canary live in their own
/// `Chrome Beta`, `Chrome Dev` and `Chrome SxS` folders.
fn is_stable_chrome(candidate: &PathBuf) -> bool {
    if cfg!(target_os = "windows") {
        return candidate.is_file() && candidate.ends_with("Google/Chrome/Application/chrome.exe");
    }
    candidate.is_file()
        && output(std::process::Command::new(candidate).arg("--version"))
            .map(|version| version.to_ascii_lowercase())
            .is_some_and(|version| {
                version.contains("google chrome")
                    && !version.contains("beta")
                    && !version.contains("dev")
                    && !version.contains("canary")
            })
}

fn output(command: &mut std::process::Command) -> Option<String> {
    let output = command.output().ok().filter(|output| output.status.success())?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The WebSocket endpoint of a Chrome running on this profile, from the file
/// Chrome keeps it in; stale after a Chrome that was killed, which the
/// connection then finds out.
pub(crate) async fn active_port(profile: &Path, patience: Duration) -> Option<String> {
    let noted = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    let port: u16 = noted.lines().next()?.trim().parse().ok()?;
    debugger_url(&format!("http://127.0.0.1:{port}"), patience).await.ok()
}

/// Chrome's WebSocket endpoint, asked of its HTTP one. Chrome answers and
/// keeps the socket open whatever the request says about closing it, so the
/// answer is read to its Content-Length and not to an end that never comes.
pub(crate) async fn debugger_url(endpoint: &str, patience: Duration) -> Result<String> {
    let url =
        url::Url::parse(endpoint).map_err(|error| Error::Config(format!("BROWSER_CMD_CDP: {error}")))?;
    let host = url.host_str().unwrap_or("127.0.0.1");
    let port = url.port_or_known_default().unwrap_or(9222);
    let mut stream = tokio::net::TcpStream::connect((host, port))
        .await
        .map_err(|error| Error::Tool(format!("no Chrome is listening at {endpoint}: {error}")))?;
    stream
        .write_all(
            format!("GET /json/version HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await?;
    let response =
        tokio::time::timeout(patience, crate::http::read(&mut stream, VERSION_BYTES, VERSION_BYTES))
            .await
            .map_err(|_| Error::Tool(format!("{endpoint}/json/version did not answer in time")))?;
    let version: Value = match response {
        Err(Error::Io(error)) => return Err(Error::Io(error)),
        response => response.ok().and_then(|message| serde_json::from_slice(&message.body).ok()),
    }
    .ok_or_else(|| Error::Tool(format!("{endpoint}/json/version did not answer with JSON")))?;
    version["webSocketDebuggerUrl"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Tool(format!("{endpoint}/json/version names no webSocketDebuggerUrl")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_google_chrome_is_preferred_when_it_is_installed() {
        if let Some(chrome) = preferred_stable_chrome() {
            assert!(is_stable_chrome(&chrome));
        }
    }
}

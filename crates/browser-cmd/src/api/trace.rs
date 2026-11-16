//! The trace a failed command leaves in `.browser-cmd/traces/`: the command,
//! the error and the browser log, so the step can be understood without
//! running it again. Best effort throughout.

use std::path::Path;

use crate::api::client::Daemon;
use serde_json::json;

/// Traces kept in one directory; the oldest make way.
const KEPT: usize = 50;

/// The command line as a trace records it: what `type` typed may be a password,
/// and is not written to disk.
fn recorded(argv: &[String]) -> String {
    match argv {
        [name, typed @ ..] if name == "type" => {
            let ours =
                |word: &&String| word.starts_with('@') || matches!(word.as_str(), "submit" | "--brief");
            let kept: Vec<&str> = typed.iter().filter(ours).map(String::as_str).collect();
            format!("type {} (the typed text is not recorded)", kept.join(" "))
        }
        _ => argv.join(" "),
    }
}

#[cfg(unix)]
fn write(path: &Path, body: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(body.as_bytes())
}

#[cfg(not(unix))]
fn write(path: &Path, body: &str) -> std::io::Result<()> {
    std::fs::write(path, body)
}

/// Their names are the moment each was written, so name order is age order.
fn prune(dir: &Path) {
    let mut traces: Vec<_> =
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|entry| entry.path()).collect();
    traces.sort();
    for old in traces.iter().rev().skip(KEPT) {
        let _ = std::fs::remove_file(old);
    }
}

/// `answered`: the daemon answered the command, so it can be asked for the
/// log; one that did not is busy or gone, and asking would wait for it again.
pub(crate) async fn trace(daemon: &Daemon, argv: &[String], error: &str, answered: bool) {
    if crate::env::flag("NO_TRACE").ok().flatten() == Some(true) {
        return;
    }
    let logs = match answered {
        true => daemon.call("console", json!({})).await.ok().map(|reply| reply.output),
        false => None,
    };
    let when = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let body = format!(
        "command: {}\nerror: {error}\n\nbrowser log:\n{}\n",
        recorded(argv),
        logs.as_deref().unwrap_or("(none)")
    );
    let dir = Path::new(".browser-cmd").join("traces");
    if std::fs::create_dir_all(&dir).is_ok() {
        let path = dir.join(format!("{when}.txt"));
        if write(&path, &body).is_ok() {
            eprintln!("browser-cmd: trace written to {}", path.display());
        }
        prune(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_text_stays_out_of_the_trace_and_the_ref_stays_in() {
        let argv = ["type", "@e3.4", "hunter2", "submit"].map(String::from);
        assert_eq!(recorded(&argv), "type @e3.4 submit (the typed text is not recorded)");
        assert_eq!(recorded(&["click".to_owned(), "Sign in".to_owned()]), "click Sign in");
    }

    #[test]
    fn the_oldest_traces_make_way() {
        let dir = tempfile::tempdir().unwrap();
        for moment in 0..KEPT + 3 {
            std::fs::write(dir.path().join(format!("{:013}.txt", moment)), "").unwrap();
        }
        prune(dir.path());
        let mut left: Vec<_> =
            std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        left.sort();
        assert_eq!(left.len(), KEPT);
        assert_eq!(left[0], "0000000000003.txt");
    }
}

//! The shell CLI drives a warm daemon, and a ref taken by one command process
//! resolves in the next.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

mod support;
use support::stop_owned_daemon;

#[test]
fn command_help_does_not_connect_to_or_start_a_browser() {
    let dir = tempfile::tempdir().unwrap();
    // An output flag may come before the command as well as after it.
    for args in [vec!["text", "--help"], vec!["--brief", "text", "--help"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_browser-cmd"))
            .args(&args)
            .current_dir(dir.path())
            .env("BROWSER_CMD_ADDR", "invalid-endpoint")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.starts_with("text ") && !text.contains("Commands."),
            "expected command-specific help: {text}"
        );
        assert!(text.contains("max_bytes=N") && text.contains("context=false"), "{text}");
    }
    assert!(!dir.path().join(".browser-cmd").exists());
}

#[test]
fn version_prints_the_crate_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_browser-cmd")).arg("--version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("browser-cmd {}\n", env!("CARGO_PKG_VERSION"))
    );
}

/// docs/commands.md carries every command's syntax as its own --help gives it.
#[test]
fn the_command_reference_matches_the_help() {
    let reference =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/commands.md")).unwrap();
    let catalogue = Command::new(env!("CARGO_BIN_EXE_browser-cmd")).arg("--help").output().unwrap().stdout;
    for line in String::from_utf8(catalogue)
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
    {
        let name = line.split_whitespace().next().unwrap();
        if name == "browser-cmd" {
            continue;
        }
        let help =
            Command::new(env!("CARGO_BIN_EXE_browser-cmd")).args([name, "--help"]).output().unwrap().stdout;
        let usage = String::from_utf8(help).unwrap().lines().next().unwrap().trim_end().to_owned();
        assert!(reference.contains(&format!("browser-cmd {usage}\n")), "docs/commands.md lacks: {usage}");
    }
}

/// A session whose browser belongs to someone else neither starts a daemon on
/// its own when that browser is unreachable nor serves one when asked.
#[test]
fn no_spawn_refuses_to_start_a_daemon_or_a_browser() {
    let dir = tempfile::tempdir().unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let addr = format!("127.0.0.1:{port}");
    for args in [vec!["navigate", "about:blank"], vec!["--serve", addr.as_str()]] {
        let output = Command::new(env!("CARGO_BIN_EXE_browser-cmd"))
            .args(&args)
            .current_dir(dir.path())
            .env("HOME", dir.path())
            .env("BROWSER_CMD_ADDR", &addr)
            .env("BROWSER_CMD_NO_SPAWN", "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success() && stderr.contains("BROWSER_CMD_NO_SPAWN"), "{args:?}: {stderr}");
    }
    assert!(TcpListener::bind(&addr).is_ok(), "nothing was left serving");
}

/// A one-page server on an ephemeral port, returning its address.
fn serve(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    address
}

fn cli(dir: &std::path::Path, addr: &str, home: &std::path::Path, args: &[&str]) -> String {
    cli_with(dir, addr, home, args, "").0
}

/// One CLI process given `stdin`, answering with its stdout and stderr.
fn cli_with(
    dir: &std::path::Path,
    addr: &str,
    home: &std::path::Path,
    args: &[&str],
    stdin: &str,
) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_browser-cmd"))
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("BROWSER_CMD_ADDR", addr)
        .env("BROWSER_CMD_HEADLESS", "1")
        .env("BROWSER_CMD_PACE", "0")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the CLI runs");
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
#[ignore = "requires installed Chrome; run with --ignored"]
fn a_ref_from_one_command_process_clicks_in_the_next() {
    let page = serve(
        "<!doctype html><meta charset=utf-8><title>t</title>\
         <a href=\"/next\">Go onward</a>",
    );
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let dir = temp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    // Its own port and profile, so the run neither joins nor disturbs a daemon
    // a developer has open.
    let addr = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().to_string()
    };

    // Three separate processes share one session through the daemon the first
    // one starts.
    let navigated = cli(&dir, &addr, &home, &["navigate", &format!("http://{page}")]);
    assert!(navigated.contains("Navigated"), "navigate: {navigated}");

    let snapshot = cli(&dir, &addr, &home, &["snapshot"]);
    assert!(snapshot.contains("Go onward") && snapshot.contains(" @e"), "snapshot: {snapshot}");

    let (value, _) = cli_with(&dir, &addr, &home, &["eval", "-", "--raw"], "({sum: 1 +\n 1})");
    assert_eq!(value.trim(), r#"{"sum":2}"#, "eval - reads stdin and --raw prints one line");
    let text = cli(&dir, &addr, &home, &["text", "--raw"]);
    assert!(text.starts_with("URL: ") && text.contains("Go onward"), "text --raw: {text}");
    let (_, stderr) = cli_with(&dir, &addr, &home, &["click", "No such control"], "");
    assert!(stderr.contains("browser-cmd: Error: ") && stderr.contains("No such control"), "{stderr}");

    let state = cli(&dir, &addr, &home, &["snapshot"]);
    let reference = state
        .lines()
        .find(|line| line.contains("Go onward"))
        .and_then(|line| line.split_whitespace().find(|word| word.starts_with("@e")))
        .unwrap_or_else(|| panic!("no ref for Go onward: {state}"));
    let clicked = cli(&dir, &addr, &home, &["click", reference, "--brief"]);
    assert!(
        clicked.starts_with("click: Clicked")
            && clicked.to_lowercase().contains("| url: http")
            && clicked.contains("/next"),
        "a ref from a prior process did not click: {clicked}"
    );
    assert_eq!(clicked.trim().lines().count(), 1, "--brief is one line: {clicked}");

    // Several steps in one call: one line each, the last one whole.
    let (batched, _) =
        cli_with(&dir, &addr, &home, &["batch"], &format!("navigate http://{page}\nback\nsnapshot\n"));
    assert!(batched.starts_with("navigate: Navigated."), "batch: {batched}");
    assert!(batched.contains("Go onward") && batched.contains(" @e"), "the last step is whole: {batched}");
    let (_, failed) = cli_with(&dir, &addr, &home, &["batch"], "click \"No such control\"\nsnapshot\n");
    assert!(failed.contains("No such control"), "the first failure stops the batch: {failed}");

    // Stop this run's daemon by verifying its full command line and PID.
    let _ = cli(&dir, &addr, &home, &["close"]);
    stop_owned_daemon(&addr);
}

//! Actual daemon/client processes on local sockets. No Chrome or external API.

use std::{
    io::Read,
    net::TcpListener,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

fn command(workspace: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_browser-cmd"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("BROWSER_CMD_") {
            command.env_remove(key);
        }
    }
    command.current_dir(workspace);
    command
}

fn bounded_output(command: &mut Command) -> Output {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("test process exceeded deadline: {}", String::from_utf8_lossy(&output.stderr));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn rpc_deadline_does_not_replay_an_unconfirmed_action() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let server = std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            let mut data = [0_u8; 4096];
            if stream.read(&mut data).unwrap_or(0) == 0 {
                continue;
            } // liveness probe
            count.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(250));
            break;
        }
    });
    let temp = tempfile::tempdir().unwrap();
    let mut command = command(temp.path());
    command
        .args(["navigate", "https://example.org/"])
        .env("BROWSER_CMD_ADDR", address.to_string())
        .env("BROWSER_CMD_RPC_SECS", "0.05")
        .env("BROWSER_CMD_NO_TRACE", "1");
    let output = bounded_output(&mut command);
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("may still be running") && error.contains("timed out"), "{error}");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[test]
fn invalid_configuration_returns_errors_without_panics_or_browser_startup() {
    let temp = tempfile::tempdir().unwrap();
    for (key, value) in [
        ("BROWSER_CMD_RPC_SECS", "-1"),
        ("BROWSER_CMD_PACE", "inf"),
        ("BROWSER_CMD_HEADLESS", "maybe"),
        ("BROWSER_CMD_WINDOW", "0,900"),
        ("BROWSER_CMD_DIALOGS", "sometimes"),
    ] {
        let mut command = command(temp.path());
        let output =
            bounded_output(command.args(["--serve", "127.0.0.1:0"]).env(key, value).stdin(Stdio::null()));
        assert_eq!(output.status.code(), Some(1), "{key}={value}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("configuration error") && !error.contains("panicked"), "{error}");
    }
    let mut command = command(temp.path());
    let output = bounded_output(command.args(["--serve", "0.0.0.0:0"]).stdin(Stdio::null()));
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires BROWSER_CMD_RPC_TOKEN"));
}

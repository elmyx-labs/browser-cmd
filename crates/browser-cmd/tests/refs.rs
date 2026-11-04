//! Ref identity across independently started daemons: a replacement never
//! honours its predecessor's refs, even when a page wears one as its label.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

mod support;
use support::stop_owned_daemon;

/// One page with a button that counts the clicks it receives.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}/", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.read(&mut [0_u8; 4096]);
            let body = "<!doctype html><meta charset=utf-8><button onclick='document.body.dataset.clicks\
                        =Number(document.body.dataset.clicks||0)+1'>Press</button>";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    address
}

/// One command, in a process of its own, against the daemon at `address`.
struct Shell {
    directory: std::path::PathBuf,
    home: std::path::PathBuf,
    address: String,
}

impl Shell {
    /// Its output, whatever it said on the way out, and whether it succeeded.
    fn run(&self, args: &[&str]) -> (String, String, bool) {
        let output = Command::new(env!("CARGO_BIN_EXE_browser-cmd"))
            .args(args)
            .current_dir(&self.directory)
            .env("HOME", &self.home)
            .env("BROWSER_CMD_ADDR", &self.address)
            .env("BROWSER_CMD_HEADLESS", "1")
            .env("BROWSER_CMD_PACE", "0")
            .env("BROWSER_CMD_NO_TRACE", "1")
            .output()
            .expect("the CLI runs");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.success(),
        )
    }

    fn ok(&self, args: &[&str]) -> String {
        let (out, said, ok) = self.run(args);
        assert!(ok, "{args:?} failed: {said}{out}");
        out
    }

    /// The same, for a command that is expected to be refused.
    fn refused(&self, args: &[&str]) -> String {
        let (out, said, ok) = self.run(args);
        assert!(!ok, "{args:?} was expected to fail: {out}");
        format!("{said}{out}")
    }

    /// The ref the page's only button has right now.
    fn button(&self, url: &str) -> String {
        self.ok(&["navigate", url]);
        let state = self.ok(&["snapshot"]);
        state
            .lines()
            .find(|line| line.contains("Press"))
            .and_then(|line| line.split_whitespace().find(|word| word.starts_with("@e")))
            .expect("the button is in the state")
            .to_owned()
    }

    fn clicks(&self) -> String {
        self.ok(&["eval", "Number(document.body.dataset.clicks||0)"])
    }

    /// Stop the daemon this run started, by the address on its command line.
    fn stop(&self) {
        let _ = self.run(&["close"]);
        stop_owned_daemon(&self.address);
    }
}

#[test]
#[ignore = "requires installed Chrome; run with --ignored"]
fn a_previous_daemons_ref_cannot_act_in_a_replacement_even_after_a_fresh_snapshot() {
    let url = serve();
    let temp = tempfile::tempdir().unwrap();
    let (home, directory) = (temp.path().join("home"), temp.path().join("work"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&directory).unwrap();
    // Its own port and profile, so the run neither joins nor disturbs a daemon
    // a developer has open.
    let address = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().to_string();
    let shell = Shell { directory, home, address };

    let old = shell.button(&url);
    shell.ok(&["click", &old]);
    shell.stop();

    let fresh = shell.button(&url);
    assert_ne!(old, fresh, "a replacement daemon hands out names of its own");

    shell.refused(&["click", &old]);

    // An unknown handle must not fall back to a coincidentally matching label:
    // the page is made to call its button by the old ref's exact text.
    shell.ok(&["eval", &format!("document.querySelector('button').textContent={old:?}")]);
    shell.ok(&["snapshot"]);
    shell.refused(&["click", &old]);
    assert!(shell.clicks().contains(">\n0\n<"), "nothing was clicked: {}", shell.clicks());

    shell.ok(&["click", &fresh]);
    assert!(shell.clicks().contains(">\n1\n<"), "the current ref did not click: {}", shell.clicks());
    shell.stop();
}

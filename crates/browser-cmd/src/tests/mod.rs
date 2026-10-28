//! The test suite: shared fixtures here, then one file per subject. Live
//! Chrome cases are `#[ignore]` so an ordinary run cannot report them as
//! passed without executing them — run with --include-ignored.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::browser::host::{Host, Shared};
use crate::chrome::tab::Tab;
use crate::page::world::World;
use crate::session::Session;
use crate::text::fence::untrusted;
use crate::text::passages::passages;
use crate::text::sentence;
use crate::web::web_url;
use crate::{Error, Outcome, Settings};

/// The catalogue's dispatch, as the daemon does it, for a test to name a command.
trait Execute {
    async fn execute(&mut self, name: &str, args: Value) -> Outcome;
}

impl Execute for Session {
    async fn execute(&mut self, name: &str, args: Value) -> Outcome {
        match crate::api::command::find(name) {
            Some(command) => {
                let acting = command.acts(&args);
                self.perform(command.name, command.changing, acting, command.run, args).await
            }
            None => Outcome::failed(format!("Error: unknown command {name}"), "tool"),
        }
    }
}

/// A local server that answers every request with one page.
fn serve(body: String) -> std::net::SocketAddr {
    serve_type("text/html", body)
}

fn serve_type(content_type: &'static str, body: String) -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            // A connection Chrome opens ahead and leaves unused, for up to its
            // 10 s idle timeout, must not hold up the requests behind it.
            let response = response.clone();
            std::thread::spawn(move || {
                let mut request = [0_u8; 8192];
                let _ = std::io::Read::read(&mut stream, &mut request);
                let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
            });
        }
    });
    address
}

/// A browser for a test: the defaults, never the environment, headless, and
/// without a person's tempo.
fn settings(home: &std::path::Path, workspace: &std::path::Path) -> Settings {
    let mut settings = Settings::new(home, workspace);
    settings.headless = true;
    settings.pace = 0.0;
    settings
}

/// A fresh browser with the test defaults over a temporary home, opened on the
/// page at `address`: `test` runs with the session and what opening the page
/// answered, and the browser is shut down after it.
async fn on_page<T>(address: std::net::SocketAddr, test: impl AsyncFnOnce(&mut Session, Outcome) -> T) -> T {
    on_page_with(address, |_| {}, test).await
}

/// `on_page`, with the defaults `configure`d first.
async fn on_page_with<T>(
    address: std::net::SocketAddr,
    configure: impl FnOnce(&mut Settings),
    test: impl AsyncFnOnce(&mut Session, Outcome) -> T,
) -> T {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path(), temp.path());
    configure(&mut settings);
    let host = Host::new(settings);
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut session = host.session();
            let opened = session.execute("navigate", json!({ "url": format!("http://{address}") })).await;
            assert!(opened.ok, "{}", opened.output);
            let result = test(&mut session, opened).await;
            host.shutdown().await;
            result
        })
        .await
}

/// Whether a test may open a real window: a display exists, and the operator
/// has not asked for headless. A test that covers both modes covers only the
/// allowed one rather than opening a window nobody asked for.
fn headed_allowed() -> bool {
    crate::env::display_available() && crate::env::flag("HEADLESS").ok().flatten() != Some(true)
}

mod arguments;
mod canvas;
mod choose;
mod conditions;
mod context;
mod detection;
mod diagnostics;
mod downloads;
mod evidence;
mod extract;
mod kept;
mod keyboard;
mod lifecycle;
mod network;
mod ownership;
mod pages;
mod pointer;
mod process;
mod proxy;
mod readiness;
mod reading;
mod refs;
mod replies;
mod scroll;
mod stealth;
mod submission;
mod tabs;
mod walls;
mod workflows;

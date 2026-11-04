//! When a page counts as ready: calm against its size, holding something,
//! old enough not to be redirecting, its own requests answered when the
//! Network domain is on, and a cap that is said when it is hit.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_settles_past_a_ticker_and_says_when_the_cap_was_hit() {
    let rows = "<li>row</li>".repeat(2000);
    let address = serve(format!(
        "<!doctype html><title>t</title><p id=clock></p><ul id=list>{rows}</ul>
         <button onclick=\"setInterval(() => {{ for (const row of [...list.children].slice(0, 50)) row.textContent = Math.random() }}, 50)\">Churn</button>
         <script>setInterval(() => clock.textContent = Date.now(), 30)</script>"
    ));
    let configure = |settings: &mut Settings| settings.waits.settle = std::time::Duration::from_secs(3);
    on_page_with(address, configure, async |session, opened| {
        assert!(!opened.output.contains("[warning]"), "a ticker is not a page loading: {}", opened.output);
        let churned = session.execute("click", json!({ "ref": "Churn" })).await;
        assert!(churned.ok, "{}", churned.output);
        assert!(
            churned.output.starts_with("Clicked. [warning] page kept changing; settled after 3.0 s cap"),
            "{}",
            churned.output
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_waits_out_a_client_redirect_and_tells_the_time_once() {
    let address = serve(
        "<!doctype html><title>t</title><p>Somewhere with enough words to read.</p>
         <script>if (location.pathname === '/') setTimeout(() => location.replace('/landed'), 500)</script>"
            .to_owned(),
    );
    on_page(address, async |session, opened| {
        assert!(
            opened.output.contains("URL: http://") && opened.output.contains("/landed"),
            "{}",
            opened.output
        );
        assert_eq!(opened.output.matches("\nLocal time: ").count(), 1, "{}", opened.output);
        let again = session.execute("snapshot", json!({})).await;
        assert!(!again.output.contains("Local time:"), "{}", again.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_waits_for_a_same_site_request_when_network_notes_are_on() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0_u8; 8192];
            let read = std::io::Read::read(&mut stream, &mut request).unwrap_or_default();
            let body = match String::from_utf8_lossy(&request[..read]).starts_with("GET /slow") {
                true => {
                    std::thread::sleep(std::time::Duration::from_millis(900));
                    "arrived".to_owned()
                }
                false => "<!doctype html><title>t</title><p id=out>waiting for the answer</p>
                    <button onclick=\"fetch('/slow').then(r => r.text()).then(t => out.textContent = t)\">Ask</button>"
                    .to_owned(),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    on_page_with(
        address,
        |settings| settings.network_notes = true,
        async |session, _| {
            let asked = session.execute("click", json!({ "ref": "Ask" })).await;
            assert!(asked.ok, "{}", asked.output);
            assert!(asked.output.contains("arrived"), "the answer was not waited for: {}", asked.output);
        },
    )
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_hung_chrome_is_stopped_and_the_next_command_gets_a_fresh_one() {
    let address = serve("<!doctype html><title>t</title><h1>Alive</h1>".into());
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path(), temp.path());
    settings.attach = None;
    let host = Host::new(settings);
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut session = host.session();
            let url = format!("http://{address}");
            assert!(session.execute("navigate", json!({ "url": url })).await.ok);
            let owner = host.shared().await.unwrap();
            let pid = owner.process.as_ref().unwrap().chrome.borrow().id().unwrap();
            std::process::Command::new("kill").args(["-STOP", &pid.to_string()]).status().unwrap();
            owner.cdp.disconnect();
            drop(owner);
            // The command that meets the hung browser either fails saying so or,
            // where it can start over, goes on in a fresh one and says that.
            let hung = session.execute("navigate", json!({ "url": url })).await;
            let told = hung.output.contains("stopped answering");
            assert!(told, "the agent was not told: {}", hung.output);
            let again = session.execute("navigate", json!({ "url": url })).await;
            assert!(again.ok && again.output.contains("Alive"), "{}", again.output);
            let fresh = host.shared().await.unwrap();
            assert_ne!(fresh.process.as_ref().unwrap().chrome.borrow().id(), Some(pid));
            host.shutdown().await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_says_a_page_crashed_and_reopens_it() {
    let address = serve("<!doctype html><title>t</title><h1>Alive</h1>".into());
    on_page(address, async |session, _| {
        let crashed = session.execute("cdp", json!({ "method": "Page.crash" })).await;
        let again = session.execute("navigate", json!({ "url": format!("http://{address}") })).await;
        let told = format!("{}\n{}", crashed.output, again.output);
        assert!(told.contains("The page crashed"), "{told}");
        assert!(again.ok && again.output.contains("Alive"), "{}", again.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_stops_a_navigation_the_site_never_answers() {
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let silent_address = silent.local_addr().unwrap();
    std::thread::spawn(move || {
        let held: Vec<_> = silent.incoming().flatten().collect();
        drop(held);
    });
    let page = serve("<!doctype html><title>t</title><h1>Alive</h1>".into());
    on_page_with(
        page,
        |settings| settings.waits.navigate = std::time::Duration::from_secs(2),
        async |session, _| {
            let started = std::time::Instant::now();
            let stuck =
                session.execute("navigate", json!({ "url": format!("http://{silent_address}/") })).await;
            assert!(!stuck.ok && stuck.output.contains("did not start answering"), "{}", stuck.output);
            assert!(started.elapsed() < std::time::Duration::from_secs(15), "{:?}", started.elapsed());
            let again = session.execute("navigate", json!({ "url": format!("http://{page}/") })).await;
            assert!(again.ok && again.output.contains("Alive"), "{}", again.output);
        },
    )
    .await;
}

/// Any navigation nobody answers is stopped, not only the navigate command's,
/// so a restored or retried tab is not left loading for minutes.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_leaves_no_tab_loading_after_a_navigation_nobody_answers() {
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let silent_address = silent.local_addr().unwrap();
    std::thread::spawn(move || {
        let held: Vec<_> = silent.incoming().flatten().collect();
        drop(held);
    });
    on_page(serve("<!doctype html><title>t</title><h1>Alive</h1>".into()), async |session, _| {
        let tab = session.current().await.unwrap();
        let wait = std::time::Duration::from_secs(1);
        let url = format!("http://{silent_address}/");
        let landing = crate::page::navigation::navigate(&tab, &url, wait).await.unwrap();
        assert!(matches!(landing, crate::page::navigation::Landing::NoAnswer));
        let stopped = async {
            while !tab.cdp.events().navigation_quiet(&tab.session, std::time::Duration::ZERO) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        };
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), stopped).await.is_ok(),
            "still loading"
        );
    })
    .await;
}

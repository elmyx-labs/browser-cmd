//! Looking under the page: the console, raw CDP, and failed requests noted
//! when asked for.

use super::*;

/// Read through the Log domain, with no Runtime tell.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_console_reports_a_failed_request() {
    // A subresource that cannot load — port 1 is refused — which the browser
    // records as a network error in its log.
    let address = serve(
        "<!doctype html><meta charset=utf-8><title>t</title>\
         <img src=\"http://127.0.0.1:1/nope.png\"><p>hi</p>"
            .to_owned(),
    );
    on_page(address, async |session, _| {
        session.execute("wait", json!({ "seconds": 1.0 })).await;
        let logs = session.execute("console", json!({})).await;
        assert!(logs.ok, "console failed: {}", logs.output);
        let text = logs.output.to_lowercase();
        assert!(
            text.contains("fail")
                || text.contains("error")
                || text.contains("net::")
                || text.contains("127.0.0.1:1"),
            "the failed request was not logged: {}",
            logs.output
        );
        let original = session.book.active.as_ref().unwrap().clone();
        original.cdp.disconnect();
        let cached = session.execute("console", json!({})).await;
        assert!(cached.ok, "{}", cached.output);
        assert_eq!(cached.output, logs.output, "disconnect lost captured diagnostics");
        assert!(Rc::ptr_eq(&session.book.active.as_ref().unwrap().cdp, &original.cdp));
        assert!(!original.cdp.alive(), "console recovered the connection");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_cdp_passes_a_raw_method_through() {
    let address = serve("<!doctype html><meta charset=utf-8><title>t</title><p>hi</p>".to_owned());
    on_page(address, async |session, _| {
        let out =
            session.execute("cdp", json!({ "method": "DOM.getDocument", "params": { "depth": 0 } })).await;
        assert!(out.ok, "cdp failed: {}", out.output);
        assert!(
            out.output.contains("nodeId") && out.output.contains("root"),
            "no CDP result: {}",
            out.output
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_reports_a_request_the_page_did_not_get_when_asked_to() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0_u8; 8192];
            let read = std::io::Read::read(&mut stream, &mut request).unwrap_or_default();
            let wanted = String::from_utf8_lossy(&request[..read]).into_owned();
            let (status, body) = match wanted.contains("/missing.js") {
                true => ("404 Not Found", String::new()),
                false => (
                    "200 OK",
                    format!(
                        "<!doctype html><title>t</title><h1>Here</h1><script src=\"http://{address}/missing.js\"></script>"
                    ),
                ),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    let quiet = on_page(address, async |_, opened| opened).await;
    assert!(!quiet.output.contains("404"), "off by default: {}", quiet.output);
    let told =
        on_page_with(address, |settings| settings.network_notes = true, async |_, opened| opened).await;
    assert!(told.output.contains("missing.js") && told.output.contains("404"), "{}", told.output);
}

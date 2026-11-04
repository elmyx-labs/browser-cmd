//! The data a page fetched for itself, read back with `network`.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_reads_back_the_json_a_page_fetched() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0_u8; 8192];
            let read = std::io::Read::read(&mut stream, &mut request).unwrap_or_default();
            let (kind, body) = match String::from_utf8_lossy(&request[..read]).starts_with("GET /api/points")
            {
                true => (
                    "application/json",
                    format!(
                        "{{\"points\":[{}]}}",
                        "{\"city\":\"Lyon\",\"t\":21},".repeat(400) + "{\"city\":\"Nice\",\"t\":25}"
                    ),
                ),
                false => (
                    "text/html",
                    "<!doctype html><title>t</title><canvas width=600 height=400></canvas>
                    <button onclick=\"fetch('/api/points').then(r => r.json())\">Load</button>"
                        .to_owned(),
                ),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    on_page(address, async |session, _| {
        let first = session.execute("network", json!({})).await;
        assert!(first.ok && first.output.contains("Recording"), "{}", first.output);
        assert!(session.execute("click", json!({ "ref": "Load" })).await.ok);
        let listed = session.execute("network", json!({})).await;
        assert!(
            listed.output.contains("200 Fetch application/json") && listed.output.contains("/api/points"),
            "{}",
            listed.output
        );
        let none = session.execute("network", json!({ "find": "nothing-like-this" })).await;
        assert!(none.output.contains("No data responses"), "{}", none.output);
        let body =
            session.execute("network", json!({ "find": "points", "body": true, "max_bytes": 200 })).await;
        assert!(body.output.contains("{\"points\":[{\"city\":\"Lyon\""), "{}", body.output);
        let saved = body.output.split("saved to ").nth(1).and_then(|rest| rest.split(']').next()).unwrap();
        assert!(std::fs::read_to_string(saved).unwrap().contains("Nice"), "the whole body is kept");
    })
    .await;
}

/// A tab asked to record goes on recording once attached again; one never
/// asked is not recorded for being attached.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_goes_on_recording_a_tab_attached_again() {
    let address = serve("<!doctype html><title>t</title><h1>Here</h1>".into());
    on_page(address, async |session, _| {
        let asked = session.execute("network", json!({})).await;
        assert!(asked.ok && asked.output.contains("Recording"), "{}", asked.output);
        let first = session.book.active.as_ref().unwrap().session.clone();
        let url = format!("http://{address}/");
        assert!(session.execute("tab", json!({ "action": "new", "url": url })).await.ok);
        assert!(session.execute("tab", json!({ "action": "switch", "id": "t1" })).await.ok);
        assert_ne!(session.book.active.as_ref().unwrap().session, first, "attached again");
        let again = session.execute("network", json!({})).await;
        assert!(again.ok && !again.output.contains("Recording"), "{}", again.output);
        assert!(session.execute("tab", json!({ "action": "switch", "id": "t2" })).await.ok);
        let never = session.execute("network", json!({})).await;
        assert!(never.ok && never.output.contains("Recording"), "{}", never.output);
    })
    .await;
}

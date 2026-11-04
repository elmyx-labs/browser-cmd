//! Many pages read in one call, each in a tab of its own and kept when left
//! open, and a tab that stops answering replaced rather than waited on.

use super::*;

/// A server whose every path is its own small page with a picture, /gone a refusal.
fn site() -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0_u8; 8192];
            let read = std::io::Read::read(&mut stream, &mut request).unwrap_or_default();
            let path =
                String::from_utf8_lossy(&request[..read]).split_whitespace().nth(1).unwrap_or("/").to_owned();
            let status = if path == "/gone" { "404 Not Found" } else { "200 OK" };
            let say =
                |what: &str| format!("document.body.insertAdjacentHTML('beforeend','<p>picture {what}</p>')");
            let body = format!(
                "<!doctype html><title>Page {path}</title><h1>About {path}</h1><p>The price at {path} is 42 dollars.</p>\
                 <img src=/pic.gif onload=\"{}\" onerror=\"{}\"><script>document.body.insertAdjacentHTML('beforeend', '<p>Agent ' + navigator.userAgent + \
                 ' webdriver ' + navigator.webdriver + '</p>')</script>",
                say("shown"),
                say("not fetched")
            );
            // The smallest GIF: one transparent pixel.
            let gif = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";
            let (kind, body) = match path.ends_with(".gif") {
                true => ("image/gif", gif.to_vec()),
                false => ("text/html", body.into_bytes()),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, &[response.as_bytes(), &body].concat());
        }
    });
    address
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_reads_many_pages_in_tabs_of_their_own() {
    let address = site();
    on_page_with(
        address,
        |settings| settings.limits.text = 500,
        async |session, _| {
            let tab = session.current().await.unwrap();
            let before = tab.url().await.unwrap();
            let urls: Vec<String> = ["/a", "/b", "/c", "/d", "/e", "/gone"]
                .iter()
                .map(|p| format!("http://{address}{p}"))
                .collect();
            let read = session.execute("pages", json!({ "urls": urls, "find": "price" })).await;
            assert!(read.ok, "{}", read.output);
            let head = &read.output[..read.output.find("\n\n## ").unwrap_or(read.output.len())];
            assert!(
                head.starts_with("<browser trusted=\"no\">\n1. Page /a — ")
                    && head.contains("\n6. Page /gone"),
                "every page in the summary, numbered as its block: {head}"
            );
            assert!(read.output.contains(&format!("## 6. http://{address}/gone\n")), "{}", read.output);
            for path in ["/a", "/b", "/c", "/d", "/e"] {
                assert!(
                    read.output.contains(&format!("The price at {path} is 42 dollars")),
                    "{path}: {}",
                    read.output
                );
            }
            assert!(read.output.contains("The server answered 404"), "{}", read.output);
            let pictured = session.execute("pages", json!({ "urls": [format!("http://{address}/p")] })).await;
            assert!(pictured.output.starts_with("<browser trusted=\"no\">\n## 1. "), "{}", pictured.output);
            assert!(
                pictured.output.contains("picture not fetched"),
                "a page only read loads no pictures: {}",
                pictured.output
            );
            assert_eq!(
                session.current().await.unwrap().url().await.unwrap(),
                before,
                "the current tab is untouched"
            );
            // The session's own tab and a pages tab wear the same identity.
            let own = session.execute("text", json!({ "find": "Agent", "max_bytes": 2000 })).await.output;
            let agent = |text: &str| {
                text.split("Agent ")
                    .nth(1)
                    .and_then(|rest| rest.split(" webdriver").next())
                    .map(str::to_owned)
            };
            let alone = session
                .execute("pages", json!({ "urls": [format!("http://{address}/ua")], "find": "Agent" }))
                .await;
            assert!(agent(&own).is_some() && agent(&alone.output) == agent(&own), "{}\n{own}", alone.output);
            assert!(
                !alone.output.contains("Headless") && alone.output.contains("webdriver false"),
                "{}",
                alone.output
            );
            let listed = session.execute("tab", json!({ "action": "list" })).await;
            assert!(!listed.output.contains("/a"), "read tabs are closed: {}", listed.output);
            let kept = session
                .execute("pages", json!({ "urls": [format!("http://{address}/k")], "keep": true }))
                .await;
            assert!(kept.output.contains("picture shown"), "a kept tab is the whole page: {}", kept.output);
            let id = kept
                .output
                .split("left open as tab ")
                .nth(1)
                .and_then(|rest| rest.split(')').next())
                .unwrap();
            assert_eq!(id, "t2", "{}", kept.output);
            let listed = session.execute("tab", json!({ "action": "list" })).await;
            assert!(
                listed.output.contains("/k") && listed.output.contains(&format!(" {id} kept ok ")),
                "a kept tab stays: {}",
                listed.output
            );
            let tidied = session.execute("tab", json!({ "action": "tidy" })).await;
            assert_eq!(tidied.output, format!("Closed t1; open and kept: {id}. Switched to {id}."));
            let switched = session.execute("tab", json!({ "action": "switch", "id": id })).await;
            assert!(switched.ok, "the handle names the tab: {}", switched.output);
            let missing = session.execute("tab", json!({ "action": "switch", "id": "12" })).await;
            assert!(
                !missing.ok && missing.output.contains("this session has no tab 12"),
                "{}",
                missing.output
            );
        },
    )
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_replaces_a_tab_that_stops_answering() {
    let address = site();
    on_page_with(
        address,
        |settings| settings.waits.request = std::time::Duration::from_secs(3),
        async |session, _| {
            let tab = session.current().await.unwrap();
            let _ = tab.send(
                "Runtime.evaluate",
                json!({ "expression": "setTimeout(() => { while (true) {} }, 100)", "returnByValue": true }),
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let stuck = session.execute("text", json!({})).await;
            assert!(!stuck.ok && stuck.output.contains("stopped answering"), "{}", stuck.output);
            assert!(session.book.registry.named("t1").is_none(), "the hung tab is forgotten");
            let again =
                session.execute("navigate", json!({ "url": format!("http://{address}/fresh") })).await;
            assert!(again.ok && again.output.contains("About /fresh"), "{}", again.output);
        },
    )
    .await;
}

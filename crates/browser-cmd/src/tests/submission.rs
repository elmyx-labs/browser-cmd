//! Sending a form: once, whichever road reaches it, and only when it was
//! really sent; and the reply waits for the page the form went to.

use super::*;

/// One order must not become two because the page was slow to answer.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_refuses_to_send_one_form_twice_unless_it_is_meant() {
    let orders = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = orders.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0_u8; 8192];
            let read = std::io::Read::read(&mut stream, &mut request).unwrap_or_default();
            if String::from_utf8_lossy(&request[..read]).contains("/order") {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            // The page never navigates: the form is sent, and nothing
            // visible happens, which is the shape that costs money.
            let body = r#"<!doctype html><title>t</title><h1>Cart</h1>
            <form onsubmit="fetch('/order'); return false"><button type="submit">Place order</button></form>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    on_page(address, async |session, _| {
        let first = session.execute("click", json!({ "ref": "Place order" })).await;
        assert!(first.ok, "{}", first.output);
        let second = session.execute("click", json!({ "ref": "Place order" })).await;
        assert!(!second.ok, "the second order went through: {}", second.output);
        assert!(second.output.contains("again=true"), "{}", second.output);

        let meant = session.execute("click", json!({ "ref": "Place order", "again": true })).await;
        assert!(meant.ok, "{}", meant.output);
        // Two orders: the one asked for and the one asked for twice.
        assert_eq!(orders.load(std::sync::atomic::Ordering::SeqCst), 2);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_refuses_every_road_to_the_same_form_twice() {
    let orders = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = orders.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0_u8; 8192];
            let read = std::io::Read::read(&mut stream, &mut request).unwrap_or_default();
            if String::from_utf8_lossy(&request[..read]).contains("/order") {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            let body = r#"<!doctype html><title>t</title><h1>Cart</h1>
            <form onsubmit="fetch('/order'); return false">
              <label>Promo code <input name="promo"></label>
              <button type="submit">Place order</button></form>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    on_page(address, async |session, _| {
        // Enter in the named field sends the form once.
        let typed =
            session.execute("type", json!({ "ref": "Promo code", "text": "SAVE", "submit": true })).await;
        assert!(typed.ok, "{}", typed.output);
        // Enter again, through the focused field this time: refused.
        let pressed = session.execute("press", json!({ "key": "Enter" })).await;
        assert!(!pressed.ok && pressed.output.contains("again=true"), "{}", pressed.output);
        // Return is the same key by another name: refused.
        let returned = session.execute("press", json!({ "key": "Return" })).await;
        assert!(!returned.ok && returned.output.contains("again=true"), "{}", returned.output);
        // Typed with no ref into the focused field and submitted: refused.
        let typed = session.execute("type", json!({ "text": "X", "submit": true })).await;
        assert!(!typed.ok && typed.output.contains("again=true"), "{}", typed.output);
        // Space on the focused submit button presses it: refused.
        assert!(session.execute("press", json!({ "key": "Tab" })).await.ok);
        let spaced = session.execute("press", json!({ "key": "Space" })).await;
        assert!(!spaced.ok && spaced.output.contains("again=true"), "{}", spaced.output);
        // A click on the button's coordinates is the same form: refused.
        let snapshot = session.execute("snapshot", json!({ "coords": true })).await;
        // Positions come as one line of `@ref x,y` pairs after the tree.
        let button = snapshot
            .output
            .lines()
            .find(|l| l.contains("Place order"))
            .and_then(|l| l.split(" @e").nth(1))
            .and_then(|rest| rest.split(' ').next())
            .map(|rest| format!("@e{rest}"))
            .expect("the button's ref");
        let positions = snapshot
            .output
            .lines()
            .find(|l| l.starts_with(&format!("{button} ")) || l.contains(&format!("  {button} ")))
            .expect("positions");
        let point = positions
            .split_whitespace()
            .collect::<Vec<_>>()
            .chunks(2)
            .find(|pair| pair[0] == button)
            .and_then(|pair| pair[1].split_once(','))
            .map(|(x, y)| (x.parse::<f64>().unwrap(), y.parse::<f64>().unwrap()))
            .unwrap_or_else(|| panic!("the button's point, in:\n{}", snapshot.output));
        let aimed = session.execute("click", json!({ "x": point.0, "y": point.1 })).await;
        assert!(!aimed.ok && aimed.output.contains("again=true"), "{}", aimed.output);
        // Meant: the same road, said so.
        let meant = session.execute("press", json!({ "key": "Enter", "again": true })).await;
        assert!(meant.ok, "{}", meant.output);
        assert_eq!(orders.load(std::sync::atomic::Ordering::SeqCst), 2);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_click_that_never_landed_is_not_a_submission() {
    let address=serve(r#"<!doctype html><form onsubmit='event.preventDefault()'><button>Send</button></form><div id='cover' style='position:fixed;inset:0;background:white;z-index:100'>Cover</div>"#.into());
    on_page(address, async |session, _| {
        let first = session.execute("click", json!({"ref":"Send"})).await;
        assert!(!first.ok, "must be covered");
        let _ =
            session.execute("eval", json!({"expression":"document.getElementById('cover').remove()"})).await;
        let second = session.execute("click", json!({"ref":"Send"})).await;
        assert!(second.ok, "never submitted but now blocked: {}", second.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn submitting_a_form_waits_for_its_navigation_instead_of_accepting_the_old_quiet_document() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut bytes = [0; 4096];
            let n = std::io::Read::read(&mut stream, &mut bytes).unwrap_or(0);
            let done = String::from_utf8_lossy(&bytes[..n]).starts_with("GET /done");
            if done {
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            let body = if done {
                "<!doctype html><h1>Done</h1>"
            } else {
                "<!doctype html><form action='/done'><input aria-label='Name'><button>Send</button></form>"
            };
            let _ = std::io::Write::write_all(&mut stream, format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes());
        }
    });
    on_page(address, async |session, _| {
        let submitted = session.execute("type", json!({"ref":"Name","text":"x","submit":true})).await;
        assert!(
            submitted.ok && submitted.output.contains("/done") && submitted.output.contains("Done"),
            "{}",
            submitted.output
        );
    })
    .await;
}

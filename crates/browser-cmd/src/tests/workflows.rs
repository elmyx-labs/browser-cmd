//! Whole tasks driven end to end, the tools composed as an agent uses them.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_completes_the_type_click_loop_without_a_snapshot_when_requested() {
    let address = serve(r#"<!doctype html><title>Native browser test</title><h1>Form</h1><label>Name <input aria-label="Name"></label>
            <select aria-label="Colour"><option>Red</option><option>Green</option></select>
            <button onclick="setTimeout(() => { alert('saved'); document.querySelector('#result').textContent = document.querySelector('input').value + ' ' + document.querySelector('select').value }, 300)">Save</button><p id="result">Empty</p>"#.to_owned());
    on_page(address, async |session, navigated| {
        assert!(
            navigated.output.contains("heading \"Form\""),
            "the state comes with the action: {}",
            navigated.output
        );
        assert!(navigated.output.contains("textbox \"Name\" @e"), "{}", navigated.output);
        let typed = session.execute("type", json!({ "ref": "Name", "text": "Ada" })).await;
        assert!(typed.ok, "{}", typed.output);
        assert!(typed.output.contains("[value=\"Ada\"]"), "{}", typed.output);
        let selected = session.execute("select", json!({ "ref": "Colour", "option": "green" })).await;
        assert!(selected.ok && selected.output.starts_with("Selected \"Green\""), "{}", selected.output);
        let clicked = session.execute("click", json!({ "ref": "Save" })).await;
        assert!(clicked.ok, "{}", clicked.output);
        let waited = session.execute("wait", json!({ "text": "Ada Green" })).await;
        assert!(waited.ok, "{}", waited.output);
        assert!(
            clicked.output.contains("alert dialog saying \"saved\"; it was accepted")
                || waited.output.contains("alert dialog saying \"saved\"; it was accepted"),
            "{}",
            waited.output
        );
        // A step closed by a claim about the page, which is the only
        // kind of "done" this harness accepts anywhere else.
        let held = session.execute("verify", json!({ "text": "Ada Green", "gone": "Empty" })).await;
        assert!(held.ok, "{}", held.output);
        assert!(held.output.starts_with("Verified:"), "{}", held.output);
        let broken = session.execute("verify", json!({ "text": "Ada Red", "url": "elsewhere" })).await;
        assert!(!broken.ok, "a claim that does not hold is an error: {}", broken.output);
        assert!(broken.output.contains("\"Ada Red\" is not on the page"), "{}", broken.output);
        assert!(broken.output.contains("does not contain \"elsewhere\""), "{}", broken.output);
        let vanished = session.execute("verify", json!({ "ref": "@e99" })).await;
        assert!(!vanished.ok, "{}", vanished.output);
        let nothing = session.execute("verify", json!({})).await;
        assert!(!nothing.ok, "a claim about nothing is not a check: {}", nothing.output);

        // Fields by selector, with a shape the caller can check,
        // instead of the whole page in prose.
        let picked = session
            .execute(
                "extract",
                json!({ "fields": { "heading": "h1", "answer": "#result", "colour": "select@value" } }),
            )
            .await;
        assert!(picked.ok, "{}", picked.output);
        assert!(picked.output.contains("\"heading\":\"Form\""), "{}", picked.output);
        assert!(picked.output.contains("\"answer\":\"Ada Green\""), "{}", picked.output);
        assert!(picked.output.contains("\"colour\":\"Green\""), "{}", picked.output);
        assert!(picked.output.contains("trusted=\"no\""), "extraction is data: {}", picked.output);
        let rows = session.execute("extract", json!({ "each": "option", "fields": { "name": "." } })).await;
        assert!(rows.ok, "{}", rows.output);
        assert!(
            rows.output.contains("\"name\":\"Red\"") && rows.output.contains("\"matched\":2"),
            "{}",
            rows.output
        );
        let empty = session.execute("extract", json!({ "fields": {} })).await;
        assert!(!empty.ok, "{}", empty.output);

        let shot = session.execute("screenshot", json!({})).await;
        assert!(shot.ok, "{}", shot.output);
        assert_eq!(shot.images.len(), 1, "the picture rides with the result");
        assert!(
            std::fs::metadata(session.settings().workspace.join("media/browser/browser-0001.png"))
                .unwrap()
                .len()
                > 100
        );
        let back = session.execute("navigate", json!({ "url": "back" })).await;
        assert!(back.ok && back.output.contains("URL: about:blank"), "{}", back.output);
        let forward = session.execute("navigate", json!({ "url": "forward" })).await;
        assert!(forward.ok && forward.output.contains("button \"Save\""), "{}", forward.output);
        let nowhere = session.execute("navigate", json!({ "url": "forward" })).await;
        assert!(!nowhere.ok, "{}", nowhere.output);
        let closed = session.execute("close", json!({})).await;
        assert!(closed.ok, "{}", closed.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_completes_a_constrained_task_end_to_end() {
    // A small shop. The rating lives only in a class, so it is in neither
    // the accessibility tree nor the text — extract must reach the markup.
    // Cheapest four-star-or-better in-stock item is Desert Roads (£20):
    // Coastal Trails is cheaper and five-star but out of stock, Alpine
    // Guide is cheap but three-star, Balloon Atlas is four-star but £25.
    let items = [
        ("a", "Alpine Guide", "30.00", "Three", "In stock"),
        ("b", "Balloon Atlas", "25.00", "Four", "In stock"),
        ("c", "Coastal Trails", "15.00", "Five", "Out of stock"),
        ("d", "Desert Roads", "20.00", "Four", "In stock"),
    ];
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0_u8; 8192];
            let read = std::io::Read::read(&mut stream, &mut request).unwrap_or_default();
            let path = String::from_utf8_lossy(&request[..read])
                .lines()
                .next()
                .and_then(|line| line.split(' ').nth(1))
                .unwrap_or("/")
                .to_owned();
            let body = if let Some(id) = path.strip_prefix("/item/") {
                match items.iter().find(|item| item.0 == id) {
                    Some((_, title, price, rating, stock)) => format!(
                        "<!doctype html><meta charset=utf-8><title>{title}</title><h1>{title}</h1>\
                         <p class=\"price\">£{price}</p><p class=\"star-rating {rating}\">rated</p>\
                         <p class=\"stock\">{stock}</p>"
                    ),
                    None => "<!doctype html><title>gone</title><h1>No such item</h1>".to_owned(),
                }
            } else {
                let rows: String = items
                    .iter()
                    .map(|(id, title, price, rating, stock)| {
                        format!(
                            "<li class=\"item\"><a href=\"/item/{id}\" title=\"{title}\">{title}</a>\
                         <span class=\"price\">£{price}</span>\
                         <span class=\"star-rating {rating}\"></span>\
                         <span class=\"stock\">{stock}</span></li>"
                        )
                    })
                    .collect();
                format!(
                    "<!doctype html><meta charset=utf-8><title>Travel</title><h1>Travel</h1><ol>{rows}</ol>"
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    on_page(address, async |session, opened| {
        assert!(!opened.output.contains("Four"), "the rating is not in the tree: {}", opened.output);

        // Pull every row's facts, the rating out of its class.
        let listed = session
            .execute(
                "extract",
                json!({ "each": "li.item", "fields": {
                    "title": "a@title", "price": ".price", "rating": ".star-rating@class", "stock": ".stock"
                } }),
            )
            .await;
        assert!(listed.ok, "{}", listed.output);
        let extracted: Value = serde_json::from_str(
            listed
                .output
                .trim_start_matches("<browser trusted=\"no\">")
                .trim_end_matches("</browser>")
                .trim(),
        )
        .expect("extract returns a JSON envelope");
        let rows = extracted["rows"].as_array().expect("extract returns rows");
        assert_eq!(rows.len(), 4, "{}", listed.output);

        // The task's constraints, applied to what was pulled: rating
        // four or more, in stock, then the cheapest of those.
        let stars = |r: &str| match r.rsplit(' ').next() {
            Some("Five") => 5,
            Some("Four") => 4,
            Some("Three") => 3,
            Some("Two") => 2,
            _ => 1,
        };
        let pick = rows
            .iter()
            .filter(|row| stars(row["rating"].as_str().unwrap_or("")) >= 4)
            .filter(|row| row["stock"].as_str() == Some("In stock"))
            .min_by(|a, b| {
                let price = |row: &Value| {
                    row["price"]
                        .as_str()
                        .unwrap_or("")
                        .trim_start_matches('£')
                        .parse::<f64>()
                        .unwrap_or(f64::MAX)
                };
                price(a).partial_cmp(&price(b)).unwrap()
            })
            .expect("a candidate survives the constraints");
        let chosen = pick["title"].as_str().unwrap().to_owned();
        assert_eq!(chosen, "Desert Roads", "the constraints picked the wrong item: {rows:?}");

        // Open the item the task's own logic chose, by its name, and
        // confirm the page is the one the constraints promised.
        let went = session.execute("click", json!({ "ref": chosen })).await;
        assert!(went.ok, "{}", went.output);
        let confirmed = session
            .execute(
                "verify",
                json!({
                    "text": ["Desert Roads", "£20.00", "In stock"], "gone": "Out of stock", "url": "/item/d"
                }),
            )
            .await;
        assert!(confirmed.ok, "the task's own answer did not check out: {}", confirmed.output);
    })
    .await;
}

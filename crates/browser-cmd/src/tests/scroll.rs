//! Scrolling a real page: the region under the pointer, a feed to its end,
//! and a phrase into view.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_scrolls_the_panel_under_the_pointer_and_not_the_page() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0_u8; 8192];
            let read = std::io::Read::read(&mut stream, &mut request).unwrap_or_default();
            let inner = String::from_utf8_lossy(&request[..read]).contains("/inner");
            // The frame is served from another port, so it is another
            // origin: the page cannot measure what happens inside it, and
            // must say that rather than report a zero.
            let page = format!(
                r#"<!doctype html><title>t</title>
            <style>body{{margin:0;height:4000px}}
            #panel{{position:fixed;right:0;top:0;width:280px;height:400px;overflow-y:scroll}}
            #panel p{{height:120px;margin:0}}
            iframe{{position:fixed;left:0;bottom:0;width:300px;height:200px;border:0}}</style>
            <h1>Page</h1>
            <div id="panel" tabindex="0" role="region" aria-label="Side panel">
              <p>one</p><p>two</p><p>three</p><p>four</p><p>five</p>
              <p>six</p><p>seven</p><p>eight</p><p>nine</p><p>ten</p>
            </div>
            <iframe src="http://localhost:{}/inner"></iframe>"#,
                address.port()
            );
            let body = match inner {
                true => "<!doctype html><title>inner</title><p>inner</p>".to_owned(),
                false => page,
            };
            let body = body.as_str();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    on_page(address, async |session, _| {
        async fn measure(session: &mut Session) -> (f64, f64) {
            let read = session
                .execute(
                    "eval",
                    json!({ "expression": "[scrollY, document.getElementById('panel').scrollTop]" }),
                )
                .await;
            assert!(read.ok, "{}", read.output);
            let numbers: Vec<f64> =
                read.output.lines().find_map(|line| serde_json::from_str(line).ok()).unwrap_or_default();
            assert_eq!(numbers.len(), 2, "{}", read.output);
            (numbers[0], numbers[1])
        }
        assert_eq!(measure(session).await, (0.0, 0.0));

        // No target: the page moves, the panel does not.
        let paged = session.execute("scroll", json!({ "direction": "down" })).await;
        assert!(paged.ok, "{}", paged.output);
        let (page, panel) = measure(session).await;
        assert!(page > 0.0, "the page did not scroll: {page}");
        assert_eq!(panel, 0.0, "the panel moved when nothing pointed at it");

        // By point: nothing is aimed at, so the page is not touched at
        // all — which is what a live list needs, since its refs do not
        // survive the round trip that would use them.
        let wide = session.execute("eval", json!({ "expression": "innerWidth" })).await;
        let inner: f64 = wide.output.lines().find_map(|line| line.trim().parse().ok()).expect(&wide.output);
        let by_point = session
            .execute("scroll", json!({ "x": inner - 140.0, "y": 200, "direction": "down", "amount": 300 }))
            .await;
        assert!(by_point.ok, "{}", by_point.output);
        assert!(
            by_point.output.contains("This region moved") && by_point.output.contains(" of "),
            "the wheel says where it went: {}",
            by_point.output
        );
        let (page_again, moved) = measure(session).await;
        assert!(moved > 0.0, "the panel did not scroll: {moved}");
        assert!(
            page_again - page < moved,
            "the wheel turned the page rather than the panel: page {page} to {page_again}, panel {moved}"
        );

        // By ref: the panel scrolls further. Aiming brings the target
        // into view first, so the page may be nudged; the wheel itself
        // still turns in the panel.
        let over = session
            .execute("scroll", json!({ "ref": "Side panel", "direction": "down", "amount": 300 }))
            .await;
        assert!(over.ok, "{}", over.output);
        assert!(over.output.contains("over Side panel"), "{}", over.output);
        let (_, further) = measure(session).await;
        assert!(further > moved, "the panel did not scroll further: {further} after {moved}");

        // Run the region out and it says so, which is what ends a loop
        // rather than leaving it to guess from an empty result.
        let mut last = String::new();
        for round in 0..12 {
            let more = session
                .execute(
                    "scroll",
                    json!({ "x": inner - 140.0, "y": 200, "direction": "down", "amount": 500 }),
                )
                .await;
            assert!(more.ok, "{}", more.output);
            last = more.output;
            if last.contains("already at the bottom") {
                break;
            }
            assert!(round < 11, "the region never reported its end: {last}");
        }
        assert!(last.contains("This region did not move: it is already at the bottom."), "{last}");

        // The frame sits at the bottom left, where the page cannot see in.
        let blind = session.execute("scroll", json!({ "x": 150, "y": 620, "direction": "down" })).await;
        assert!(blind.ok, "{}", blind.output);
        assert!(
            blind.output.contains("another document, which this page cannot measure"),
            "a frame is not a zero: {}",
            blind.output
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_scroll_to_bottom_loads_an_infinite_list() {
    // A page that appends ten rows whenever the scroll nears its end, up to a
    // hundred — the shape of a feed or a catalogue that loads as you go.
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title><style>.row{height:120px}</style><div id=list></div>
        <script>
          let n = 0;
          const add = () => { for (let i = 0; i < 10 && n < 100; i++) { const d = document.createElement('p'); d.className = 'row'; d.textContent = 'Row ' + (++n); list.appendChild(d); } };
          add();
          addEventListener('scroll', () => { if (innerHeight + scrollY >= document.body.scrollHeight - 400) add(); });
        </script>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let before = session
            .execute("eval", json!({ "expression": "document.querySelectorAll('.row').length" }))
            .await;
        assert!(before.output.contains("10"), "the list did not start at ten: {}", before.output);
        let scrolled = session.execute("scroll", json!({ "to": "bottom" })).await;
        assert!(scrolled.ok, "{}", scrolled.output);
        assert!(scrolled.output.contains("reached the bottom"), "did not reach the end: {}", scrolled.output);
        let after = session
            .execute("eval", json!({ "expression": "document.querySelectorAll('.row').length" }))
            .await;
        assert!(after.output.contains("100"), "the whole list did not load: {}", after.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_scroll_bottom_drives_an_observer_feed() {
    // Modern infinite scroll: an IntersectionObserver on a sentinel at the end
    // appends the next page when it comes into view — not a scroll-event handler.
    let address = serve(r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div id=list></div><div id=end style="height:1px"></div>
        <script>
          let n=0; const add=()=>{ for(let i=0;i<10 && n<80;i++){const d=document.createElement('p');d.className='row';d.style.height='100px';d.textContent='Row '+(++n);list.appendChild(d);} };
          add();
          new IntersectionObserver((es)=>{ if(es[0].isIntersecting && n<80) add(); }).observe(document.getElementById('end'));
        </script>"#.to_owned());
    on_page(address, async |session, _| {
        let s = session.execute("scroll", json!({"to":"bottom"})).await;
        assert!(s.ok, "scroll to bottom failed: {}", s.output);
        let count =
            session.execute("eval", json!({"expression":"document.querySelectorAll('.row').length"})).await;
        assert!(count.output.contains("80"), "the observer feed was not driven to its end: {}", count.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_scrolls_text_into_view() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div style="height:3000px">top matter</div>
        <p id="mark">The quiet conclusion</p>
        <div style="height:1500px">more</div>
        <button id="far">Far button</button>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        // A ref word, as the shell gives it, brings that element into view.
        let tree = session.execute("snapshot", json!({})).await;
        let line = tree.output.lines().find(|line| line.contains("Far button")).unwrap_or_default();
        let handle = line.split_whitespace().find(|word| word.starts_with('@')).expect("a ref for the button");
        let words = crate::api::command::find("scroll").unwrap().arguments(&[handle.to_owned()]).unwrap();
        let scrolled = session.execute("scroll", words).await;
        assert!(scrolled.ok && scrolled.output.contains("into view"), "{}", scrolled.output);
        let seen = session
            .execute("eval", json!({ "expression": "(() => { const r = far.getBoundingClientRect(); return r.top >= 0 && r.bottom <= innerHeight; })()" }))
            .await;
        assert!(seen.output.contains("true"), "the ref was not brought into view: {}", seen.output);
        let scrolled =
            session.execute("scroll", json!({ "to": "quiet conclusion" })).await;
        assert!(scrolled.ok, "scroll to text failed: {}", scrolled.output);
        let seen = session
            .execute("eval", json!({ "expression": "(() => { const r = document.getElementById('mark').getBoundingClientRect(); return r.top >= 0 && r.top < innerHeight; })()" }))
            .await;
        assert!(seen.output.contains("true"), "the text was not brought into view: {}", seen.output);
        let missing =
            session.execute("scroll", json!({ "to": "no such words here anywhere" })).await;
        assert!(!missing.ok, "scroll to absent text should fail: {}", missing.output);
    })
    .await;
}

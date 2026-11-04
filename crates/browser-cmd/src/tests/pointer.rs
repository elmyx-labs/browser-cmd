//! Pointing at a thing on a real page: clicks by name and by ref, hover,
//! drag and right click, and aiming that lands inside frames, shadow roots,
//! oversized targets and the viewport that is really there.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_drags_a_slider_and_every_state_says_where_it_is() {
    let address = serve(r#"<!doctype html><title>t</title>
            <style>body{margin:0;height:3000px} input{width:400px;margin:40px}</style>
            <h1>Volume</h1>
            <input type="range" min="0" max="100" value="0" aria-label="Volume">
            <p id="level">0</p>
            <script>document.querySelector('input').oninput = e => level.textContent = e.target.value</script>"#.to_owned());
    on_page(address, async |session, opened| {
        assert!(opened.output.contains("Scroll: 0 of "), "no position on open: {}", opened.output);
        assert!(opened.output.contains("(top)"), "{}", opened.output);

        // The thumb sits at the left edge of the track at value 0; the
        // drag runs to its right edge.
        let track = session
            .execute("eval", json!({ "expression": "(r => [r.left + 8, r.top + r.height / 2, r.right - 8])(document.querySelector('input').getBoundingClientRect())" }))
            .await;
        let edges: Vec<f64> =
            track.output.lines().find_map(|line| serde_json::from_str(line).ok()).unwrap_or_default();
        assert_eq!(edges.len(), 3, "{}", track.output);
        let dragged = session
            .execute("drag", json!({ "from": { "x": edges[0], "y": edges[1] }, "to": { "x": edges[2], "y": edges[1] } }))
            .await;
        assert!(dragged.ok, "{}", dragged.output);
        assert!(dragged.output.starts_with("Dragged"), "{}", dragged.output);
        let level = session
            .execute("eval", json!({ "expression": "Number(level.textContent)" }))
            .await;
        let value: f64 = level.output.lines().find_map(|line| line.trim().parse().ok()).expect(&level.output);
        assert!(value >= 90.0, "the slider did not follow the drag: {value}");

        // The same by ref: the thumb is where the element is, and a drag
        // from it back toward the left edge lowers the value.
        let back = session
            .execute("drag", json!({ "from": { "ref": "Volume" }, "to": { "x": edges[0], "y": edges[1] } }))
            .await;
        assert!(back.ok, "{}", back.output);
        let lowered = session
            .execute("eval", json!({ "expression": "Number(level.textContent)" }))
            .await;
        let lower: f64 = lowered.output.lines().find_map(|line| line.trim().parse().ok()).expect(&lowered.output);
        assert!(lower < value, "the second drag did not move it back: {lower} after {value}");

        let down = session.execute("scroll", json!({ "direction": "down", "amount": 5000 })).await;
        assert!(down.ok, "{}", down.output);
        assert!(down.output.contains("(bottom)"), "the state does not say it is at the end: {}", down.output);
    })
    .await;
}

/// A control revealed only on hover appears in the state after the hover,
/// marked new, since a person would now see it.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_shows_what_a_hover_reveals() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <style>.menu{position:relative}.pop{display:none}.menu:hover .pop{display:block}</style>
        <div class="menu"><button>Account</button><div class="pop"><a href="/settings">Settings</a></div></div>"#
            .to_owned(),
    );
    on_page(address, async |session, opened| {
        assert!(
            !opened.output.contains("Settings"),
            "the hidden link was in the state before hover: {}",
            opened.output
        );
        let hovered = session.execute("hover", json!({ "ref": "Account" })).await;
        assert!(hovered.ok, "{}", hovered.output);
        assert!(
            hovered.output.contains("\"Settings\""),
            "the hover-revealed link is not in the state: {}",
            hovered.output
        );
        assert!(
            hovered.output.contains("Settings\" @e") && hovered.output.contains("[new]"),
            "it is not marked new: {}",
            hovered.output
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_clicks_by_name_below_the_fold() {
    // A role-less styled control (name-search path, not the ref path) placed
    // far below the opening viewport.
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div style="height:3000px">a tall page</div>
        <div style="cursor:pointer" onclick="document.querySelector('#out').textContent='opened'">Sign up</div>
        <p id="out">closed</p>
        <div style="height:1500px">and more below</div>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        // The control sits 3000px down, below the opening viewport.
        let clicked = session.execute("click", json!({ "ref": "Sign up" })).await;
        assert!(clicked.ok, "a control below the fold was not clicked by name: {}", clicked.output);
        let read = session
            .execute("eval", json!({ "expression": "document.querySelector('#out').textContent" }))
            .await;
        assert!(read.output.contains("opened"), "the off-screen click did not land: {}", read.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_clicks_a_role_less_control_by_text() {
    // A div that only a stylesheet and an onclick make a button — the shape of
    // a react-select box or a hand-rolled dropdown, invisible to the tree.
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div style="cursor:pointer" onclick="document.querySelector('#out').textContent='opened'">Open the menu</div>
        <p id="out">closed</p>"#
            .to_owned(),
    );
    on_page(address, async |session, opened| {
        // The div has no interactive role, so the tree gives it no ref.
        assert!(!opened.output.contains("Open the menu"), "a styled div got a ref: {}", opened.output);
        let clicked = session.execute("click", json!({ "ref": "Open the menu" })).await;
        assert!(clicked.ok, "role-less control not clicked by text: {}", clicked.output);
        let read = session
            .execute("eval", json!({ "expression": "document.querySelector('#out').textContent" }))
            .await;
        assert!(read.output.contains("opened"), "the click did not land on the control: {}", read.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_right_clicks_for_a_context_menu() {
    let address = serve(
        "<!doctype html><meta charset=utf-8><title>t</title><button aria-label='cell'>Cell</button><p id=out>idle</p><script>document.querySelector('button').addEventListener('contextmenu', e => { e.preventDefault(); out.textContent = 'menu'; });</script>".to_owned(),
    );
    on_page(address, async |session, _| {
        let right = session.execute("click", json!({ "ref": "cell", "right": true })).await;
        assert!(right.ok, "{}", right.output);
        assert!(right.output.contains("Right-clicked"), "not announced as a right click: {}", right.output);
        let read = session
            .execute("eval", json!({ "expression": "document.getElementById('out').textContent" }))
            .await;
        assert!(read.output.contains("menu"), "the context menu did not fire: {}", read.output);
    })
    .await;
}

/// The box model must carry the offset of both frames, not just the outer one.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_clicks_inside_a_nested_iframe() {
    // An iframe inside an iframe, each offset down the page, the button in the
    // innermost one — the shape of an embedded widget inside an embed.
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div style="height:120px">outer top</div>
        <iframe style="width:600px;height:400px" srcdoc='
          <div style=&quot;height:90px&quot;>middle top</div>
          <iframe style=&quot;width:400px;height:250px&quot; srcdoc=&quot;
            <button onclick=&amp;quot;document.title=String(Date.now())&amp;quot; id=b>Deep button</button>
          &quot;></iframe>
        '></iframe>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let clicked = session.execute("click", json!({ "ref": "Deep button" })).await;
        assert!(clicked.ok, "a button two iframes deep was not clicked: {}", clicked.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_clicks_by_name_inside_a_shadow_root() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <my-widget></my-widget><p id="out">idle</p>
        <script>
          customElements.define('my-widget', class extends HTMLElement {
            connectedCallback() {
              const root = this.attachShadow({ mode: 'open' });
              const d = document.createElement('div');
              d.style.cursor = 'pointer';
              d.textContent = 'Add to cart';
              d.onclick = () => { document.querySelector('#out').textContent = 'added'; };
              root.appendChild(d);
            }
          });
        </script>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let clicked = session.execute("click", json!({ "ref": "Add to cart" })).await;
        assert!(clicked.ok, "a control in a shadow root was not clicked by name: {}", clicked.output);
        let read = session
            .execute("eval", json!({ "expression": "document.querySelector('#out').textContent" }))
            .await;
        assert!(read.output.contains("added"), "the shadow-DOM click did not land: {}", read.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_reaches_into_a_shadow_root() {
    // elementFromPoint stops at the shadow host; the aim check must descend
    // or it reads the control as covered by the host div.
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title><div id=host></div>
        <script>
          const sr = host.attachShadow({ mode: 'open' });
          sr.innerHTML = '<input aria-label="name" style="display:block;margin:8px"><button>Save</button><p id=out>waiting</p>';
          sr.querySelector('button').onclick = () => { sr.getElementById('out').textContent = 'saved ' + sr.querySelector('input').value; };
        </script>"#
            .to_owned(),
    );
    on_page(address, async |session, opened| {
        assert!(opened.output.contains("\"name\""), "the shadow input got no ref: {}", opened.output);
        let typed = session.execute("type", json!({ "ref": "name", "text": "Ada" })).await;
        assert!(typed.ok, "could not type into the shadow control: {}", typed.output);
        let clicked = session.execute("click", json!({ "ref": "Save" })).await;
        assert!(clicked.ok, "could not click the shadow button: {}", clicked.output);
        let read = session
            .execute("eval", json!({ "expression": "host.shadowRoot.getElementById('out').textContent" }))
            .await;
        assert!(read.output.contains("saved Ada"), "the shadow interaction did not land: {}", read.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_reaches_into_an_iframe() {
    // A form in a frame offset down the page: a frame's own coordinates would
    // aim above it, and the aim check would call it covered by the iframe.
    let address = serve(
        "<!doctype html><meta charset=utf-8><title>t</title><h1 style='height:120px'>Header</h1><iframe style='width:320px;height:160px' srcdoc=\"<input aria-label='field'><button onclick='out.textContent=&quot;got:&quot;+document.querySelector(String.fromCharCode(105,110,112,117,116)).value'>Send</button><p id=out>idle</p>\"></iframe>".to_owned(),
    );
    on_page(address, async |session, opened| {
        assert!(opened.output.contains("\"field\""), "the iframe input got no ref: {}", opened.output);
        let typed = session.execute("type", json!({ "ref": "field", "text": "hello" })).await;
        assert!(typed.ok, "could not type into the iframe control: {}", typed.output);
        let clicked = session.execute("click", json!({ "ref": "Send" })).await;
        assert!(clicked.ok, "could not click the iframe button: {}", clicked.output);
        let read = session.execute("eval", json!({ "expression": "document.querySelector('iframe').contentDocument.getElementById('out').textContent" })).await;
        assert!(read.output.contains("got:hello"), "the iframe interaction did not land: {}", read.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn oversized_targets_are_clicked_inside_the_actual_viewport() {
    let address = serve("<!doctype html><button style='width:4000px;height:4000px' onclick=\"this.dataset.clicks=String(Number(this.dataset.clicks||0)+1)\">Huge target</button>".into());
    on_page(address, async |session, _| {
        for _ in 0..3 {
            let clicked = session.execute("click", json!({"ref":"Huge target"})).await;
            assert!(clicked.ok, "{}", clicked.output);
        }
        let tab = session.current().await.unwrap();
        assert_eq!(
            session
                .document
                .world
                .evaluate(&tab, "document.querySelector('button').dataset.clicks")
                .await
                .unwrap(),
            "3"
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn unavailable_viewport_does_not_turn_into_guessed_coordinates() {
    let address =
        serve("<!doctype html><button onclick=\"this.dataset.clicked='yes'\">Target</button>".into());
    on_page(address, async |session, _| {
        let tab = session.current().await.unwrap();
        session.document.world.evaluate(&tab, "Object.defineProperty(window,'innerWidth',{get(){throw Error('fixture viewport unavailable')}}); true").await.unwrap();
        let clicked = session.execute("click", json!({"ref":"Target"})).await;
        assert!(!clicked.ok && clicked.output.contains("viewport unavailable"), "{}", clicked.output);
        assert_eq!(session.document.world.evaluate(&tab, "document.querySelector('button').hasAttribute('data-clicked')").await.unwrap(), false);
    })
    .await;
}

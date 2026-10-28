//! Content only pixels carry: a canvas is waited for until it stops drawing,
//! a region of it can be enlarged, and the text it draws can be recorded.

use super::*;

/// Width and height of a PNG, from its header.
fn png_size(data: &str) -> (u32, u32) {
    let bytes = crate::text::base64::unbase64(data).unwrap();
    let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (word(16), word(20))
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_waits_for_a_large_canvas_to_finish_drawing() {
    let address = serve(
        "<!doctype html><title>t</title><button>Draw</button>
         <canvas id=c width=800 height=500 style='width:100vw;height:90vh'></canvas>
         <script>document.querySelector('button').onclick = () => {
           const g = c.getContext('2d'), start = performance.now();
           const step = (now) => { const done = Math.min(1, (now - start) / 1200);
             g.fillStyle = 'red'; g.fillRect(0, 0, 800 * done, 500);
             if (done < 1) requestAnimationFrame(step); };
           requestAnimationFrame(step); };</script>"
            .into(),
    );
    on_page(address, async |session, _| {
        let drew = session.execute("click", json!({ "ref": "Draw" })).await;
        assert!(drew.ok, "{}", drew.output);
        let end = session
            .execute(
                "eval",
                json!({ "expression": "[...c.getContext('2d').getImageData(795, 250, 1, 1).data]" }),
            )
            .await;
        assert!(end.output.contains("[255,0,0,"), "the click returned mid-drawing: {}", end.output);
    })
    .await;
}

/// A region of the viewport is pictured enlarged, and said how its pixels map back.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_enlarges_a_region_of_the_viewport() {
    let address = serve("<!doctype html><title>t</title><p style='font-size:6px'>tiny print</p>".into());
    on_page(address, async |session, _| {
        let shot = session.execute("screenshot", json!({ "region": [10, 20, 200, 100], "zoom": 2 })).await;
        assert!(shot.ok && shot.output.contains("enlarged 2x"), "{}", shot.output);
        assert!(shot.output.contains("10+px/2, 20+py/2"), "{}", shot.output);
        assert_eq!(png_size(&shot.images[0].data), (400, 200));
        let whole = session.execute("screenshot", json!({ "zoom": 2 })).await;
        assert!(!whole.ok, "a viewport shot stays in click coordinates: {}", whole.output);
        let outside = session.execute("screenshot", json!({ "region": [5000, 5000, 10, 10] })).await;
        assert!(!outside.ok, "{}", outside.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_tells_the_text_a_canvas_draws_when_asked_to() {
    let page = "<!doctype html><title>t</title><button>Hover</button>
         <canvas id=c width=400 height=200 style='width:800px;height:400px;position:absolute;left:0;top:100px'></canvas>
         <script>const g = c.getContext('2d'); g.font = '20px sans-serif'; g.fillText('Paris', 50, 60);
           document.querySelector('button').onclick = () => g.fillText('42%', 300, 150);</script>";
    on_page_with(
        serve(page.into()),
        |settings| settings.canvas_text = true,
        async |session, opened| {
            assert!(opened.output.contains("\"Paris\" 100,220"), "{}", opened.output);
            let hovered = session.execute("click", json!({ "ref": "Hover" })).await;
            assert!(hovered.output.contains("\"42%\" 600,400"), "{}", hovered.output);
            assert!(!hovered.output.contains("Paris"), "only what is newly drawn: {}", hovered.output);
        },
    )
    .await;
    on_page(serve(page.into()), async |_, opened| {
        assert!(!opened.output.contains("Paris"), "off unless asked: {}", opened.output);
    })
    .await;
}

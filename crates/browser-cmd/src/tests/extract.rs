//! Collecting rows: sibling rows, tables, lists that virtualize
//! by scroll or by transform, and coverage that never claims more than it saw.

use super::*;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_extract_reaches_a_sibling_row() {
    // A title row and a detail row as siblings, no element wrapping the pair
    // — the shape Hacker News and many tables use.
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title><table><tbody>
        <tr class="head"><td><a href="/a" class="t">Alpha</a></td></tr>
        <tr class="sub"><td><span class="pts">110 points</span> <a class="who">ada</a></td></tr>
        <tr class="head"><td><a href="/b" class="t">Beta</a></td></tr>
        <tr class="sub"><td><span class="pts">36 points</span> <a class="who">ben</a></td></tr>
        </tbody></table>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let rows = session
            .execute(
                "extract",
                json!({ "each": "tr.head", "fields": {
                    "title": ".t", "url": ".t@href", "points": "+ .pts", "who": "+ .who"
                } }),
            )
            .await;
        assert!(rows.ok, "{}", rows.output);
        assert!(rows.output.contains("\"title\":\"Alpha\""), "{}", rows.output);
        assert!(rows.output.contains("\"points\":\"110 points\""), "sibling not reached: {}", rows.output);
        assert!(rows.output.contains("\"who\":\"ada\""), "{}", rows.output);
        assert!(rows.output.contains("\"points\":\"36 points\""), "{}", rows.output);
        assert!(rows.output.contains("\"matched\":2"), "{}", rows.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_extract_reads_a_data_table() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <table><thead><tr><th>Name</th><th>Price</th></tr></thead>
        <tbody>
          <tr><td><a href="/p/1">Widget</a></td><td>$3</td></tr>
          <tr><td><a href="/p/2">Gadget</a></td><td>$5</td></tr>
        </tbody></table>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let rows = session
            .execute("extract", json!({ "each": "tbody tr", "fields": { "name": "td a", "price": "td:nth-child(2)", "url": "a@href" } }))
            .await;
        assert!(rows.ok, "{}", rows.output);
        assert!(rows.output.contains("Widget") && rows.output.contains("Gadget"), "rows missing: {}", rows.output);
        assert!(rows.output.contains("$3") && rows.output.contains("$5"), "prices missing: {}", rows.output);
        assert!(rows.output.contains("/p/1") && rows.output.contains("/p/2"), "hrefs missing: {}", rows.output);
        assert!(!rows.output.contains("\"Name\""), "the header row was not skipped: {}", rows.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_extract_all_gathers_a_virtualized_list() {
    // A hundred rows in a scrollable box that only renders the dozen on screen,
    // recycling them as it scrolls — a real virtualized list on native scroll.
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div id=box style="height:300px;overflow:auto;border:1px solid"><div id=pad style="position:relative"></div></div>
        <script>
          const N = 100, H = 30;
          pad.style.height = (N * H) + 'px';
          function render() {
            const start = Math.floor(box.scrollTop / H), vis = Math.ceil(300 / H) + 2;
            pad.innerHTML = '';
            for (let i = start; i < Math.min(N, start + vis); i++) {
              const d = document.createElement('div');
              d.className = 'vrow'; d.style.position = 'absolute'; d.style.top = (i * H) + 'px';
              d.textContent = 'Item ' + i; pad.appendChild(d);
            }
          }
          box.addEventListener('scroll', render); render();
        </script>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        // Only the dozen on screen exist at once.
        let dom = session
            .execute("eval", json!({ "expression": "document.querySelectorAll('.vrow').length" }))
            .await;
        assert!(!dom.output.contains("100"), "the list was not virtualized: {}", dom.output);
        let all = session
            .execute(
                "extract",
                json!({ "each": ".vrow", "fields": { "text": "." }, "all": true, "limit": 500 }),
            )
            .await;
        assert!(all.ok, "{}", all.output);
        assert!(all.output.contains("\"Item 0\""), "the first row is missing: {}", all.output);
        assert!(all.output.contains("\"Item 99\""), "the last row was never scrolled to: {}", all.output);
        assert!(all.output.contains("\"collected\":100"), "not every row was gathered: {}", all.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_extract_all_gathers_a_transform_grid() {
    // The box does not scroll (overflow hidden); a wheel handler moves an
    // offset and re-renders the dozen visible rows — the shape of ag-grid and
    // other JS-driven grids, where scrollTop stays 0 the whole way down.
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <div id=box style="height:300px;overflow:hidden;border:1px solid"><div id=pad></div></div>
        <script>
          const N = 80, H = 30, VIS = 12; let off = 0;
          function render() {
            const start = Math.floor(off / H);
            pad.innerHTML = '';
            for (let i = start; i < Math.min(N, start + VIS); i++) {
              const d = document.createElement('div');
              d.className = 'trow'; d.style.height = H + 'px'; d.textContent = 'Item ' + i; pad.appendChild(d);
            }
          }
          box.addEventListener('wheel', (e) => { e.preventDefault(); off = Math.max(0, Math.min(off + e.deltaY, (N - VIS) * H)); render(); }, { passive: false });
          render();
        </script>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let all = session
            .execute("extract", json!({ "each": ".trow", "fields": { "text": "." }, "all": true, "identity": ".", "limit": 500 }))
            .await;
        assert!(all.ok, "{}", all.output);
        assert!(all.output.contains("\"Item 0\""), "the first row is missing: {}", all.output);
        assert!(
            all.output.contains("\"Item 40\""),
            "a middle row was skipped by too big a step: {}",
            all.output
        );
        assert!(
            all.output.contains("\"Item 79\""),
            "the transform grid never reached its end: {}",
            all.output
        );
        assert!(
            all.output.contains("\"collected\":80"),
            "not every row of the transform grid was gathered: {}",
            all.output
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn extraction_preserves_equal_rows_and_reports_traversal_limits() {
    let address =
        serve("<!doctype html><div class=row data-id=a>same</div><div class=row data-id=b>same</div>".into());
    fn parsed(output: &str) -> Value {
        serde_json::from_str(
            output.strip_prefix("<browser trusted=\"no\">\n").unwrap().strip_suffix("\n</browser>").unwrap(),
        )
        .unwrap()
    }
    on_page(address, async |handler, _| {
        for options in [json!({}), json!({"all":true,"identity":".@data-id"})] {
            let mut args = json!({"each":".row","fields":{"name":"."},"limit":10});
            args.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
            let result = handler.execute("extract", args).await;
            assert!(result.ok, "{}", result.output);
            let data = parsed(&result.output);
            assert_eq!(data["rows"], json!([{"name":"same"},{"name":"same"}]));
            if options.get("all").is_some() {
                assert_eq!(data["coverage"]["complete"], false);
            }
        }
        // An identity the caller chose badly, or rows whose values cannot
        // tell them apart, is reported in coverage, with the rows still
        // returned: a list worth reading is not thrown away over it, and
        // completeness is not claimed.
        for identity in [json!({"identity":"."}), json!({})] {
            let mut args = json!({"each":".row","fields":{"name":"."},"all":true});
            args.as_object_mut().unwrap().extend(identity.as_object().unwrap().clone());
            let repeated = handler.execute("extract", args).await;
            assert!(repeated.ok, "{}", repeated.output);
            let data = parsed(&repeated.output);
            assert_eq!(data["rows"], json!([{"name":"same"}]));
            assert_eq!(data["coverage"]["complete"], false);
            assert!(
                data["coverage"]["identity_anomalies"]["identities_repeated_in_one_batch"]
                    .as_u64()
                    .is_some_and(|count| count > 0),
                "{}",
                repeated.output
            );
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn extraction_steps_fit_a_small_virtual_viewport() {
    let address=serve(r#"<!doctype html><div id=box style="height:60px;overflow:auto;width:200px"><div id=pad style="height:600px;position:relative"></div></div>
    <script>function draw(){pad.innerHTML='';let start=Math.floor(box.scrollTop/30);for(let i=start;i<Math.min(20,start+2);i++){let d=document.createElement('div');d.className='row';d.dataset.id=i;d.style='position:absolute;height:30px;top:'+i*30+'px';d.textContent='same';pad.appendChild(d)}}box.addEventListener('scroll',draw);draw()</script>"#.into());
    on_page(address, async |handler, _| {
        let result = handler
            .execute("extract", json!({"each":".row","fields":{"text":".","id":".@data-id"},"all":true,"identity":".@data-id"}))
            .await;
        assert!(result.ok, "{}", result.output);
        let body = result
            .output
            .strip_prefix("<browser trusted=\"no\">\n")
            .unwrap()
            .strip_suffix("\n</browser>")
            .unwrap();
        let data: Value = serde_json::from_str(body).unwrap();
        assert_eq!(data["rows"].as_array().unwrap().len(), 20, "{}", result.output);
        assert_eq!(data["coverage"]["complete"], false);
    })
    .await;
}

//! What a page can learn about the browser: a consistent fingerprint, the
//! native language and client hints in pages and workers, a screen that agrees
//! with itself, and helpers that leave no mark in the main world.

use super::*;

/// Measured on the browser as configured, headless disguise included.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_presents_a_consistent_fingerprint() {
    let address = serve("<!doctype html><meta charset=utf-8><title>t</title><h1>fingerprint</h1>".to_owned());
    on_page(address, async |session, _| {
        // The page lists every inconsistency it can find in itself; a real
        // browser finds none, so anything here is the disguise slipping.
        let probe = r#"(async () => {
                const p = [];
                const ua = navigator.userAgent;
                const d = navigator.userAgentData;
                const h = await d.getHighEntropyValues(['platformVersion', 'architecture', 'bitness']);
                if (navigator.webdriver) p.push('webdriver-true');
                if (/Headless/.test(ua)) p.push('headless-in-ua');
                if (d.brands.some(b => /Headless/.test(b.brand))) p.push('headless-in-brands');
                // A worker sees the process-level user agent, which a per-frame
                // override would miss — the leak CreepJS reads a worker to catch.
                const workerUa = await new Promise(res => {
                    try {
                        const url = URL.createObjectURL(new Blob(['postMessage(navigator.userAgent)']));
                        const w = new Worker(url);
                        w.onmessage = e => res(e.data);
                        setTimeout(() => res(ua), 2000);
                    } catch (e) { res(ua); }
                });
                if (/Headless/.test(workerUa)) p.push('headless-in-worker');
                if (workerUa !== ua) p.push('worker-ua-mismatch');
                const uaMajor = (ua.match(/Chrome\/(\d+)/) || [])[1];
                const brand = (d.brands.find(b => b.brand === 'Google Chrome') || {}).version;
                if (uaMajor && brand && uaMajor !== brand) p.push('brand-version-mismatch');
                if (/Macintosh/.test(ua) && navigator.platform !== 'MacIntel') p.push('mac-platform-mismatch');
                if (/Windows/.test(ua) && !/Win/.test(navigator.platform)) p.push('win-platform-mismatch');
                if (navigator.languages[0] !== navigator.language) p.push('language-mismatch');
                if ((/Macintosh|Windows/.test(ua)) && !h.platformVersion) p.push('empty-platform-version');
                if (h.architecture === 'arm' && !/Mac|Windows|Linux/.test(ua)) p.push('arch-without-os');
                const gl = document.createElement('canvas').getContext('webgl');
                const dbg = gl && gl.getExtension('WEBGL_debug_renderer_info');
                if (!dbg || !gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL)) p.push('no-webgl-renderer');
                return JSON.stringify(p);
            })()"#;
        let read = session.execute("eval", json!({ "expression": probe })).await;
        assert!(read.ok, "{}", read.output);
        assert!(
            read.output.contains("[]"),
            "the fingerprint contradicts itself: {}",
            read.output
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Chrome; also opens an isolated headed window when a display is available"]
async fn native_language_agrees_in_requests_pages_and_workers() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let language =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<String, String>::new()));
    let received = language.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut bytes = [0_u8; 8192];
            let n = std::io::Read::read(&mut stream, &mut bytes).unwrap_or(0);
            let headers = String::from_utf8_lossy(&bytes[..n])
                .lines()
                .filter_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    Some((name.to_ascii_lowercase(), value.trim().to_owned()))
                })
                .collect();
            *received.lock().unwrap() = headers;
            let body = "<!doctype html><button>Language probe</button>";
            let _=std::io::Write::write_all(&mut stream,format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nAccept-CH: Sec-CH-UA-Full-Version-List\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes());
        }
    });
    for headless in [true, false] {
        if !headless && !headed_allowed() {
            continue;
        }
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path(), temp.path());
        settings.headless = headless;
        settings.language = "fr-FR,fr".into();
        let host = Host::new(settings);
        tokio::task::LocalSet::new().run_until(async {
            let mut session = host.session();
            let opened = session.execute("navigate", json!({"url":format!("http://{address}")})).await;
            assert!(opened.ok, "{}", opened.output);
            let tab = session.current().await.unwrap();
            let values = session.document.world.evaluate(&tab, r#"(async()=>{
                const worker=await new Promise((resolve,reject)=>{
                    const url=URL.createObjectURL(new Blob(['postMessage({language:navigator.language,languages:navigator.languages})']));
                    const w=new Worker(url);
                    const done=()=>{clearTimeout(timer);w.terminate();URL.revokeObjectURL(url)};
                    const timer=setTimeout(()=>{done();reject(Error('worker did not answer'))},5000);
                    w.onmessage=e=>{done();resolve(e.data)};w.onerror=e=>{done();reject(Error(e.message))};
                });return {language:navigator.language,languages:navigator.languages,worker};
            })()"#).await.unwrap();
            assert_eq!(values["language"], "fr-FR", "headless={headless}: {values}");
            assert_eq!(values["worker"]["language"], values["language"]);
            assert_eq!(values["worker"]["languages"], values["languages"]);
            session.document.world.evaluate(&tab, "fetch('/probe').then(r => r.text()).then(() => true)").await.unwrap();
            let actual = tab.cdp.call(None, "Browser.getVersion", json!({})).await.unwrap();
            let full = actual["product"].as_str().unwrap().split_once('/').unwrap().1;
            {
                let headers = language.lock().unwrap();
                assert!(headers["accept-language"].starts_with("fr-FR"), "headless={headless}: {headers:?}");
                assert!(!headers["user-agent"].contains("Headless"), "{headers:?}");
                assert!(headers["sec-ch-ua-full-version-list"].contains(full), "{headers:?}");
            }
            host.shutdown().await;
        }).await;
    }
}

#[tokio::test]
#[ignore = "requires Chrome; native identity diagnostic"]
async fn headless_identity_preserves_native_hints_in_page_and_worker() {
    let address = serve("<!doctype html><button>Native identity</button>".into());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    tokio::task::LocalSet::new().run_until(async {
        let (cdp, _child) = crate::chrome::launch::open(&settings, None).await.unwrap();
        let cdp = std::rc::Rc::new(cdp);
        let tab = crate::chrome::tab::new_tab(&cdp, None, false).await.unwrap();
        tab.call("Page.enable", json!({})).await.unwrap();
        let mut world = World::new();
        let probe = r#"(async()=>{
            const keys=['fullVersionList','platformVersion','architecture','bitness','wow64','model','uaFullVersion','formFactors'];
            const worker=isSecureContext ? await new Promise((resolve,reject)=>{
                const script=`(async()=>postMessage({ua:navigator.userAgent,data:navigator.userAgentData ? await navigator.userAgentData.getHighEntropyValues(${JSON.stringify(keys)}) : null}))()`;
                const url=URL.createObjectURL(new Blob([script]));const w=new Worker(url);
                const timer=setTimeout(()=>{w.terminate();URL.revokeObjectURL(url);reject(Error('worker timeout'))},5000);
                w.onmessage=e=>{clearTimeout(timer);w.terminate();URL.revokeObjectURL(url);resolve(e.data)};
                w.onerror=e=>{clearTimeout(timer);w.terminate();URL.revokeObjectURL(url);reject(Error(e.message))};
            }) : null;
            return {secure:isSecureContext,ua:navigator.userAgent,data:navigator.userAgentData ? await navigator.userAgentData.getHighEntropyValues(keys) : null,worker};
        })()"#;
        let blank = world.evaluate(&tab, probe).await.unwrap();
        tab.call("Page.navigate", json!({"url":format!("http://{address}")})).await.unwrap();
        crate::page::settle::settle(&mut world, &tab, &settings.waits).await;
        let native = world.evaluate(&tab, probe).await.unwrap();
        let targets_before = cdp.call(None, "Target.getTargets", json!({})).await.unwrap();
        let identity = crate::browser::identity::identity(&cdp, &settings).await.unwrap();
        let targets_after = cdp.call(None, "Target.getTargets", json!({})).await.unwrap();
        let ids = |value: &Value| value["targetInfos"].as_array().unwrap().iter().filter(|target| target["type"] == "page").filter_map(|target| target["targetId"].as_str().map(str::to_owned)).collect::<std::collections::HashSet<_>>();
        assert_eq!(ids(&targets_before), ids(&targets_after), "identity probe leaked a target");
        crate::browser::prepare::prepare(&settings, &tab, Some(&identity)).await.unwrap();
        let prepared = world.evaluate(&tab, probe).await.unwrap();
        println!("blank={blank}\nnative={native}\nprepared={prepared}");
        assert_eq!(prepared["data"], native["data"]);
        assert_eq!(prepared["worker"]["data"], prepared["data"]);
        assert_eq!(prepared["worker"]["ua"], prepared["ua"]);
        assert!(!prepared["ua"].as_str().unwrap().contains("Headless"));
        let _ = cdp.call(None, "Browser.close", json!({})).await;
    }).await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_headless_retina_screen_agrees_with_itself_and_its_pictures_are_css_pixels() {
    let address = serve("<!doctype html><body style='height:4000px'><button>Deep</button>".into());
    let screen = crate::chrome::launch::arguments::SCREEN;
    on_page(address, async |session, _| {
        let seen = session
            .execute("eval", json!({"expression":
                "[screen.width, screen.height, screen.availTop, screen.availHeight, devicePixelRatio, innerWidth, innerHeight].join(' ')"}))
            .await;
        let tab = session.current().await.unwrap();
        let viewport = crate::page::view::viewport(&mut session.document.world, &tab).await.unwrap();
        let expected = format!(
            "{} {} {} {} {} {} {}",
            screen.size.0,
            screen.size.1,
            screen.menubar,
            screen.size.1 - screen.menubar - screen.taskbar,
            screen.scale,
            viewport.0,
            viewport.1
        );
        assert!(seen.output.contains(&expected), "{}", seen.output);
        assert!(session.execute("scroll", json!({"direction":"down"})).await.ok);
        let shot = session.execute("screenshot", json!({})).await;
        assert!(shot.ok, "{}", shot.output);
        let png = std::fs::read(session.settings().screenshots.join("browser-0001.png")).unwrap();
        let size = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as f64;
        assert_eq!((size(16), size(20)), viewport, "a point read off the picture must be a point to click");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn node_lookup_and_cover_checks_do_not_write_dom_markers() {
    let address = serve(r#"<!doctype html><button id='covered'>Covered target</button>
        <div id='overlay' style='position:fixed;inset:0;background:white;z-index:100'>Overlay</div>
        <div data-browser-cmd-hit='site-owned'>Sentinel</div>
        <div id='custom' style='cursor:pointer' onclick='window.clicked=true'>Custom control</div>
        <script>window.markerWrites=0;window.clicked=false;
        new MutationObserver(records=>{markerWrites+=records.filter(r=>r.attributeName==='data-browser-cmd-hit').length})
          .observe(document.documentElement,{attributes:true,subtree:true});</script>"#.into());
    on_page(address, async |session, _| {
        let blocked = session.execute("click", json!({"ref":"Covered target"})).await;
        assert!(!blocked.ok && blocked.output.contains("Overlay"), "{}", blocked.output);
        session.execute("eval", json!({"expression":"document.querySelector('#overlay').remove()"})).await;
        let clicked = session.execute("click", json!({"ref":"Custom control"})).await;
        assert!(clicked.ok, "{}", clicked.output);
        let tab = session.current().await.unwrap();
        let probe = tab.call("Runtime.evaluate", json!({"expression":"({writes:markerWrites,clicked,owned:document.querySelector('[data-browser-cmd-hit]').getAttribute('data-browser-cmd-hit')})","returnByValue":true})).await.unwrap();
        assert_eq!(probe["result"]["value"]["writes"], 0);
        assert_eq!(probe["result"]["value"]["clicked"], true);
        assert_eq!(probe["result"]["value"]["owned"], "site-owned");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn script_errors_do_not_replay_side_effects_and_setup_avoids_main_world_hooks() {
    let address = serve(r#"<!doctype html><button>Ready</button><script>
      window.probeHits=0;
      const create=Document.prototype.createElement;
      Document.prototype.createElement=function(...args){probeHits++;return create.apply(this,args)};
      const data=Object.getOwnPropertyDescriptor(Navigator.prototype,'userAgentData');
      Object.defineProperty(Navigator.prototype,'userAgentData',{get(){probeHits++;return data.get.call(this)}});
    </script>"#.into());
    on_page(address, async |session, _| {
        let failed = session.execute("eval", json!({"expression":"document.documentElement.dataset.runs=String(Number(document.documentElement.dataset.runs||0)+1); throw Error('context failure after effect')"})).await;
        assert!(!failed.ok);
        let tab = session.current().await.unwrap();
        let shared = session.host.shared().await.unwrap();
        crate::browser::prepare::prepare(session.settings(), &tab, shared.identity.as_ref()).await.unwrap();
        let probe = tab.call("Runtime.evaluate", json!({"expression":"({runs:document.documentElement.dataset.runs,hooks:probeHits})","returnByValue":true})).await.unwrap();
        assert_eq!(probe["result"]["value"]["runs"], "1");
        assert_eq!(probe["result"]["value"]["hooks"], 0);
        let actual = tab.cdp.call(None, "Browser.getVersion", json!({})).await.unwrap();
        let hints = session.document.world.evaluate(&tab, "navigator.userAgentData.getHighEntropyValues(['fullVersionList'])").await.unwrap();
        let full = actual["product"].as_str().unwrap().split_once('/').unwrap().1;
        assert!(hints["fullVersionList"].as_array().unwrap().iter().any(|brand| brand["brand"] == "Google Chrome" && brand["version"] == full));
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn caller_globals_cannot_poison_helpers_or_another_session() {
    let address =
        serve("<!doctype html><input aria-label='Field' value='Old'><p id='status'>Initial</p>".into());
    on_page(address, async |a, _| {
        let setup = a.execute("eval", json!({"expression":"document.getElementById('status').textContent='Changed'; globalThis.answer=42; globalThis.__quiet={at:Infinity}; Object.defineProperty(document,'body',{get(){throw Error('caller body hook')}}); Document.prototype.querySelector=function(){throw Error('caller selector hook')}; answer"})).await;
        assert!(setup.ok, "{}", setup.output);
        let read = a.execute("text", json!({})).await;
        assert!(read.ok && read.output.contains("Changed"), "{}", read.output);
        let typed = a.execute("type", json!({"ref":"Field","text":"New"})).await;
        assert!(typed.ok, "{}", typed.output);
        let persisted = a.execute("eval", json!({"expression":"answer"})).await;
        assert!(persisted.ok && persisted.output.contains("42"), "{}", persisted.output);
        let mut b = a.host.session();
        let given = a.execute("tab", json!({"action":"give","id":"t1","to":b.book.name})).await;
        assert!(given.ok, "{}", given.output);
        let switched = b.execute("tab", json!({"action":"switch","id":"t1"})).await;
        assert!(switched.ok, "{}", switched.output);
        let isolated = b.execute("eval", json!({"expression":"({answer:typeof answer,value:document.querySelector('input').value})"})).await;
        assert!(isolated.ok && isolated.output.contains("undefined") && isolated.output.contains("New"), "{}", isolated.output);
    })
    .await;
}

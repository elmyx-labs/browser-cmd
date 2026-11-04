//! Downloads: reported to the tab that started them at the path Chrome
//! chose, kept apart between tabs and contexts, and an unusable directory
//! refused.

use super::*;

/// The path is the one Chrome reports, not one inferred from the file's name.
#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_reports_a_download_by_path() {
    let address = serve(
        r#"<!doctype html><meta charset=utf-8><title>t</title>
        <a id=d download="report.csv" href="data:text/csv,name,price%0AWidget,3%0AGadget,5">Download CSV</a>"#
            .to_owned(),
    );
    on_page(address, async |session, _| {
        let downloads = session.settings().downloads.clone();
        let click = session.execute("click", json!({ "ref": "Download CSV" })).await;
        assert!(click.ok, "{}", click.output);
        // A completed download is reported to the originating tab.
        let after = session.execute("wait", json!({ "seconds": 1.5 })).await;
        assert!(
            format!("{}\n{}", click.output, after.output)
                .contains(&serde_json::to_string(&downloads.join("report.csv")).unwrap()),
            "the download note did not give the path: {}",
            after.output
        );
        let saved = std::fs::read(downloads.join("report.csv")).expect("the file is on disk");
        assert!(
            saved.starts_with(b"name,price"),
            "the file content is wrong: {:?}",
            &saved[..saved.len().min(10)]
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn real_chrome_reports_a_download_and_marks_what_is_new() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0_u8; 8192];
            let read = std::io::Read::read(&mut stream, &mut request).unwrap_or_default();
            let asked = String::from_utf8_lossy(&request[..read]).into_owned();
            let (headers, body): (&str, &[u8]) = if asked.contains("/report.csv") {
                (
                    "Content-Type: text/csv\r\nContent-Disposition: attachment; filename=\"report.csv\"",
                    b"a,b\n1,2\n",
                )
            } else {
                (
                    "Content-Type: text/html",
                    b"<!doctype html><title>t</title><h1>Files</h1>
                <a href=\"/report.csv\" download>Download report</a>
                <button onclick=\"more.hidden=false\">Show more</button>
                <div id=\"more\" hidden><button>Archive</button><button>Share</button></div>",
                )
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{headers}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, head.as_bytes());
            let _ = std::io::Write::write_all(&mut stream, body);
        }
    });
    on_page(address, async |session, opened| {
        let downloads = session.settings().downloads.clone();
        assert!(!opened.output.contains("[new]"), "a fresh document is not all new: {}", opened.output);

        let shown = session.execute("click", json!({ "ref": "Show more" })).await;
        assert!(shown.ok, "{}", shown.output);
        assert!(
            shown.output.lines().any(|line| line.contains("\"Archive\" @e") && line.contains("[new]")),
            "what the click added is not marked: {}",
            shown.output
        );
        assert!(
            !shown.output.lines().any(|line| line.contains("\"Show more\" @e") && line.contains("[new]")),
            "what was already there is marked: {}",
            shown.output
        );

        let fetched = session.execute("click", json!({ "ref": "Download report" })).await;
        assert!(fetched.ok, "{}", fetched.output);
        let mut told = fetched.output.clone();
        for round in 0..20 {
            if told.contains("Chrome reported download completed:") && told.contains("report.csv") {
                break;
            }
            assert!(round < 19, "the download was never reported: {told}");
            told = session.execute("wait", json!({ "seconds": 0.5 })).await.output;
        }
        let reported: Value = serde_json::from_str(
            told.lines()
                .find_map(|line| line.strip_prefix("Chrome reported download completed: "))
                .expect("completed download record"),
        )
        .unwrap();
        assert_eq!(reported["receivedBytes"], 8);
        assert_eq!(reported["browserPath"], downloads.join("report.csv").to_str().unwrap());
        let saved = std::fs::read(downloads.join("report.csv")).unwrap();
        assert_eq!(saved, b"a,b\n1,2\n");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn download_paths_follow_browser_renaming_and_preserve_identically_named_files() {
    let address = serve("<!doctype html><a id='d' download='same.csv'>Download</a>".into());
    on_page(address, async |session, _| {
        let directory = session.settings().downloads.clone();
        let tab = session.current().await.unwrap();
        // A real browser policy deliberately chooses a name unrelated to the suggestion.
        // This also models the GUID staging needed for concurrent session file routing.
        tab.cdp.call(None, "Browser.setDownloadBehavior", json!({
            "behavior":"allowAndName", "downloadPath":directory, "eventsEnabled":true
        })).await.unwrap();
        let mut paths = Vec::new();
        for payload in ["first", "second"] {
            session.document.world.evaluate(&tab, &format!(
                "document.getElementById('d').href='data:text/csv,{payload}'; document.getElementById('d').click(); true"
            )).await.unwrap();
            let record = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let response = session.execute("wait", json!({"seconds":0})).await;
                    assert!(response.ok, "{}", response.output);
                    if let Some(record) = response.output.lines().find_map(|line| line.strip_prefix("Chrome reported download completed: ")) {
                        break serde_json::from_str::<Value>(record).unwrap();
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }).await.expect("download completion was not reported");
            assert_eq!(record["suggestedFilename"], "same.csv");
            let path = std::path::PathBuf::from(record["browserPath"].as_str().expect("tested Chrome must report its download path"));
            assert_ne!(path.file_name().unwrap(), "same.csv");
            assert_eq!(std::fs::read(&path).unwrap(), payload.as_bytes());
            paths.push(path);
        }
        assert_ne!(paths[0], paths[1]);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"first");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn concurrent_tabs_keep_download_notifications_and_console_logs_separate() {
    let address = serve(
        r#"<!doctype html><meta http-equiv="Content-Security-Policy" content="img-src 'none'">
        <title>Session events</title><a id="download" download="session-a.csv"
        href="data:text/csv,source%0Asession-a">Download</a>"#
            .into(),
    );
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let downloads = settings.downloads.clone();
    let host = Host::new(settings);
    tokio::task::LocalSet::new().run_until(async {
        let mut a = host.session();
        let mut b = host.session();
        for session in [&mut a, &mut b] {
            let result = session.execute("navigate", json!({"url":format!("http://{address}")})).await;
            assert!(result.ok, "{}", result.output);
        }
        let tab = a.current().await.unwrap();
        assert_ne!(tab.target, b.current().await.unwrap().target);
        a.document.world.evaluate(&tab, "document.getElementById('download').click(); new Image().src='https://session-a.invalid/blocked'; true").await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut saw_download = false;
            let mut saw_log = false;
            loop {
                // B asks first on every poll. It must neither receive nor consume A's events.
                let other = b.execute("wait", json!({"seconds":0})).await;
                assert!(other.ok, "{}", other.output);
                assert!(!other.output.contains("session-a.csv"), "other tab stole download: {}", other.output);
                let other_logs = b.execute("console", json!({})).await;
                assert!(other_logs.ok, "{}", other_logs.output);
                assert!(!other_logs.output.contains("session-a.invalid"), "other tab exposed console: {}", other_logs.output);
                let own = a.execute("wait", json!({"seconds":0})).await;
                assert!(own.ok, "{}", own.output);
                saw_download |= own.output.contains("session-a.csv");
                let own_logs = a.execute("console", json!({})).await;
                assert!(own_logs.ok, "{}", own_logs.output);
                saw_log |= own_logs.output.contains("session-a.invalid");
                if saw_download && saw_log { break; }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.expect("originating tab lost download or log events");
        assert_eq!(std::fs::read(downloads.join("session-a.csv")).unwrap(), b"source\nsession-a");
        host.shutdown().await;
    }).await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn clean_context_download_policy_does_not_overwrite_the_default_context_policy() {
    let address = serve(
        "<!doctype html><a id='d' download='clean.csv' href='data:text/csv,context-data'>Download</a>".into(),
    );
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let directory = settings.downloads.clone();
    let host = Host::new(settings);
    tokio::task::LocalSet::new().run_until(async {
        let mut normal = host.session();
        let mut clean = host.session();
        for handler in [&mut normal, &mut clean] {
            let result = handler.execute("navigate", json!({"url":format!("http://{address}")})).await;
            assert!(result.ok, "{}", result.output);
        }
        let shared = host.shared().await.unwrap();
        shared.cdp.call(None, "Browser.setDownloadBehavior", json!({"behavior":"deny","eventsEnabled":true})).await.unwrap();
        let retried = clean.execute("retry-clean", json!({})).await;
        assert!(retried.ok, "{}", retried.output);
        for (handler, filename) in [(&mut normal,"default.csv"),(&mut clean,"clean.csv")] {
            let tab = handler.current().await.unwrap();
            handler.document.world.evaluate(&tab, &format!("document.getElementById('d').download='{filename}'; document.getElementById('d').click(); true")).await.unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut denied = false;
            let mut completed = false;
            loop {
                let normal_result = normal.execute("wait", json!({"seconds":0})).await;
                assert!(normal_result.ok, "{}", normal_result.output);
                assert!(!normal_result.output.contains("Chrome reported download completed:"), "clean context changed default download policy: {}", normal_result.output);
                denied |= normal_result.output.contains("Chrome reported download canceled:");
                let clean_result = clean.execute("wait", json!({"seconds":0})).await;
                assert!(clean_result.ok, "{}", clean_result.output);
                completed |= clean_result.output.contains("Chrome reported download completed:");
                if denied && completed { break; }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.expect("did not observe denied default and completed clean-context downloads");
        assert_eq!(std::fs::read(directory.join("clean.csv")).unwrap(), b"context-data");
        assert!(!directory.join("default.csv").exists());
        host.shutdown().await;
    }).await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn unusable_download_directory_fails_tab_preparation_explicitly() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = settings(temp.path(), temp.path());
    settings.downloads = temp.path().join("not-a-directory");
    std::fs::write(&settings.downloads, b"existing file").unwrap();
    let host = Host::new(settings);
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut handler = host.session();
            let result = handler.current().await;
            assert!(
                matches!(result, Err(Error::Io(_))),
                "an unusable download directory was not reported as an I/O error"
            );
            assert!(handler.book.active.is_none());
            host.shutdown().await;
        })
        .await;
}

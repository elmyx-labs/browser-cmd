//! What public bot-detection panels report about this browser, asserted.
//! Off by default: it needs the network and installed Chrome.
//! `cargo test -p browser-cmd public_detection -- --ignored --nocapture`

use super::*;

/// A page that reports what it found, and what its report must say.
struct Panel {
    url: &'static str,
    /// Verdicts that must appear, so a silent or half-loaded page is a failure
    /// rather than a pass.
    required: &'static [&'static str],
    /// Anything whose presence is a detection.
    forbidden: &'static [&'static str],
}

const PANELS: &[Panel] = &[
    // The oldest and most quoted panel. Every row it decides is marked passed
    // or failed, so one failed row anywhere is the whole verdict.
    Panel { url: "https://bot.sannysoft.com/", required: &["missing (passed)"], forbidden: &["(failed)"] },
    // Built for the CDP leaks in particular: the `Runtime.enable` flag, the
    // main-world execution an isolated world avoids, and the init scripts an
    // automation library leaves behind. A red circle is a detection.
    Panel {
        url: "https://bot-detector.rebrowser.net/",
        required: &["No leak detected", "No webdriver presented", "No window.__pwInitScripts detected"],
        forbidden: &["\u{1f534}"],
    },
];

#[tokio::test]
#[ignore = "requires Chrome and visits public detection panels; run with --ignored"]
async fn public_detection_panels_do_not_catch_this() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let host = Host::new(settings);
            let mut against = Vec::new();
            for panel in PANELS {
                let mut session = host.session();
                let opened = session.execute("navigate", json!({ "url": panel.url })).await;
                assert!(opened.ok, "{}: {}", panel.url, opened.output);
                // These pages fill themselves in after load, and a verdict read
                // too early is the absence of a verdict.
                let _ = session.execute("wait", json!({ "seconds": 6 })).await;
                let read = session.execute("text", json!({})).await;
                assert!(read.ok, "{}: {}", panel.url, read.output);
                if std::env::var_os("BROWSER_CMD_TEST_DETECTION_DUMP").is_some() {
                    println!("{}", read.output);
                }
                // A window larger than the screen it is on describes no
                // machine, and headless Chrome reports one unless it is told
                // the size of its own virtual display.
                let measured = session
                    .execute(
                        "eval",
                        json!({ "expression": "[screen.width, screen.height, innerWidth, innerHeight]" }),
                    )
                    .await;
                assert!(measured.ok, "{}", measured.output);
                let sizes: Vec<i64> = measured
                    .output
                    .lines()
                    .find_map(|line| serde_json::from_str(line).ok())
                    .unwrap_or_default();
                assert_eq!(sizes.len(), 4, "{}", measured.output);
                assert!(
                    sizes[0] >= sizes[2] && sizes[1] >= sizes[3],
                    "the screen is smaller than the window on it: {sizes:?}"
                );
                let _ = session.execute("close", json!({})).await;

                let silent: Vec<_> =
                    panel.required.iter().filter(|want| !read.output.contains(**want)).collect();
                let caught: Vec<_> =
                    panel.forbidden.iter().filter(|bad| read.output.contains(**bad)).collect();
                println!(
                    "{:<42} {}",
                    panel.url,
                    match (silent.is_empty(), caught.is_empty()) {
                        (true, true) => "clean".to_owned(),
                        (false, _) => format!("no verdict for {silent:?}"),
                        (_, false) => format!("caught by {caught:?}"),
                    }
                );
                if !silent.is_empty() || !caught.is_empty() {
                    against.push((panel.url, silent, caught));
                }
            }
            assert!(against.is_empty(), "a detector reported this browser: {against:?}");
        })
        .await;
}

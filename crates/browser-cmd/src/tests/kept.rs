//! Kept tabs on a real browser: a navigation that would leave one opens a new
//! tab, `tab check` says what each tab shows, `tab tidy` closes the rest, a
//! kept tab lost with its page or the browser is reopened where it was kept,
//! never in another context than its own, and after a daemon restart a
//! session takes back its tabs still open and reopens the others on `tab restore`.

use super::*;

const PAGE: &str = r#"<!doctype html><meta charset=utf-8><title>Evidence</title>
    <h1>The price is 42 dollars</h1><a href="/next">Next</a>"#;

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn kept_tabs_are_not_navigated_away_and_check_and_tidy_tell_them_from_the_rest() {
    let address = serve(PAGE.to_owned());
    on_page(address, async |session, _| {
        let kept = session.execute("tab", json!({ "action": "keep", "label": "price" })).await;
        assert_eq!(kept.output, format!("Kept t1 at http://{address}/."));
        let away = session.execute("navigate", json!({ "url": format!("http://{address}/other") })).await;
        assert!(
            away.output.starts_with("Navigated. t1 is kept, so this opened in a new tab, t2."),
            "{}",
            away.output
        );
        let listed = session.execute("tab", json!({ "action": "list" })).await.output;
        assert!(listed.contains(&format!(" t1 kept \"price\" ok http://{address}/ — Evidence")), "{listed}");
        assert!(listed.contains(&format!("*t2 ok http://{address}/other — Evidence")), "{listed}");

        let blank = session.execute("tab", json!({ "action": "new" })).await;
        assert!(blank.output.starts_with("Opened tab t3."), "{}", blank.output);
        assert!(session.execute("tab", json!({ "action": "keep" })).await.ok);
        let refused = session.execute("tab", json!({ "action": "new", "url": "http://127.0.0.1:1/" })).await;
        assert!(!refused.ok, "{}", refused.output);
        let back = session.execute("tab", json!({ "action": "switch", "id": "t1" })).await;
        assert!(back.output.starts_with("Switched to t1."), "{}", back.output);
        let moved = session.execute("click", json!({ "ref": "Next" })).await;
        assert!(moved.output.contains("t1 (kept) left its kept page"), "{}", moved.output);

        let checked = session.execute("tab", json!({ "action": "check" })).await.output;
        assert!(checked.starts_with("2 kept tabs: 0 ok, 2 not (t1 moved, t3 blank)."), "{checked}");
        assert!(checked.contains(&format!("(moved from http://{address}/)")), "{checked}");
        assert!(checked.contains(" t4 error net::ERR_UNSAFE_PORT http://127.0.0.1:1/"), "{checked}");
        let shots: Vec<&str> =
            checked.split("[screenshot ").skip(1).filter_map(|rest| rest.split(']').next()).collect();
        assert_eq!(shots.len(), 2, "one picture per kept tab: {checked}");
        for shot in shots {
            assert!(session.settings().workspace.join(shot).exists(), "{shot}");
        }

        let tidied = session.execute("tab", json!({ "action": "tidy" })).await;
        assert_eq!(tidied.output, "Closed t2, t4; open and kept: t1, t3.");
        let listed = session.execute("tab", json!({ "action": "list" })).await.output;
        assert!(
            listed.contains("*t1 kept") && listed.contains(" t3 kept") && !listed.contains("t2"),
            "{listed}"
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn kept_tabs_come_back_after_their_page_or_the_browser_goes_and_after_a_daemon_restart_on_restore() {
    let address = serve(PAGE.to_owned());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    tokio::task::LocalSet::new()
        .run_until(async {
            let host = Host::new(settings.clone());
            let mut session = host.named_session("cli-kept");
            let url = format!("http://{address}/a");
            assert!(session.execute("navigate", json!({ "url": url })).await.ok);
            assert!(session.execute("tab", json!({ "action": "keep" })).await.ok);
            assert!(
                session
                    .execute("tab", json!({ "action": "new", "url": format!("http://{address}/b") }))
                    .await
                    .ok
            );

            let shared = host.shared().await.unwrap();
            let kept = session.book.registry.named("t1").unwrap().target.clone();
            shared.cdp.close_target(&kept).await.unwrap();
            let checked = session.execute("tab", json!({ "action": "check" })).await.output;
            assert!(checked.starts_with("1 kept tab: 0 ok, 1 not (t1 closed)."), "{checked}");
            assert!(checked.contains(" t1 closed (was kept)") && !checked.contains("Restored"), "{checked}");
            for (command, args) in [("tab", json!({ "action": "list" })), ("console", json!({}))] {
                let looked = session.execute(command, args).await.output;
                assert!(!looked.contains("Restored"), "{command} reopened a tab: {looked}");
            }
            let listed = session.execute("tab", json!({ "action": "list" })).await.output;
            assert!(listed.contains(&format!(" t1 kept closed {url}")), "{listed}");
            let open = crate::tabs::pages(&host).await.unwrap();
            assert!(!open.iter().any(|page| page["url"] == *url), "reopened by a look: {open:?}");
            let read = session.execute("snapshot", json!({})).await.output;
            assert!(read.contains("Restored kept tab t1 after its page closed or crashed."), "{read}");
            let listed = session.execute("tab", json!({ "action": "list" })).await.output;
            assert!(
                listed.contains("*t2 ok") && listed.contains(&format!(" t1 kept restored {url}")),
                "{listed}"
            );

            host.shutdown().await;
            let read = session.execute("snapshot", json!({})).await.output;
            assert!(read.contains("Restored kept tab t1 after the browser was replaced."), "{read}");
            let listed = session.execute("tab", json!({ "action": "list" })).await.output;
            assert!(
                listed.contains(&format!("*t1 kept restored {url}")) && !listed.contains("t2"),
                "{listed}"
            );
            let checked = session.execute("tab", json!({ "action": "check" })).await.output;
            assert!(checked.starts_with("1 kept tab: all ok."), "{checked}");
            host.shutdown().await;
            drop(session);

            let saved = std::fs::read_to_string(crate::tabs::store::path(&settings, "cli-kept")).unwrap();
            assert!(
                saved.starts_with("{\"version\":1,") && saved.contains(&format!("\"kept\":\"{url}\"")),
                "{saved}"
            );
            let again = Host::new(settings);
            let mut session = again.named_session("cli-kept");
            let listed = session.execute("tab", json!({ "action": "list" })).await.output;
            assert!(
                listed.contains("Kept tab t1 did not outlast an earlier run; `tab restore` reopens it."),
                "{listed}"
            );
            assert!(listed.contains(&format!(" t1 kept gone (earlier run) {url}")), "{listed}");
            assert!(!listed.contains("Restored"), "reopened by itself: {listed}");
            let checked = session.execute("tab", json!({ "action": "check" })).await.output;
            assert!(checked.starts_with("1 kept tab: 0 ok, 1 not (t1 gone (earlier run))."), "{checked}");
            let restored = session.execute("tab", json!({ "action": "restore" })).await;
            assert_eq!(restored.output, "Restored kept tab t1.");
            let listed = session.execute("tab", json!({ "action": "list" })).await.output;
            assert!(listed.contains(&format!("*t1 kept restored {url}")), "{listed}");
            let again_restored = session.execute("tab", json!({ "action": "restore", "id": "t1" })).await;
            assert!(
                !again_restored.ok && again_restored.output.contains("nothing to restore"),
                "{}",
                again_restored.output
            );
            let opened = session.execute("tab", json!({ "action": "new" })).await;
            assert!(
                opened.output.starts_with("Opened tab t3."),
                "a handle is never given twice: {}",
                opened.output
            );
            again.shutdown().await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_daemon_restarted_on_a_live_browser_takes_back_handles_the_active_tab_and_borrowed_ones() {
    let address = serve(PAGE.to_owned());
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    tokio::task::LocalSet::new()
        .run_until(async {
            let first = Host::new(settings.clone());
            let mut session = first.named_session("cli-live");
            assert!(session.execute("navigate", json!({ "url": format!("http://{address}/a") })).await.ok);
            assert!(session.execute("tab", json!({ "action": "keep" })).await.ok);
            let b = json!({ "action": "new", "url": format!("http://{address}/b") });
            assert!(session.execute("tab", b).await.ok);
            let shared = first.shared().await.unwrap();
            let theirs = json!({ "url": format!("http://{address}/theirs") });
            shared.cdp.call(None, "Target.createTarget", theirs).await.unwrap();
            assert!(
                session
                    .execute("tab", json!({ "action": "use", "id": "1" }))
                    .await
                    .output
                    .starts_with("Borrowed t3.")
            );
            assert!(session.execute("tab", json!({ "action": "switch", "id": "t2" })).await.ok);
            let targets: Vec<String> = ["t1", "t2", "t3"]
                .iter()
                .map(|t| session.book.registry.named(t).unwrap().target.clone())
                .collect();
            // The daemon goes; its Chrome, on the profile, stays and is picked up by the next.
            drop(session);

            let again = Host::new(settings);
            let mut session = again.named_session("cli-live");
            let listed = session.execute("tab", json!({ "action": "list" })).await.output;
            assert!(
                listed.contains(&format!(" t1 kept ok http://{address}/a"))
                    && listed.contains(&format!("*t2 ok http://{address}/b"))
                    && listed.contains(&format!(" t3 borrowed ok http://{address}/theirs")),
                "{listed}"
            );
            assert!(!listed.contains("Not any session's") && !listed.contains("earlier run"), "{listed}");
            let left = [targets[0].clone(), targets[2].clone()];
            assert_eq!(
                session.book.previous, left,
                "the order of the tabs left, as it was, not the active one"
            );
            let mut other = again.session();
            let seen = other.execute("tab", json!({ "action": "list" })).await.output;
            assert!(seen.contains("3 other tabs open, of session cli-live"), "{seen}");
            let refused = other.execute("tab", json!({ "action": "switch", "id": targets[1] })).await;
            assert!(refused.output.contains("session cli-live's tab"), "{}", refused.output);
            let back = session.execute("tab", json!({ "action": "close", "id": "t3" })).await.output;
            assert!(back.starts_with("Gave the borrowed tab back, open"), "{back}");
            assert!(shared.cdp.target_exists(&targets[2]).await.unwrap(), "a borrowed tab was closed");
            first.shutdown().await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_kept_tab_whose_own_context_is_lost_is_gone_and_never_reopened_in_the_profile() {
    let address = serve(PAGE.to_owned());
    on_page(address, async |session, _| {
        assert!(session.execute("retry-clean", json!({})).await.ok);
        assert!(
            session.book.registry.named("t1").unwrap().isolated,
            "a clean retry's tab is in its own context"
        );
        assert!(session.execute("tab", json!({ "action": "keep" })).await.ok);
        session.host.shutdown().await;
        let read = session.execute("snapshot", json!({})).await.output;
        assert!(read.contains("previous isolated context is unavailable"), "{read}");
        assert!(!read.contains("Restored"), "{read}");
        let listed = session.execute("tab", json!({ "action": "list" })).await.output;
        assert!(listed.contains(&format!(" t1 kept gone (context lost) http://{address}/")), "{listed}");
        let refused = session.execute("tab", json!({ "action": "restore", "id": "t1" })).await;
        assert!(!refused.ok && refused.output.contains("not reopened in another"), "{}", refused.output);
        let pages = crate::tabs::pages(&session.host).await.unwrap();
        assert!(
            pages.iter().all(|page| !page["url"].as_str().unwrap_or_default().contains(&address.to_string()))
        );
    })
    .await;
}

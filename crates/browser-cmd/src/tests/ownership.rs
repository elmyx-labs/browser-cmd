//! Who holds a tab, on a real browser: a person's Chrome attached to, whose
//! tabs are borrowed and never closed; two sessions that refuse each other's
//! tabs until one is handed over; a window opened late; a crashed tab; a kept
//! tab moved by somebody else.

use super::*;

const PAGE: &str = r#"<!doctype html><meta charset=utf-8><title>Fixture</title><h1>Fixture page</h1>
    <button onclick="setTimeout(() => window.open('/late', '_blank'), 1500)">Later</button>"#;

/// The id prefix `tab list` shows for the tab at `url` in its block of tabs nobody holds.
fn unowned_id(list: &str, url: &str) -> String {
    let line = list.lines().find(|line| line.contains(url)).unwrap_or_else(|| panic!("{url}: {list}"));
    line.split_whitespace().nth(1).unwrap().to_owned()
}

/// A page's id, attached anew outside any session, as a person's own tool would.
async fn outside(shared: &Shared, target: &str) -> Tab {
    crate::chrome::tab::attach(&shared.cdp, target.into()).await.unwrap()
}

async fn window(shared: &Shared, target: &str) -> Value {
    shared.cdp.call(None, "Browser.getWindowForTarget", json!({ "targetId": target })).await.unwrap()["windowId"]
        .clone()
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_persons_chrome_keeps_its_tabs_and_the_session_works_in_a_window_of_its_own() {
    let address = serve(PAGE.to_owned());
    let (mine, theirs) = (format!("http://{address}/mine"), format!("http://{address}/theirs"));
    let temp = tempfile::tempdir().unwrap();
    let person = settings(&temp.path().join("person"), temp.path());
    tokio::task::LocalSet::new()
        .run_until(async {
            let (cdp, _chrome) = crate::chrome::launch::open(&person, None).await.unwrap();
            for page in ["one", "two"] {
                let url = format!("http://{address}/{page}");
                cdp.call(None, "Target.createTarget", json!({ "url": url })).await.unwrap();
            }
            let mut attached = settings(temp.path(), temp.path());
            attached.attach = Some(cdp.endpoint().to_owned());
            let host = Host::new(attached);
            let mut session = host.session();

            let listed = session.execute("tab", json!({ "action": "list" })).await.output;
            assert!(listed.contains("Not any session's; `tab use N` borrows one:"), "{listed}");
            assert!(listed.contains("/one") && listed.contains("/two"), "{listed}");
            let one = unowned_id(&listed, "/one");

            let opened = session.execute("navigate", json!({ "url": mine })).await;
            assert!(opened.ok, "{}", opened.output);
            let shared = host.shared().await.unwrap();
            let own = session.book.active.as_ref().unwrap().target.clone();
            let pages = crate::tabs::pages(&host).await.unwrap();
            let person = pages.iter().find(|page| page["url"].as_str().unwrap().ends_with("/one")).unwrap();
            let person_tab = crate::tabs::target_id(person).to_owned();
            assert!(person_tab.starts_with(&one));
            assert_ne!(
                window(&shared, &own).await,
                window(&shared, &person_tab).await,
                "not in a window of its own"
            );

            let switched = session.execute("tab", json!({ "action": "switch", "id": person_tab })).await;
            assert!(
                switched.output.contains("`tab use"),
                "a person's tab taken implicitly: {}",
                switched.output
            );
            let used = session.execute("tab", json!({ "action": "use", "id": person_tab })).await;
            assert!(used.output.starts_with("Borrowed t2."), "{}", used.output);
            let listed = session.execute("tab", json!({ "action": "list" })).await.output;
            assert!(listed.contains("*t2 borrowed") && !listed.contains(&one), "{listed}");

            let away = session.execute("navigate", json!({ "url": theirs })).await;
            assert!(
                away.output.contains("t2 is borrowed, so this opened in a new tab, t3"),
                "{}",
                away.output
            );
            assert!(session.execute("tab", json!({ "action": "switch", "id": "t2" })).await.ok);
            let here = session.execute("navigate", json!({ "url": theirs, "here": true })).await;
            assert!(here.output.starts_with("Navigated."), "{}", here.output);
            assert_eq!(outside(&shared, &person_tab).await.url().await.unwrap(), theirs);

            assert!(session.execute("tab", json!({ "action": "keep", "id": "t1" })).await.ok);
            let tidied = session.execute("tab", json!({ "action": "tidy" })).await.output;
            assert_eq!(tidied, "Closed t3; open and kept: t1.", "the borrowed tab it is on stays");
            let back = session.execute("tab", json!({ "action": "close", "id": "t2" })).await.output;
            assert!(back.starts_with("Gave the borrowed tab back, open"), "{back}");
            assert!(shared.cdp.target_exists(&person_tab).await.unwrap(), "a borrowed tab was closed");
            let listed = session.execute("tab", json!({ "action": "list" })).await.output;
            assert!(listed.contains(&one[..8]) && !listed.contains(" t2"), "given back: {listed}");
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_session_refuses_anothers_tab_by_name_until_it_is_handed_over() {
    let address = serve(PAGE.to_owned());
    let temp = tempfile::tempdir().unwrap();
    let host = Host::new(settings(temp.path(), temp.path()));
    tokio::task::LocalSet::new()
        .run_until(async {
            let (mut a, mut b) = (host.session(), host.session());
            assert!(a.execute("navigate", json!({ "url": format!("http://{address}/a") })).await.ok);
            assert!(b.execute("navigate", json!({ "url": format!("http://{address}/b") })).await.ok);
            let target = a.book.active.as_ref().unwrap().target.clone();
            let listed = b.execute("tab", json!({ "action": "list" })).await.output;
            assert!(listed.contains(&format!("1 other tab open, of session {}", a.book.name)), "{listed}");
            for action in ["switch", "use", "close"] {
                let refused = b.execute("tab", json!({ "action": action, "id": target })).await;
                assert!(
                    !refused.ok && refused.output.contains(&format!("session {}'s tab", a.book.name)),
                    "{}",
                    refused.output
                );
            }
            let given = a.execute("tab", json!({ "action": "give", "id": "t1", "to": b.book.name })).await;
            assert_eq!(given.output, format!("Gave t1 to session {}.", b.book.name));
            let taken = b.execute("tab", json!({ "action": "switch", "id": "t2" })).await;
            assert!(
                taken.output.contains("Tab t2 at") && taken.output.contains("was handed to this session"),
                "{}",
                taken.output
            );
            assert_eq!(b.book.active.as_ref().unwrap().target, target);
            let none = a.execute("tab", json!({ "action": "list" })).await.output;
            assert!(
                none.contains(&format!("of session {}", b.book.name)) && !none.contains(" t1 "),
                "{none}"
            );
            host.shutdown().await;
        })
        .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_window_opened_late_is_the_sessions_and_said_once_without_taking_the_active_tab() {
    let address = serve(PAGE.to_owned());
    on_page(address, async |session, _| {
        let clicked = session.execute("click", json!({ "ref": "Later" })).await;
        assert!(clicked.ok && !clicked.output.contains("A new tab"), "{}", clicked.output);
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        let listed = session.execute("tab", json!({ "action": "list" })).await.output;
        assert!(listed.contains(&format!("A new tab t2 opened at http://{address}/late.")), "{listed}");
        assert!(listed.contains("*t1 ") && listed.contains(" t2 "), "{listed}");
        let again = session.execute("tab", json!({ "action": "list" })).await.output;
        assert!(!again.contains("A new tab"), "said twice: {again}");
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_crashed_tab_is_said_and_reloaded_in_place_and_a_moved_kept_tab_shows_live() {
    let address = serve(PAGE.to_owned());
    on_page(address, async |session, _| {
        assert!(session.execute("tab", json!({ "action": "keep" })).await.ok);
        let shared = session.host.shared().await.unwrap();
        let target = session.book.active.as_ref().unwrap().target.clone();
        let other = outside(&shared, &target).await;
        let _ = other.send("Page.crash", json!({}));
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !crate::tabs::pages(&session.host).await.unwrap().iter().any(|page| page["crashed"] == true)
            {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the crash was not told");
        let checked = session.execute("tab", json!({ "action": "check" })).await.output;
        assert!(checked.starts_with("1 kept tab: 0 ok, 1 not (t1 crashed).\n"), "{checked}");
        assert!(checked.contains(" t1 crashed") && !checked.contains("reloaded"), "{checked}");
        let read = session.execute("snapshot", json!({})).await;
        assert!(read.output.contains("t1 crashed and was reloaded."), "{}", read.output);
        assert!(read.ok && read.output.contains("Fixture page"), "{}", read.output);
        assert_eq!(session.book.active.as_ref().unwrap().target, target, "the same tab, the same handle");
        let next = session.execute("navigate", json!({ "url": format!("http://{address}/") })).await;
        assert!(next.ok && !next.output.contains("The page crashed"), "reloaded, not gone: {}", next.output);

        let moved = format!("http://{address}/elsewhere");
        let other = outside(&shared, &target).await;
        other.call("Page.navigate", json!({ "url": moved })).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !crate::tabs::pages(&session.host).await.unwrap().iter().any(|page| page["url"] == *moved) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the navigation was not told");
        let listed = session.execute("tab", json!({ "action": "list" })).await.output;
        let line = listed.lines().find(|line| line.starts_with("*t1 kept ok ")).unwrap_or_default();
        assert!(
            line.contains(&moved) && line.ends_with(&format!("(moved from http://{address}/)")),
            "{listed}"
        );
    })
    .await;
}

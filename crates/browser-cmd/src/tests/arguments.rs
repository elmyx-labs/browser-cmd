//! What is refused, or answered, before any browser starts: arguments that
//! cannot be right fail without launching Chrome, and constructing a host or
//! asking for diagnostics without a tab leaves nothing on disk.

use super::*;

#[test]
fn construction_is_inert_until_a_browser_tool_is_called() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let settings = settings(temp.path(), &workspace);
    let profile = settings.profile.clone();
    let _handler = Host::new(settings).session();
    assert!(!profile.exists());
    assert!(!workspace.join("media").exists());
}

#[tokio::test]
async fn console_without_a_tab_does_not_start_a_browser() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let settings = settings(temp.path(), &workspace);
    let profile = settings.profile.clone();
    let mut session = Host::new(settings).session();
    for arguments in [json!({}), json!({"errors": true})] {
        let result = session.execute("console", arguments).await;
        assert!(result.ok, "{}", result.output);
        assert!(result.output.contains("No active tab"), "{}", result.output);
        assert!(session.book.active.is_none());
        assert!(!profile.exists(), "diagnostics created a browser profile");
        assert!(!workspace.join("media").exists());
    }
}

#[tokio::test]
async fn empty_conditions_cannot_pass_vacuously() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let profile = settings.profile.clone();
    let mut session = Host::new(settings).session();
    for (tool, args) in [
        ("verify", json!({"text":""})),
        ("verify", json!({"url":""})),
        ("verify", json!({"ref":" "})),
        ("wait", json!({"gone":""})),
    ] {
        let result = session.execute(tool, args).await;
        assert!(!result.ok && result.output.contains("empty"), "{}", result.output);
    }
    assert!(!profile.exists());
}

#[tokio::test]
async fn missing_required_arguments_and_misspelled_conditions_fail_before_any_browser_action() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let profile = settings.profile.clone();
    let mut session = Host::new(settings).session();
    for (tool, args) in [
        ("type", json!({})),
        ("click", json!({"ref":""})),
        ("hover", json!({})),
        ("drag", json!({"from":{"ref":"@e1"},"to":{"x":1}})),
        ("cdp", json!({"method":"Browser.close","params":[]})),
        ("navigate", json!({})),
        ("wait", json!([])),
        ("verify", json!({"url":"example.org","texxt":"required"})),
        ("wait", json!({"until":"ready"})),
        ("text", json!({"fnd":"phrase"})),
    ] {
        let result = session.execute(tool, args).await;
        assert!(!result.ok, "{tool}: {}", result.output);
    }
    assert!(!profile.exists(), "invalid arguments must not start the browser");
}

#[tokio::test]
async fn invalid_wait_duration_fails_before_launching_chrome() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let profile = settings.profile.clone();
    let mut session = Host::new(settings).session();
    let result = session.execute("wait", json!({"seconds":-1})).await;
    assert!(!result.ok && result.output.contains("finite and nonnegative"), "{}", result.output);
    assert!(!profile.exists());
}

#[tokio::test]
async fn extraction_rejects_ambiguous_traversal_before_browser_startup() {
    let temp = tempfile::tempdir().unwrap();
    let settings = settings(temp.path(), temp.path());
    let profile = settings.profile.clone();
    let mut handler = Host::new(settings).session();
    for extra in [
        json!({"all":true}),
        json!({"identity":".@id"}),
        json!({"limit":0}),
        json!({"each":" "}),
        json!({"all":true,"each":".row","next":"a"}),
    ] {
        let mut args = json!({"fields":{"name":"."}});
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let result = handler.execute("extract", args).await;
        assert!(!result.ok, "{}", result.output);
        assert!(!profile.exists());
    }
}

#[test]
fn text_phrase_and_options_are_both_parsed() {
    let command = crate::api::command::find("text").unwrap();
    let args = command.arguments(&["Alpha".into(), "max_bytes=900".into(), "context=false".into()]).unwrap();
    assert_eq!(args["find"], "Alpha");
    assert_eq!(args["max_bytes"], 900);
    assert_eq!(args["context"], false);
    assert!(command.arguments(&["Alpha".into(), "unexpected".into()]).is_err());
}

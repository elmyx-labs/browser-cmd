//! The browser is spoken to only by the modules that own each part of the
//! protocol (ARCHITECTURE.md), checked from the source: every quoted
//! `Domain.method` outside tests must be one its file is allowed, every
//! allowance must be used, and nothing in `session` or `tools` sends a command
//! of its own.

use std::path::Path;

use regex::Regex;

/// By file, or by directory when it ends in `/`, the methods and events it may
/// name; `Domain.*` is a whole domain.
const ALLOWED: &[(&str, &[&str])] = &[
    ("chrome/cdp.rs", &["Target.*", "Browser.getVersion"]),
    ("chrome/tab.rs", &["Target.*", "Page.enable", "Page.getFrameTree"]),
    ("chrome/launch/", &["Browser.close"]),
    ("browser/host.rs", &["Browser.close"]),
    (
        "chrome/events/",
        &[
            "Page.frameAttached",
            "Page.frameDetached",
            "Page.frameNavigated",
            "Page.navigatedWithinDocument",
            "Page.frameRequestedNavigation",
            "Page.frameStartedLoading",
            "Page.frameStoppedLoading",
            "Page.javascriptDialogOpening",
            "Page.handleJavaScriptDialog",
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
            "Network.loadingFailed",
            "Browser.downloadWillBegin",
            "Browser.downloadProgress",
            "Inspector.targetCrashed",
            "Log.entryAdded",
            "Target.detachedFromTarget",
            "Target.targetCreated",
            "Target.targetInfoChanged",
            "Target.targetDestroyed",
            "Target.targetCrashed",
        ],
    ),
    ("chrome/traffic.rs", &["Network.*"]),
    ("input/", &["Input.*"]),
    ("page/world.rs", &["Runtime.*", "Page.createIsolatedWorld", "DOM.resolveNode", "DOM.describeNode"]),
    ("page/frames.rs", &["Page.getFrameTree"]),
    (
        "page/navigation.rs",
        &[
            "Page.navigate",
            "Page.getNavigationHistory",
            "Page.navigateToHistoryEntry",
            "Page.reload",
            "Page.stopLoading",
        ],
    ),
    ("page/view.rs", &["Page.captureScreenshot", "Page.getLayoutMetrics", "DOM.getBoxModel"]),
    ("page/dom.rs", &["DOM.getDocument", "DOM.setFileInputFiles", "DOM.getNodeForLocation"]),
    ("page/snapshot/capture.rs", &["Accessibility.getFullAXTree"]),
    ("page/canvas.rs", &["Page.addScriptToEvaluateOnNewDocument", "Runtime.evaluate"]),
    (
        "browser/prepare.rs",
        &["Accessibility.enable", "Log.enable", "Inspector.enable", "Browser.setDownloadBehavior"],
    ),
    ("browser/identity.rs", &["Network.setUserAgentOverride"]),
    ("tabs/cookies.rs", &["Storage.getCookies", "Storage.setCookies"]),
];

/// The `cdp` command, which sends whatever method its caller names.
const ESCAPE_HATCH: &str = "tools/script.rs";

/// The allowances that let `file` name `method`, by their place in [`ALLOWED`].
fn allowing(file: &str, method: &str) -> Vec<(usize, usize)> {
    let places = ALLOWED.iter().enumerate();
    let mine =
        places.filter(|(_, (place, _))| file == *place || place.ends_with('/') && file.starts_with(place));
    mine.flat_map(|(at, (_, methods))| {
        methods.iter().enumerate().filter_map(move |(which, allowed)| {
            let fits = match allowed.strip_suffix('*') {
                Some(domain) => method.starts_with(domain),
                None => method == *allowed,
            };
            fits.then_some((at, which))
        })
    })
    .collect()
}

fn visit(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn the_browser_is_spoken_to_only_where_the_protocol_is_owned() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    visit(&src, &mut files);
    let literal = Regex::new(r#""([A-Z][A-Za-z]*\.[a-z][A-Za-z]*)""#).unwrap();
    let sends = Regex::new(r"\b(tab|cdp)\.(call|send)\(").unwrap();
    let (mut wrong, mut used) = (Vec::new(), std::collections::HashSet::new());
    for file in files {
        let relative = file.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
        if relative.starts_with("tests/") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        let code: String = text
            .split("#[cfg(test)]\nmod ")
            .next()
            .unwrap()
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for method in literal.captures_iter(&code).map(|found| found[1].to_owned()) {
            let allowances = allowing(&relative, &method);
            if allowances.is_empty() {
                wrong.push(format!("{relative} names {method}"));
            }
            used.extend(allowances);
        }
        let high = relative.starts_with("session/") || relative.starts_with("tools/");
        let listed = relative == ESCAPE_HATCH || ALLOWED.iter().any(|(place, _)| *place == relative);
        if high && !listed && sends.is_match(&code) {
            wrong.push(format!("{relative} sends a browser command"));
        }
    }
    for (at, (place, methods)) in ALLOWED.iter().enumerate() {
        for (_, method) in methods.iter().enumerate().filter(|(which, _)| !used.contains(&(at, *which))) {
            wrong.push(format!("{place} is allowed {method} and names none"));
        }
    }
    assert!(wrong.is_empty(), "CDP outside its owners, or allowed and unused:\n{}", wrong.join("\n"));
}

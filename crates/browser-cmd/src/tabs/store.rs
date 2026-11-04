//! The file that carries a session's tabs past a restart of the daemon: one
//! per browser and session, written whole or not at all, in a versioned form.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Settings;
use crate::tabs::registry::Registry;

const VERSION: u32 = 1;

/// What a session leaves for the next daemon: its tabs, the active one, the
/// ones it left most recent last, and the browser contexts of its own.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Saved {
    #[serde(default)]
    pub(super) version: u32,
    #[serde(flatten)]
    pub(crate) registry: Registry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) active: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) previous: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) context: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) retired: Vec<String>,
}

/// Where a session's tabs on a browser are kept — the profile it launches, or
/// the endpoint it attaches to — under the session's name made readable and a
/// digest of both, so no two keys share a file.
pub(crate) fn path(settings: &Settings, session: &str) -> PathBuf {
    let browser = settings.attach.clone().unwrap_or_else(|| settings.profile.display().to_string());
    let readable: String = session
        .chars()
        .take(40)
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    let digest = crate::workspace::digest(&format!("{browser}\0{session}"));
    settings.workspace.join(".browser-cmd").join(format!("tabs-{readable}-{digest:016x}.json"))
}

/// Where a registry is kept, and what was last written there.
pub(crate) struct Store {
    path: PathBuf,
    written: String,
}

impl Store {
    /// What a file holds, or nothing. A file that cannot be read is set aside
    /// rather than lost, and handles go on after the highest it still shows:
    /// said once.
    pub(crate) fn open(path: PathBuf) -> (Self, Saved, Option<String>) {
        let Ok(bytes) = std::fs::read(&path) else {
            return (Self { path, written: String::new() }, Saved::default(), None);
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if let Ok(saved) = serde_json::from_str::<Saved>(&text)
            && saved.version == VERSION
        {
            return (Self { path, written: text }, saved, None);
        }
        let aside = path.with_extension("json.bad");
        let moved = std::fs::rename(&path, &aside).is_ok();
        let mut saved = Saved::default();
        saved.registry.next = highest(&text);
        let note = format!(
            "This session's tab file {} could not be read{}; its tabs are not known, and new ones are \
             numbered from t{}.",
            name(&path),
            if moved { format!(" and was set aside as {}", name(&aside)) } else { String::new() },
            saved.registry.next + 1
        );
        (Self { path, written: String::new() }, saved, Some(note))
    }

    /// Write when it changed since last written, through a file renamed into
    /// place so a reader never sees half of it; best effort.
    pub(crate) fn save(&mut self, mut saved: Saved) {
        saved.version = VERSION;
        let Ok(text) = serde_json::to_string(&saved) else { return };
        if text == self.written {
            return;
        }
        let partial = self.path.with_extension(format!("json.{}.tmp", std::process::id()));
        if self.path.parent().is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
            && std::fs::write(&partial, &text).is_ok()
            && std::fs::rename(&partial, &self.path).is_ok()
        {
            self.written = text;
        }
    }
}

fn name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The highest handle a damaged file still shows.
fn highest(text: &str) -> u32 {
    let numbers = regex::Regex::new(r#""(?:handle|next)"\s*:\s*(\d+)"#).expect("a valid pattern");
    numbers.captures_iter(text).filter_map(|found| found[1].parse().ok()).max().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_are_never_reused_and_the_file_round_trips_whole() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".browser-cmd/tabs-s.json");
        let (mut store, mut saved, note) = Store::open(path.clone());
        assert!(note.is_none(), "no file is nothing to say");
        let registry = &mut saved.registry;
        assert_eq!(registry.adopt("A").handle, 1);
        assert_eq!(registry.adopt("B").handle, 2);
        assert_eq!(registry.adopt("A").handle, 1, "a known target keeps its handle");
        let kept = registry.adopt("B");
        kept.kept = Some("https://b.test/#top".into());
        kept.label = Some("price".into());
        registry.seen("B", "https://b.test/", "B", crate::tabs::status::TabStatus::Ok);
        registry.forget("A");
        assert_eq!(registry.adopt("C").handle, 3, "a closed tab's handle is not given again");
        (saved.active, saved.previous, saved.context) =
            (Some("C".into()), vec!["B".into()], Some("X".into()));
        store.save(saved.clone());
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "nothing half-written is left beside it");
        let (_, read, _) = Store::open(path.clone());
        assert_eq!(read.registry.entries, saved.registry.entries);
        assert_eq!(
            (read.registry.next, read.active.as_deref(), read.context.as_deref()),
            (3, Some("C"), Some("X"))
        );
        assert_eq!(read.previous, ["B"]);
        assert_eq!(read.registry.named("t2").unwrap().label.as_deref(), Some("price"));
        assert!(read.registry.named("t9").is_none() && read.registry.named("BEEF").is_none());
        let text = std::fs::read_to_string(path).unwrap();
        assert!(
            text.starts_with("{\"version\":1,") && text.contains("\"kept\":\"https://b.test/#top\""),
            "{text}"
        );
    }

    #[test]
    fn a_torn_or_foreign_file_is_set_aside_and_numbering_goes_on_after_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tabs-s.json");
        for (text, next) in [
            (r#"{"version":1,"next":7,"entries":[{"handle":9,"target":"A","url":"","ti"#, 9),
            (r#"{"version":2,"next":4,"entries":[]}"#, 4),
            (r#"{"next":3,"entries":[]}"#, 3),
            ("\u{0}garbage", 0),
        ] {
            std::fs::write(&path, text).unwrap();
            let (mut store, saved, note) = Store::open(path.clone());
            let note = note.expect("said once");
            assert_eq!(saved.registry.next, next, "{text}");
            assert!(
                note.contains("tabs-s.json.bad") && note.contains(&format!("from t{}", next + 1)),
                "{note}"
            );
            assert_eq!(std::fs::read_to_string(temp.path().join("tabs-s.json.bad")).unwrap(), text);
            assert!(!path.exists());
            store.save(saved);
            assert!(Store::open(path.clone()).2.is_none(), "what is written next reads back");
            std::fs::remove_file(&path).unwrap();
        }
    }

    #[test]
    fn each_browser_and_session_has_a_file_of_its_own() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::new(temp.path(), temp.path());
        let name = |settings: &Settings, session: &str| {
            path(settings, session).file_name().unwrap().to_string_lossy().into_owned()
        };
        let plain = name(&settings, "a.b");
        assert!(plain.starts_with("tabs-a_b-") && plain.ends_with(".json"), "{plain}");
        assert_ne!(plain, name(&settings, "a_b"), "names that read alike are kept apart");
        assert_eq!(plain, name(&settings, "a.b"));
        settings.profile = temp.path().join("other");
        assert_ne!(plain, name(&settings, "a.b"), "another profile is another browser");
        settings.attach = Some("http://127.0.0.1:9222".into());
        let attached = name(&settings, "a.b");
        settings.attach = Some("http://127.0.0.1:9223".into());
        assert_ne!(attached, name(&settings, "a.b"), "another endpoint is another browser");
        assert!(name(&settings, &"x".repeat(300)).len() < 80, "a long name stays a short file name");
    }
}

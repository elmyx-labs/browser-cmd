//! The workspace and the files in it: its root, names that outlast a build,
//! numbered files. A number is claimed by creating the file, never by looking
//! first, so two sessions cannot be given the same one.

use std::path::{Path, PathBuf};

use crate::{Error, Result, Settings};

/// Saved texts of one kind kept in a workspace. A text is what a page said, to
/// be read again in the same task; a screenshot is evidence, and is never removed.
const TEXTS_KEPT: usize = 200;

/// Bytes kept under the next free `kind-NNNN.ext` from `first`: text beside the
/// workspace's own state, media with the screenshots. Answers with the number used.
pub(crate) fn spill(
    settings: &Settings,
    kind: &str,
    ext: &str,
    first: u64,
    bytes: &[u8],
) -> Result<(PathBuf, u64)> {
    let directory = match ext {
        "txt" => settings.workspace.join(".browser-cmd"),
        _ => settings.screenshots.clone(),
    };
    let saved = save_new(
        &directory,
        first,
        settings.limits.screenshots,
        |number| format!("{kind}-{number:04}.{ext}"),
        bytes,
    )?;
    if ext == "txt" {
        keep_newest(&directory, &format!("{kind}-"), TEXTS_KEPT);
    }
    Ok(saved)
}

/// Remove all but the `kept` most recently written files named `prefix…txt`, so
/// the names never run out and a workspace does not grow with every long page.
fn keep_newest(directory: &Path, prefix: &str, kept: usize) {
    let mut texts: Vec<_> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(prefix) && name.ends_with(".txt")
        })
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    texts.sort();
    for (_, old) in texts.iter().rev().skip(kept) {
        let _ = std::fs::remove_file(old);
    }
}

/// The workspace a directory is in: `BROWSER_CMD_WORKSPACE` when set, else
/// the git work tree around it, else the directory itself.
pub(crate) fn root(dir: &Path) -> PathBuf {
    crate::env::var("WORKSPACE").map_or_else(|| crate::env::repository(dir), PathBuf::from)
}

/// A 64-bit FNV-1a digest: the same in every build, for names that outlast one.
pub(crate) fn digest(text: &str) -> u64 {
    text.bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3))
}

/// Answers with the number used, so a caller counting names carries on from there.
pub(crate) fn save_new(
    directory: &Path,
    first: u64,
    tries: u64,
    name: impl Fn(u64) -> String,
    bytes: &[u8],
) -> Result<(PathBuf, u64)> {
    std::fs::create_dir_all(directory)?;
    for number in first..first.saturating_add(tries) {
        let path = directory.join(name(number));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(bytes)?;
                return Ok((path, number));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Tool(format!("no free name is left under {}", directory.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_digest_is_fnv_1a() {
        assert_eq!(digest(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(digest("a"), 0xaf63_dc4c_8601_ec8c, "FNV-1a, not a hash that may change between builds");
    }

    #[test]
    fn a_taken_name_is_stepped_over_and_an_exhausted_run_is_an_error() {
        let temp = tempfile::tempdir().unwrap();
        let name = |n: u64| format!("shot-{n:04}.png");
        let (first, number) = save_new(temp.path(), 0, 3, name, b"one").unwrap();
        assert_eq!((first.file_name().unwrap().to_str().unwrap(), number), ("shot-0000.png", 0));
        let (second, number) = save_new(temp.path(), 0, 3, name, b"two").unwrap();
        assert_eq!((second.file_name().unwrap().to_str().unwrap(), number), ("shot-0001.png", 1));
        assert_eq!(std::fs::read(first).unwrap(), b"one", "an existing file is never overwritten");
        save_new(temp.path(), 0, 3, name, b"three").unwrap();
        assert!(save_new(temp.path(), 0, 3, name, b"four").is_err());
    }

    #[test]
    fn the_newest_saved_texts_are_kept_and_nothing_else_is_removed() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["text-0001.txt", "text-0002.txt", "text-0003.txt", "network-0001.txt", "text-0001.png"] {
            std::fs::write(temp.path().join(name), "").unwrap();
        }
        keep_newest(temp.path(), "text-", 2);
        let mut left: Vec<_> =
            std::fs::read_dir(temp.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        left.sort();
        assert_eq!(left, ["network-0001.txt", "text-0001.png", "text-0002.txt", "text-0003.txt"]);
    }
}

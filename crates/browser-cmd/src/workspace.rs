//! The workspace and the files in it: its root, names that outlast a build,
//! numbered files. A number is claimed by creating the file, never by looking
//! first, so two sessions cannot be given the same one.

use std::path::{Path, PathBuf};

use crate::{Error, Result, Settings};

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
    save_new(
        &directory,
        first,
        settings.limits.screenshots,
        |number| format!("{kind}-{number:04}.{ext}"),
        bytes,
    )
}

/// The workspace a directory is in: `BROWSER_CMD_WORKSPACE` when set, else
/// the git work tree around it, else the directory itself.
pub(crate) fn root(dir: &Path) -> PathBuf {
    crate::env::var("WORKSPACE").map_or_else(|| repository(dir), PathBuf::from)
}

fn repository(dir: &Path) -> PathBuf {
    dir.ancestors().find(|at| at.join(".git").exists()).unwrap_or(dir).to_owned()
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
    fn a_subdirectory_is_in_the_workspace_of_its_git_work_tree() {
        let temp = tempfile::tempdir().unwrap();
        let deep = temp.path().join("project/src/deep");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(repository(&deep), deep, "no work tree: the directory itself");
        std::fs::create_dir(temp.path().join("project/.git")).unwrap();
        assert_eq!(repository(&deep), temp.path().join("project"));
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
}

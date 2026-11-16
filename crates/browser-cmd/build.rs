//! The build's identity: the crate version, and the commit it was built from
//! when there is one. Two builds of one version are otherwise the same name
//! for different programs, and a daemon of the older one keeps answering.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    let mut build = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    if let Some(commit) = git(&["rev-parse", "--short=10", "HEAD"]) {
        build.push_str(&format!("+{commit}"));
        // Uncommitted work has no commit to name it, so the moment it was built does.
        if git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|changes| !changes.is_empty())
        {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
            build.push_str(&format!(".dirty.{}", now.map(|d| d.as_secs()).unwrap_or_default()));
        }
        if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
            println!("cargo:rerun-if-changed={dir}/HEAD");
            println!("cargo:rerun-if-changed={dir}/index");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rustc-env=BROWSER_CMD_BUILD={build}");
}

//! The agent skill, carried in the binary it describes: a skill installed from
//! one build and a binary from another teach commands that are not there.

use std::path::PathBuf;

use crate::{Error, Result};

pub(crate) const SKILL: &str = include_str!("../../../../plugin/skills/browser-cmd/SKILL.md");

/// Where each coding agent on this machine reads the skill: Claude Code under
/// `~/.claude`, Codex and Pi under `~/.agents`. An agent whose directory is
/// absent is not installed here.
pub(crate) fn places() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
    else {
        return Vec::new();
    };
    let agents = [".codex", ".pi", ".agents"].iter().any(|agent| home.join(agent).is_dir());
    [(home.join(".claude").is_dir(), ".claude"), (agents, ".agents")]
        .into_iter()
        .filter(|(present, _)| *present)
        .map(|(_, root)| home.join(root).join("skills/browser-cmd/SKILL.md"))
        .collect()
}

/// `skill` prints it; `skill install` writes it for every agent found.
pub(crate) fn run(rest: &[String]) -> Result<()> {
    match rest {
        [] => print!("{SKILL}"),
        [install] if install == "install" => {
            let places = places();
            if places.is_empty() {
                return Err(Error::Tool(
                    "no coding agent found (~/.claude, ~/.codex, ~/.pi); `browser-cmd skill` prints the skill \
                     to put where yours reads skills"
                        .into(),
                ));
            }
            for place in places {
                std::fs::create_dir_all(place.parent().unwrap_or(&place))?;
                std::fs::write(&place, SKILL)?;
                println!("skill: wrote {}", place.display());
            }
        }
        _ => return Err(Error::Config("usage: browser-cmd skill [install]".into())),
    }
    Ok(())
}

//! `--help`: the catalogue, one wrapped line per command, between a summary and a tail.

use crate::api::command::CATALOGUE;

pub(crate) fn help() -> String {
    let mut out = String::from(SUMMARY);
    // One line each: the whole of what a command does is its own `--help`,
    // read when it is used, not every time the catalogue is.
    for command in CATALOGUE {
        let first = command.about.find(". ").map_or(command.about, |end| &command.about[..=end]);
        out.push_str(&format!("\n  {} {}\n", command.name, command.usage));
        for line in wrapped(first, 74) {
            out.push_str(&format!("      {line}\n"));
        }
    }
    out.push_str(TAIL);
    out
}

fn wrapped(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

const SUMMARY: &str = "browser-cmd: a real Chrome for an agent, driven from the shell

Usage:
  browser-cmd <command> [args]     run one browser action against a warm daemon
  browser-cmd status               is the daemon up
  browser-cmd batch <<'EOF'        several commands, one per line, in one call: each
                                   but the last says one line, the first failure stops
  browser-cmd --serve [ADDR]       run the daemon in the foreground, in its workspace
  browser-cmd <command> --help     one command's syntax

Commands. Each prints the page state afterwards — after an action on the same
document, only what changed — and a @ref from one command resolves in the next.
--brief (or BROWSER_CMD_BRIEF=1) prints one line for an action that worked.
Arguments are words where the shape is obvious, and key=value otherwise; a lone
'{json}' is passed through whole.
";

const TAIL: &str = "
The daemon starts on first use and holds the session for this workspace
(BROWSER_CMD_WORKSPACE, else the git work tree, else the working directory);
BROWSER_CMD_ADDR picks its port. A failed command drops a trace under
.browser-cmd/traces/ (BROWSER_CMD_NO_TRACE=1 to disable) and repeats its reason
on stderr. The browser is configured by BROWSER_CMD_* variables: PROFILE, CDP,
EXECUTABLE, HEADLESS, NO_SANDBOX, ARGS, WINDOW, LANGUAGE, TIMEZONE, PROXY,
PROXY_HOSTS, DIALOGS, PACE, NETWORK, NETWORK_NOTES and AWARENESS.
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::command::LOCAL;

    #[test]
    fn the_help_is_the_catalogue_and_leaves_nothing_out() {
        let help = help();
        for command in CATALOGUE {
            assert!(help.contains(command.name), "{} is not in --help", command.name);
            let first = command.about.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
            assert!(help.contains(&first), "{} is listed without its description", command.name);
        }
        for local in LOCAL {
            assert!(help.contains(local), "{local} is not in --help");
        }
        assert!(help.lines().all(|line| line.chars().count() <= 90), "a help line runs too wide");
    }
}

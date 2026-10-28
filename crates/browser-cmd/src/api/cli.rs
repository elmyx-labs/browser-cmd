//! The shell CLI: each subcommand is one browser action printing compact text.
//! Ref state lives in the daemon, so a ref from one process resolves in the next.

use crate::api::client::Daemon;
use crate::api::command;
use crate::text::fence::{STATE, unfenced};
use crate::{Error, Result};
use serde_json::{Value, json};

/// How much of a failure's reason is repeated on stderr, which survives an
/// agent sending the output to `/dev/null`.
const REASON_BYTES: usize = 600;

/// The date and IANA zone the browser runs in — `BROWSER_CMD_TIMEZONE`, else
/// `TZ`, else the zone `/etc/localtime` links to — read without a browser.
fn local_time() -> Option<String> {
    let zone = crate::env::var("TIMEZONE")
        .or_else(|| std::env::var("TZ").ok().filter(|zone| !zone.is_empty()))
        .or_else(|| {
            let link = std::fs::read_link("/etc/localtime").ok()?;
            Some(link.to_str()?.split_once("zoneinfo/")?.1.to_owned())
        })?;
    let date = std::process::Command::new("date").arg("+%Y-%m-%d %H:%M").env("TZ", &zone).output().ok()?;
    Some(format!("{} ({zone})", String::from_utf8(date.stdout).ok()?.trim()))
}

pub async fn run(argv: &[String]) -> Result<()> {
    let (name, rest) = argv.split_first().ok_or_else(|| Error::Config("no command".into()))?;
    match name.as_str() {
        "batch" => return batch().await,
        "back" | "forward" => {
            let argv: Vec<String> =
                ["navigate".into(), name.clone()].into_iter().chain(rest.to_vec()).collect();
            return Box::pin(run(&argv)).await;
        }
        _ => {}
    }
    // A health check that starts nothing: is the daemon up, and what is open.
    if command::LOCAL.contains(&name.as_str()) {
        let address = crate::api::http::address();
        match crate::api::client::up(&address, &crate::api::http::Limits::from_env()?).await? {
            true => println!("daemon: up at {address}"),
            false => println!("daemon: down (nothing at {address}; a command starts one)"),
        }
        if let Some(clock) = local_time() {
            println!("Local time: {clock}");
        }
        return Ok(());
    }
    let command = command::find(name).ok_or_else(|| Error::Config(format!("no such command: {name}")))?;
    // Output flags lead or trail the arguments, so text in between stays as typed.
    let is_flag = |word: &String| matches!(word.as_str(), "--brief" | "--raw");
    let mut rest = rest.to_vec();
    let mut flags = Vec::new();
    while rest.last().is_some_and(is_flag) {
        flags.extend(rest.pop());
    }
    while rest.first().is_some_and(is_flag) {
        flags.push(rest.remove(0));
    }
    let brief = flags.iter().any(|word| word == "--brief") || crate::env::flag("BRIEF")?.unwrap_or(false);
    let raw = flags.iter().any(|word| word == "--raw");
    if rest.len() == 1 && matches!(rest[0].as_str(), "--help" | "-h") {
        println!("{} {}\n\n{}", command.name, command.usage, command.about);
        return Ok(());
    }
    if command.name == "eval" && rest == ["-"] {
        rest = vec![std::io::read_to_string(std::io::stdin())?];
    }
    let arguments = command.arguments(&rest)?;
    mark_used();
    let daemon = Daemon::connect(&std::env::current_exe()?).await?;
    let reply = match daemon.call(command.name, arguments).await {
        Ok(reply) => reply,
        Err(error) => {
            trace(&daemon, argv, &error.to_string()).await;
            return Err(error);
        }
    };
    if reply.ok && brief {
        println!("{}", briefly(command.name, &reply.output));
    } else if raw && matches!(command.name, "eval" | "text") {
        println!("{}", unwrapped(command.name, &reply.output));
    } else {
        println!("{}", reply.output);
    }
    if !reply.ok {
        eprintln!("browser-cmd: {}", reason(&reply.output));
        eprintln!(
            "browser-cmd: command={} status={} kind={}",
            command.name,
            if reply.unacknowledged() { "unknown" } else { "error" },
            reply.error.as_deref().unwrap_or("tool")
        );
        trace(&daemon, argv, &reply.output).await;
        return Err(if reply.unacknowledged() {
            Error::Unacknowledged(command.name.into())
        } else {
            Error::Tool(format!("{} failed; use the error and current page state above", command.name))
        });
    }
    Ok(())
}

/// Several commands, one per line of stdin, in one call: each but the last
/// says one line, the last says all it would alone, and the first to fail
/// stops the rest with its whole answer — the page as it stood at the failure.
async fn batch() -> Result<()> {
    let script = std::io::read_to_string(std::io::stdin())?;
    let lines: Vec<Vec<String>> = script
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| words(line.strip_prefix("browser-cmd ").unwrap_or(line)))
        .collect::<Result<_>>()?;
    if lines.is_empty() {
        return Err(Error::Config("batch reads commands, one per line, from stdin".into()));
    }
    let total = lines.len();
    for (index, mut argv) in lines.into_iter().enumerate() {
        if argv.first().is_some_and(|name| name == "batch") {
            return Err(Error::Config("a batch cannot hold another batch".into()));
        }
        if index + 1 < total {
            argv.push("--brief".into());
        }
        Box::pin(run(&argv)).await?;
    }
    Ok(())
}

/// A line split as a shell would split it: on spaces, except inside quotes.
fn words(line: &str) -> Result<Vec<String>> {
    let (mut out, mut word, mut quote, mut chars) = (Vec::new(), None::<String>, None, line.chars());
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => word.get_or_insert_default().extend(chars.next()),
            (Some(_), c) => word.get_or_insert_default().push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                word.get_or_insert_default();
            }
            (None, '\\') => word.get_or_insert_default().extend(chars.next()),
            (None, c) if c.is_whitespace() => out.extend(word.take()),
            (None, c) => word.get_or_insert_default().push(c),
        }
    }
    if quote.is_some() {
        return Err(Error::Config(format!("unclosed quote in batch line: {line}")));
    }
    out.extend(word);
    Ok(out)
}

fn reason(output: &str) -> String {
    let error = output.find("Error:").map_or(output, |at| &output[at..]);
    crate::text::clip(error.split("\n\n").next().unwrap_or_default().trim(), REASON_BYTES)
}

/// An action's answer as one line: the command, what it did, where the tab is
/// now and any notes. Output that carries no page state is left whole.
fn briefly(name: &str, output: &str) -> String {
    let open = format!("<{STATE}");
    let Some((before, state)) = output.split_once(&open).filter(|_| name != "snapshot") else {
        return output.to_owned();
    };
    let (action, notes) = before.split_once('\n').unwrap_or((before, ""));
    let (state, after) = state.split_once(&format!("</{STATE}>")).unwrap_or((state, ""));
    let field = |key: &str| state.lines().find_map(|line| line.strip_prefix(key)).unwrap_or_default();
    let notes = notes.split_whitespace().chain(after.split_whitespace()).collect::<Vec<_>>().join(" ");
    let mut line = format!("{name}: {} | URL: {}", action.trim(), field("URL: "));
    if let Some(title) = state.lines().find_map(|line| line.strip_prefix("Title: ")) {
        line.push_str(&format!(" | Title: {title}"));
    }
    if !notes.is_empty() {
        line.push_str(&format!(" | {notes}"));
    }
    line
}

/// `eval` or `text` output without its quoting block; `eval`'s value as
/// one line of JSON.
fn unwrapped(name: &str, output: &str) -> String {
    let body = unfenced("browser", output).unwrap_or_else(|| output.to_owned());
    match serde_json::from_str::<Value>(&body) {
        Ok(value) if name == "eval" => value.to_string(),
        _ => body,
    }
}

/// Best-effort, so a failed step can be understood without re-running it.
async fn trace(daemon: &Daemon, argv: &[String], error: &str) {
    if crate::env::flag("NO_TRACE").ok().flatten() == Some(true) {
        return;
    }
    let logs = daemon.call("console", json!({})).await.ok().map(|reply| reply.output);
    let when = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let body = format!(
        "command: {}\nerror: {error}\n\nbrowser log:\n{}\n",
        argv.join(" "),
        logs.as_deref().unwrap_or("(none)")
    );
    let dir = std::path::Path::new(".browser-cmd").join("traces");
    if std::fs::create_dir_all(&dir).is_ok() {
        let path = dir.join(format!("{when}.txt"));
        let _ = std::fs::write(&path, body);
        eprintln!("browser-cmd: trace written to {}", path.display());
    }
}

/// Lets whatever runs after the agent tell this directory was browsed in, even
/// without a transcript.
fn mark_used() {
    let dir = std::path::Path::new(".browser-cmd");
    if std::fs::create_dir_all(dir).is_ok() {
        let _ = std::fs::write(dir.join("used"), b"");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::fence::{bounded_state, untrusted};

    #[test]
    fn the_reason_for_a_failure_is_its_first_error_paragraph_bounded() {
        assert_eq!(reason("Error: no such ref\n\nYou are repeating yourself."), "Error: no such ref");
        assert_eq!(reason(&format!("Error: {}", "x".repeat(2000))).len(), REASON_BYTES);
        assert_eq!(reason("Timed out\n\nmore"), "Timed out");
    }

    #[test]
    fn a_brief_action_is_one_line_with_its_notes_and_other_output_is_whole() {
        let state = bounded_state("URL: http://a.test/\nTitle: Shop\n\n- button \"Buy\" @e1a.1".into(), 1000);
        let output = format!("Clicked @e1a.1.\nA dialog opened.\n{state}\n\nYou are repeating yourself.");
        assert_eq!(
            briefly("click", &output),
            "click: Clicked @e1a.1. | URL: http://a.test/ | Title: Shop | A dialog opened. You are repeating yourself."
        );
        let untitled = bounded_state("URL: http://a.test/\n\n1 unchanged".into(), 1000);
        assert_eq!(
            briefly("click", &format!("Clicked.\n{untitled}")),
            "click: Clicked. | URL: http://a.test/"
        );
        assert_eq!(briefly("snapshot", &state), state);
        assert_eq!(briefly("eval", "42"), "42");
    }

    #[test]
    fn raw_output_drops_the_quoting_and_eval_prints_one_line_of_json() {
        assert_eq!(unwrapped("eval", &untrusted("{\n  \"a\": [\n    1\n  ]\n}")), r#"{"a":[1]}"#);
        assert_eq!(unwrapped("text", &untrusted("URL: x\n\nwords")), "URL: x\n\nwords");
        assert_eq!(unwrapped("eval", "Error: page script failed"), "Error: page script failed");
    }

    #[test]
    fn a_batch_line_splits_like_a_shell_line() {
        assert_eq!(
            super::words(r#"type @e1.2 "New York, NY" submit"#).unwrap(),
            ["type", "@e1.2", "New York, NY", "submit"]
        );
        assert_eq!(super::words("eval 'document.title' --raw").unwrap(), ["eval", "document.title", "--raw"]);
        assert_eq!(super::words(r#"text "say \"hi\"" "#).unwrap(), ["text", "say \"hi\""]);
        assert_eq!(super::words("click ''").unwrap(), ["click", ""]);
        assert!(super::words("text \"open").is_err());
    }
}

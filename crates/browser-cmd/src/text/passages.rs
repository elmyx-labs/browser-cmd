//! Reading a page by the phrase: the passages around each occurrence, and the
//! headings, table columns and selections that label a place in the text.

use crate::text::{ceil, clip, floor};
use crate::{Error, Result};

/// The passages of `text` around each occurrence of `phrase`, matched with Unicode simple case folding,
/// each with the byte offset it starts at so `from` can read on from there.
/// Two passages that would overlap are one. Each reaches `passage` bytes, at
/// most half of `limit`, around its match; bounded by `limit` in all.
/// With `contextual`, each passage leads with the section and selections it
/// sits under.
pub(crate) fn passages(
    text: &str,
    phrase: &str,
    passage: usize,
    limit: usize,
    contextual: bool,
) -> Result<String> {
    if phrase.is_empty() {
        return Err(Error::Tool("search phrase must not be empty".into()));
    }
    let pattern = regex::RegexBuilder::new(&regex::escape(phrase))
        .case_insensitive(true)
        .build()
        .map_err(|error| Error::Tool(format!("cannot search this phrase: {error}")))?;
    let wanted = phrase.to_lowercase();
    let width = passage.min(limit / 2);
    let mut hits = 0;
    let mut out = String::new();
    let mut shown_to = 0;
    let mut truncated = false;
    let mut said = String::new();
    for found in pattern.find_iter(text) {
        hits += 1;
        let through = found.end();
        let found = found.start();
        let start = ceil(text, found.saturating_sub(width / 2)).max(shown_to);
        let end = ceil(text, through.saturating_add(width / 2));
        if start >= end {
            continue;
        }
        let mut context =
            if contextual { context_before(text, found, context_budget(limit)) } else { String::new() };
        // Matches in one section share its labels: said once, at the first.
        if context == said {
            context.clear();
        } else {
            said.clone_from(&context);
        }
        let passage = format!("{context}[at {start}] …{}…\n", text[start..end].trim());
        if out.len() + passage.len() > limit {
            truncated = true;
            out.push_str(&format!("[more matches beyond byte {start}; text from={start} continues]\n"));
            break;
        }
        out.push_str(&passage);
        shown_to = end;
    }
    Ok(match hits {
        0 => format!("{wanted:?} is not in the available text ({} bytes)", text.len()),
        _ => format!(
            "{}{hits} match{} for {wanted:?}:\n{out}",
            if truncated { "At least " } else { "" },
            if hits == 1 { "" } else { "es" }
        ),
    })
}

pub(crate) fn context_budget(limit: usize) -> usize {
    (limit / 3).min(1200)
}

/// Carry the rendered section and preceding selected controls into a short
/// excerpt. These are page-derived labels, not verified business conditions.
pub(crate) fn context_before(text: &str, at: usize, limit: usize) -> String {
    let at = floor(text, at);
    let mut headings = vec![String::new(); 6];
    let mut selections = Vec::new();
    let mut columns = "";
    let before = &text[..at];
    let before = if before.ends_with('\n') {
        before
    } else {
        before.rsplit_once('\n').map_or("", |(complete, _)| complete)
    };
    for line in before.lines() {
        if line.starts_with("--- frame:") || line.starts_with("--- unreadable frame:") {
            headings.fill(String::new());
            selections.clear();
            columns = "";
        }
        let level = line.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ') {
            headings[level - 1] = line[level + 1..].to_owned();
            headings[level..].fill(String::new());
            columns = "";
        } else if (line.starts_with("[Selected: ") || line.starts_with("[Control value: "))
            && line.ends_with(']')
        {
            if !selections.contains(&line) {
                selections.push(line);
            }
            if selections.len() > 8 {
                selections.remove(0);
            }
        } else if line.starts_with("[Table columns:") {
            columns = line;
        } else if line == "[End table]" || line == "[Table]" {
            columns = "";
        }
    }
    let mut parts = Vec::new();
    let path = headings.iter().filter(|h| !h.is_empty()).cloned().collect::<Vec<_>>().join(" > ");
    if !path.is_empty() {
        parts.push(format!("Section: {path}"));
    }
    parts.extend(selections.into_iter().map(str::to_owned));
    if !columns.is_empty() {
        parts.push(columns.to_owned());
    }
    let prefix = "Context (page labels): ";
    let available = limit.saturating_sub(prefix.len() + 1);
    if parts.is_empty() || available < 4 {
        String::new()
    } else {
        format!("{prefix}{}\n", clip(&parts.join("; "), available))
    }
}

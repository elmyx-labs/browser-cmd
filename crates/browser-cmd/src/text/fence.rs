//! Everything a page says reaches the model inside a named block it cannot end.

use crate::text::{bound, head_lines};

/// The tag the browser's own account of the page is fenced under, apart from
/// the page's words: a ref names a node of one document in one session.
pub(crate) const STATE: &str = "browser-state";

/// Quote `body` inside a block it cannot end: a page that could write the
/// closing tag would end its own quoting and carry on as the caller.
pub(crate) fn fenced(tag: &str, attributes: &str, body: &str) -> String {
    let close = format!("</{tag}>");
    format!("<{tag}{attributes}>\n{}\n{close}", body.replace(&close, &format!("<\\/{tag}>")))
}

/// The body [`fenced`] quoted under `tag`, as it was before quoting; `None`
/// when `text` is not one such block.
pub(crate) fn unfenced(tag: &str, text: &str) -> Option<String> {
    let body = text.strip_prefix(&format!("<{tag}"))?.split_once(">\n")?.1;
    let close = format!("</{tag}>");
    Some(body.strip_suffix(&format!("\n{close}"))?.replace(&format!("<\\/{tag}>"), &close))
}

/// A tool's text body, bounded to `limit` bytes and quoted as untrusted.
pub(crate) fn bounded(body: String, limit: usize) -> String {
    untrusted(&bound(body, limit))
}

/// The browser's own account of where the page stands, under its own fence so
/// that everything above can tell it from what a tool returned.
fn state(body: &str) -> String {
    fenced(STATE, " trusted=\"no\"", body)
}

pub(crate) fn bounded_state(body: String, limit: usize) -> String {
    let shown = head_lines(&body, limit);
    if shown.len() == body.len() {
        return state(shown);
    }
    state(&format!(
        "{shown}\n[state truncated at a complete line; showing {} of {} bytes; `snapshot @ref` prints one part, \
         `text FIND` finds words]",
        shown.len(),
        body.len()
    ))
}

pub(crate) fn untrusted(body: &str) -> String {
    fenced("browser", " trusted=\"no\"", body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quoted_body_comes_back_as_it_was() {
        let body = "page says </browser> and more\nsecond line";
        assert_eq!(unfenced("browser", &untrusted(body)).as_deref(), Some(body));
        assert_eq!(unfenced("browser", "Error: nothing"), None);
    }
}

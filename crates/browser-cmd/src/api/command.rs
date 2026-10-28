//! The one catalogue: `--help` and daemon dispatch both read it, so neither can drift.

use std::{future::Future, pin::Pin};

use serde_json::{Value, json};

use crate::session::{Run, Session};
use crate::{Error, Result};

pub struct Command {
    pub name: &'static str,
    pub usage: &'static str,
    pub about: &'static str,
    /// Whether it is meant to move the page, and so enters the trajectory
    /// memory. A re-look — a snapshot, a text dump — is not progress.
    pub(crate) changing: bool,
    parse: fn(&[String]) -> Result<Value>,
    pub(crate) run: Run,
}

macro_rules! run {
    ($verb:path) => {{
        fn run(session: &mut Session, args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + '_>> {
            Box::pin($verb(session, args))
        }
        run
    }};
}

impl Command {
    /// Whether it acts on a tab, so a crashed one is reloaded and a kept one
    /// gone is reopened first; a look at the tabs as they are does neither.
    pub(crate) fn acts(&self, args: &Value) -> bool {
        match self.name {
            "console" | "network" | "close" => false,
            "tab" => !matches!(args["action"].as_str(), Some("list" | "check")),
            _ => true,
        }
    }

    /// A lone `{…}` is passed through whole, so anything the shorthand does not
    /// reach is still sayable.
    pub(crate) fn arguments(&self, rest: &[String]) -> Result<Value> {
        if let [only] = rest
            && only.trim_start().starts_with('{')
        {
            return serde_json::from_str(only)
                .map_err(|error| Error::Config(format!("{}: arguments are not JSON: {error}", self.name)));
        }
        (self.parse)(rest)
    }
}

pub(crate) fn find(name: &str) -> Option<&'static Command> {
    CATALOGUE.iter().find(|command| command.name == name)
}

pub(crate) fn is_command(name: &str) -> bool {
    find(name).is_some() || LOCAL.contains(&name) || matches!(name, "back" | "forward")
}

/// Commands the client answers itself.
pub const LOCAL: &[&str] = &["status", "batch"];

pub const CATALOGUE: &[Command] = &[
    Command {
        name: "navigate",
        usage: "URL [here=true] | back | forward",
        about: "Open an http(s) URL in a real Chrome, or go back or forward. Every command prints the \
                page state afterwards — URL, title, what happened, and the controls with their refs — so \
                the next command needs no snapshot first. A Cloudflare or CAPTCHA box is ticked for you \
                and the state says how that went. From a kept or borrowed tab the URL opens in a new \
                tab, unless here=true.",
        parse: |rest| {
            let (here, url): (Vec<String>, Vec<String>) =
                rest.iter().cloned().partition(|word| word.starts_with("here="));
            let mut options = pairs(&here)?;
            options["url"] = json!(one("navigate", &url)?);
            Ok(options)
        },
        changing: true,
        run: run!(crate::tools::navigate::navigate),
    },
    Command {
        name: "snapshot",
        usage: "[@ref] [full] [coords]",
        about: "The page as its accessibility tree: controls and headings with @e1a.31 refs by \
                default, everything readable with full. The whole tree, where an action on the same \
                document prints only what it changed, and one line when it is the tree last shown whole; \
                @ref prints one element and what is inside it, a long tree is saved to a file whose head is \
                shown. coords appends the viewport x,y of each ref. A link to another site shows its host \
                and path, … for a query (text links=true has it whole).",
        parse: |rest| {
            let mut options = json!({
                "full": rest.iter().any(|word| word == "full"),
                "coords": rest.iter().any(|word| word == "coords"),
            });
            if let Some(handle) = rest.iter().find(|word| word.starts_with('@')) {
                options["ref"] = json!(handle);
            }
            Ok(options)
        },
        changing: false,
        run: run!(crate::tools::read::snapshot),
    },
    Command {
        name: "click",
        usage: "@ref | \"name\" | X,Y [double=true] [right=true] [again=true]",
        about: "Click an element by ref or by the words on it, or a viewport point by x and y, with a \
                real pointer that travels there first. A name the accessibility tree does not carry \
                falls back to a clickable custom control bearing that visible text. A new tab that \
                opens becomes the active one.",
        parse: |rest| aim("click", rest, true),
        changing: true,
        run: run!(crate::tools::pointer::click),
    },
    Command {
        name: "hover",
        usage: "@ref | \"name\" | X,Y",
        about: "Move the pointer onto an element or point without clicking, for menus that open on hover.",
        parse: |rest| aim("hover", rest, false),
        changing: false,
        run: run!(crate::tools::pointer::hover),
    },
    Command {
        name: "drag",
        usage: "from='{…}' to='{…}'",
        about: "Press on one element or point, travel to another with the button held, and let go: a \
                slider set, a card moved between columns, a map panned.",
        parse: pairs,
        changing: true,
        run: run!(crate::tools::pointer::drag),
    },
    Command {
        name: "type",
        usage: "[@ref] TEXT… [submit]",
        about: "Replace the text of a field by typing it key by key, then press Enter if asked to \
                submit. A date, time, colour or range field is given the text as its value. An empty \
                string clears the field. A lone TEXT with no ref goes in at the caret of whatever has \
                focus — an editor with no ref, a canvas document, a sheet cell, a frame's rich text — \
                after a click has put it there; a line break in it (from stdin: type - <<'EOF') is \
                entered as the Enter key.",
        parse: |rest| {
            if let [text] | [text, _] = rest
                && !text.starts_with('@')
                && rest.get(1).is_none_or(|word| word == "submit")
            {
                let text = match text.as_str() {
                    "-" => {
                        let text = std::io::read_to_string(std::io::stdin())?;
                        text.strip_suffix('\n').unwrap_or(&text).to_owned()
                    }
                    _ => text.clone(),
                };
                return Ok(json!({ "text": text, "submit": rest.len() == 2 }));
            }
            let (reference, tail) = rest.split_first().ok_or_else(|| needs("type", "text"))?;
            if tail.is_empty() {
                return Err(needs("type", "text after the ref (use an explicit empty string to clear)"));
            }
            let submit = tail.len() > 1 && tail.last().is_some_and(|word| word == "submit");
            let words = if submit { &tail[..tail.len() - 1] } else { tail };
            Ok(json!({ "ref": reference, "text": words.join(" "), "submit": submit }))
        },
        changing: true,
        run: run!(crate::tools::keyboard::type_text),
    },
    Command {
        name: "select",
        usage: "@ref OPTION…",
        about: "Choose an option in a native <select> by its text or value. A custom dropdown is clicked \
                open and its option clicked instead.",
        parse: |rest| {
            let (reference, option) =
                rest.split_first().ok_or_else(|| needs("select", "a ref and an option"))?;
            if option.is_empty() {
                return Err(needs("select", "a ref and an option"));
            }
            Ok(json!({ "ref": reference, "option": option.join(" ") }))
        },
        changing: true,
        run: run!(crate::tools::keyboard::select),
    },
    Command {
        name: "choose",
        usage: "@ref TEXT… [option=SUGGESTION]",
        about: "Fill an autocomplete or combobox: type TEXT, wait for its suggestions, pick the one that \
                matches (option= names it when TEXT is only what to search for), and confirm the field \
                took it. Several close matches are listed with their refs instead of guessed between. A \
                native <select> or a field's <datalist> is chosen in directly.",
        parse: |rest| {
            let (option, words): (Vec<&String>, Vec<&String>) =
                rest.iter().partition(|word| word.starts_with("option="));
            let [reference, text @ ..] = words.as_slice() else {
                return Err(needs("choose", "a ref and text"));
            };
            if text.is_empty() {
                return Err(needs("choose", "text after the ref"));
            }
            let mut args = json!({ "ref": reference, "text": text.iter().map(|word| word.as_str()).collect::<Vec<_>>().join(" ") });
            if let Some(option) = option.last() {
                args["option"] = json!(&option["option=".len()..]);
            }
            Ok(args)
        },
        changing: true,
        run: run!(crate::tools::choose::choose),
    },
    Command {
        name: "press",
        usage: "KEY",
        about: "Press a key or chord such as Enter, Escape, Tab, PageDown, ArrowDown, Control+a or Meta+a.",
        parse: |rest| Ok(json!({ "key": one("press", rest)? })),
        changing: true,
        run: run!(crate::tools::keyboard::press),
    },
    Command {
        name: "scroll",
        usage: "[@ref] [up|down|left|right|bottom|top|TEXT]",
        about: "Turn the wheel over the page, or scroll to an edge until the page stops growing — which \
                loads a whole infinite list in one call. Any other text brings the first element bearing \
                it into view. With a ref and no direction it scrolls that element into view; with a ref \
                and a direction it scrolls whatever that ref sits in, which is how a chat or a sidebar \
                is reached.",
        parse: |rest| {
            let mut words = rest.to_vec();
            let handle = words.iter().position(|word| word.starts_with('@')).map(|at| words.remove(at));
            let mut args = match words.first().map(String::as_str) {
                None => json!({}),
                Some(edge @ ("bottom" | "top")) => json!({ "to": edge }),
                Some(direction @ ("up" | "down" | "left" | "right")) => json!({ "direction": direction }),
                Some(_) => json!({ "to": words.join(" ") }),
            };
            if let Some(handle) = handle {
                args["ref"] = json!(handle);
            }
            Ok(args)
        },
        changing: true,
        run: run!(crate::tools::scroll::scroll),
    },
    Command {
        name: "text",
        usage: "[FIND] [ref=@ref] [from=N] [max_bytes=N] [links=true] [context=false] [--raw]",
        about: "The rendered text of the page, bounded, as untrusted data. A long page says where its \
                text stopped and continues with from; past 16 KiB without max_bytes the whole text is \
                also saved to .browser-cmd/text-NNNN.txt and its path printed. --raw drops the quoting. FIND gives the passages around each place a phrase \
                occurs. ref reads one element instead of the whole page — a card, a table, an article. \
                Headings, table rows and selected values stay with the reading; excerpts retain preceding \
                section context. max_bytes makes a short reading without shell head/grep. context=false \
                returns raw rendered text. links inlines each link as [text](url). The site's header, \
                menus, footer and cookie notice are named in one line, not read, unless full=true, FIND \
                or ref.",
        parse: find_then_pairs,
        changing: false,
        run: run!(crate::tools::read::text),
    },
    Command {
        name: "images",
        usage: "",
        about: "Image URLs, alt text and sizes on the page, as untrusted data.",
        parse: |_| Ok(json!({})),
        changing: false,
        run: run!(crate::tools::read::images),
    },
    Command {
        name: "pages",
        usage: "URL… [find=TEXT] [max_bytes=N] [keep=true] [full=true]",
        about: "Read up to 12 pages in one call, as untrusted data: each opens in a tab of its own, four \
                at a time, settles and is read, then closes — the sources to compare, the venues to list. \
                find gives the passages around a phrase on each page instead of its head; keep=true \
                leaves the tabs open, kept, and names them for tab switch. Each site's header, menus, \
                footer and cookie notice are named in one line, not read, unless full=true. The current \
                tab is untouched.",
        parse: |rest| {
            let (options, urls): (Vec<String>, Vec<String>) = rest.iter().cloned().partition(|word| {
                word.split_once('=')
                    .is_some_and(|(key, _)| matches!(key, "find" | "max_bytes" | "keep" | "full"))
            });
            if urls.is_empty() {
                return Err(needs("pages", "one or more URLs"));
            }
            let mut args = pairs(&options)?;
            args["urls"] = json!(urls);
            if let Some(find) = options.iter().find_map(|word| word.strip_prefix("find=")) {
                args["find"] = json!(find);
            }
            Ok(args)
        },
        changing: false,
        run: run!(crate::tools::pages::pages),
    },
    Command {
        name: "markup",
        usage: "ref=@ref [levels=N]",
        about: "The HTML of one element, and levels ancestors up, cleaned and bounded. What the tree and \
                the text do not carry — a rating drawn as icons, a class, a data attribute — is here: \
                read one row's markup, then write extract for every row.",
        parse: pairs,
        changing: false,
        run: run!(crate::tools::read::markup),
    },
    Command {
        name: "extract",
        usage: "fields='{…}' [each=SEL] [all=true] [identity=SEL] [limit=N]",
        about: "Read many like-shaped things in one call — every row of a table, every card in a \
                result list, every item of a feed — as JSON rows with a coverage record. When the \
                values a choice turns on are already printed on the list (a price, a rating, a \
                count, a link), take them here instead of opening each thing in turn. Field \
                selectors support @attribute (a@href, input@value) and + for next siblings. all \
                scrolls forward, telling rows apart by identity, a unique stable row selector, or \
                else by their field values; it does not prove whole-dataset completeness — read \
                termination and output_truncated. For a value the text and the tree do not carry, \
                read one row's markup first and write the selectors from it.",
        parse: pairs,
        changing: false,
        run: run!(crate::tools::extract::extract),
    },
    Command {
        name: "screenshot",
        usage: "[full | @ref | ref=@ref | selector=CSS | X,Y,W,H] [zoom=N]",
        about: "A PNG of the viewport, of the full page, or of one element or section by ref or CSS \
                selector, returned as an image and saved under the workspace. For canvases, charts, \
                picture puzzles and cross-origin frames; click by the x and y read off a viewport shot. \
                X,Y,W,H pictures that region of the viewport and zoom=2 (up to 4) enlarges a region, \
                ref or selector, for labels too small to read. Warns when the page is raw JSON or \
                another unrendered document.",
        parse: |rest| {
            let region = |word: &String| {
                let numbers: Vec<f64> = word.split(',').map_while(|n| n.trim().parse().ok()).collect();
                (numbers.len() == 4 && word.split(',').count() == 4).then_some(numbers)
            };
            let (handles, words): (Vec<String>, Vec<String>) = rest
                .iter()
                .filter(|word| *word != "full" && region(word).is_none())
                .cloned()
                .partition(|word| word.starts_with('@'));
            let mut options = pairs(&words)?;
            options["full_page"] = json!(rest.iter().any(|word| word == "full"));
            if let Some(handle) = handles.first() {
                options["ref"] = json!(handle);
            }
            if let Some(numbers) = rest.iter().find_map(region) {
                options["region"] = json!(numbers);
            }
            Ok(options)
        },
        changing: false,
        run: run!(crate::tools::capture::screenshot),
    },
    Command {
        name: "evidence",
        usage: "\"VALUE\"…",
        about: "Which of the given strings are visible in the viewport now, and where: x,y and the \
                element, or how many matches the page has out of view and the nearest page y. Reads the \
                rendered text and field values of the top document; reports facts, never whether an \
                answer is right. Warns when the page is raw JSON or another unrendered document.",
        parse: |rest| match rest {
            [] => Err(needs("evidence", "one or more strings to look for")),
            values => Ok(json!({ "values": values })),
        },
        changing: false,
        run: run!(crate::tools::capture::evidence),
    },
    Command {
        name: "wait",
        usage: "[SECONDS | TEXT] [gone=TEXT] [url=TEXT]",
        about: "Wait for a page that draws itself: until text appears, until text disappears, until the \
                URL contains a string, or for a number of seconds and the DOM to go quiet.",
        parse: |rest| match rest.first() {
            None => Ok(json!({})),
            Some(word) if word.contains('=') => pairs(rest),
            Some(word) => match word.parse::<f64>() {
                Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => Ok(json!({ "seconds": seconds })),
                Ok(_) => Err(Error::Config("wait seconds must be finite and nonnegative".into())),
                Err(_) => Ok(json!({ "text": rest.join(" ") })),
            },
        },
        changing: true,
        run: run!(crate::tools::conditions::wait),
    },
    Command {
        name: "verify",
        usage: "[text=…] [gone=…] [url=…] [ref=@ref]",
        about: "Check that the page is now as expected and fail if it is not. text and gone each take \
                one phrase or a list, and every phrase is its own condition, so a step with several \
                constraints is one checkpoint that names which of them did not hold.",
        parse: pairs,
        changing: false,
        run: run!(crate::tools::conditions::verify),
    },
    Command {
        name: "tab",
        usage: "list|check|tidy | new [URL] | switch|close|keep|unkeep|give|restore [tN] | use N|ID",
        about: "This session's tabs, each by a handle t1, t2, … it keeps for the session: list them, \
                switch to or close one, or open a new one at an optional url; a window one of them \
                opens later is this session's too. Other sessions' tabs are one count with their names. \
                Tabs no session holds, such as a person's, are listed apart and numbered: use borrows \
                one, which is never closed — close gives it back — and from which navigate URL opens a \
                new tab. give tN to=SESSION hands a tab to another session. keep marks a tab — the \
                current one unless named — as evidence, with an optional label=TEXT, and unkeep undoes \
                it; navigate URL on a kept tab opens a new tab for it, and a kept tab lost with the \
                browser is reopened at its address; one gone since an earlier run of the daemon \
                waits for restore; a tab that crashed is reloaded. check looks at every \
                tab now — open, loaded, an error page or refusal, a wall, empty, still at its kept \
                address, closed or crashed, reopening nothing — and saves a screenshot of each kept \
                one; tidy closes the tabs not kept. action=, id=, url=, label= and to= say it as pairs.",
        parse: |rest| {
            let (named, words): (Vec<String>, Vec<String>) = rest.iter().cloned().partition(|word| {
                word.split_once('=')
                    .is_some_and(|(key, _)| matches!(key, "action" | "id" | "url" | "label" | "to"))
            });
            let mut options = pairs(&named)?;
            let mut words = words.into_iter();
            if options.get("action").is_none()
                && let Some(action) = words.next()
            {
                options["action"] = json!(action);
            }
            if let Some(target) = words.next() {
                let key = if options["action"] == "new" { "url" } else { "id" };
                options[key] = json!(target);
            }
            if let Some(extra) = words.next() {
                return Err(Error::Config(format!("tab takes one target, not also {extra:?}")));
            }
            // An id, label or session of all digits is still text, not a number.
            for key in ["id", "label", "to"] {
                let prefix = format!("{key}=");
                if let Some(raw) = named.iter().rev().find_map(|arg| arg.strip_prefix(prefix.as_str())) {
                    options[key] =
                        Value::String(serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_owned()));
                }
            }
            Ok(options)
        },
        changing: false,
        run: run!(crate::tools::tabs::tab),
    },
    Command {
        name: "upload",
        usage: "ref=@ref path=PATH",
        about: "Attach a file from the workspace to a file input, by ref.",
        parse: pairs,
        changing: true,
        run: run!(crate::tools::files::upload),
    },
    Command {
        name: "eval",
        usage: "EXPRESSION | - [--raw]",
        about: "Evaluate a JavaScript expression in the page and return its JSON value; - reads the \
                expression from stdin, and --raw prints the value as one line of JSON. It runs in a \
                caller-specific isolated world, separate from page globals and from internal helpers; \
                DOM changes are shared and globals persist on the same document. Use the pointer and \
                keyboard commands for trusted input — a scripted write may not notify a framework.",
        parse: |rest| Ok(json!({ "expression": rest.join(" ") })),
        changing: true,
        run: run!(crate::tools::script::eval),
    },
    Command {
        name: "cdp",
        usage: "METHOD ['{json params}']",
        about: "The escape hatch: send a raw Chrome DevTools Protocol method to the tab and get its \
                result. The tab's own session is used, so page domains (DOM, Page, Input, Network, \
                Emulation) are in scope.",
        parse: |rest| {
            let (method, params) = rest.split_first().ok_or_else(|| needs("cdp", "a CDP method"))?;
            let params: Value = match params {
                [] => json!({}),
                [only] => serde_json::from_str(only)
                    .map_err(|error| Error::Config(format!("cdp params are not JSON: {error}")))?,
                _ => return Err(Error::Config("cdp takes one JSON params object".into())),
            };
            if !params.is_null() && !params.is_object() {
                return Err(Error::Config("cdp params must be an object".into()));
            }
            Ok(json!({ "method": method, "params": params }))
        },
        changing: false,
        run: run!(crate::tools::script::cdp),
    },
    Command {
        name: "console",
        usage: "[errors]",
        about: "What the current tab logged — errors, blocked requests, CSP violations — as untrusted \
                data, for when a step did not do what was expected. A rolling window; errors keeps only \
                the error lines.",
        parse: |rest| Ok(json!({ "errors": rest.iter().any(|word| word == "errors") })),
        changing: false,
        run: run!(crate::tools::script::console),
    },
    Command {
        name: "network",
        usage: "[FIND] [body=true] [max_bytes=N]",
        about: "The data the page fetched for itself — XHR, fetch, event-stream and socket messages — as \
                untrusted data: the JSON behind a map, a chart, a search or a feed, which says more than \
                the pixels drawn from it. Lists the responses whose address has FIND; body=true reads \
                the newest three, saving a long one whole under .browser-cmd/. The first call on a tab \
                starts recording unless BROWSER_CMD_NETWORK=1 already did.",
        parse: find_then_pairs,
        changing: false,
        run: run!(crate::tools::network::network),
    },
    Command {
        name: "retry-clean",
        usage: "[keep_signin=true]",
        about: "Open the current URL again in a fresh isolated browser context with no cookies or cache, \
                when the profile's session, cache or reputation has poisoned the page. keep_signin \
                transfers and verifies this site's cookies; check authentication on the page that comes \
                back.",
        parse: pairs,
        changing: false,
        run: run!(crate::tools::context::retry_clean),
    },
    Command {
        name: "close",
        usage: "",
        about: "End this session: close its tabs, give back borrowed ones, and leave kept ones open. The \
                browser and its profile stay warm for the next session.",
        parse: |_| Ok(json!({})),
        changing: false,
        run: run!(crate::tools::tabs::close),
    },
];

/// A click or hover target: a `@ref`, or a name, is a ref; `X,Y` is a point.
fn aim(name: &'static str, rest: &[String], click: bool) -> Result<Value> {
    let mut end = rest.len();
    while click
        && end > 0
        && rest[end - 1].split_once('=').is_some_and(|(key, _)| matches!(key, "double" | "right" | "again"))
    {
        end -= 1;
    }
    let options = &rest[end..];
    let rest = &rest[..end];
    let word = rest.join(" ");
    if word.trim().is_empty() {
        return Err(needs(name, "a ref, a name, or coordinates"));
    }
    let point = word
        .split_once(',')
        .and_then(|(x, y)| Some((x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?)));
    let mut target = point.map_or_else(|| json!({ "ref": word }), |(x, y)| json!({ "x": x, "y": y }));
    for option in options {
        let (key, raw) = option.split_once('=').expect("the click option suffix was checked above");
        target[key] = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()));
    }
    Ok(target)
}

/// A leading word without `=` is what to find; the rest are pairs.
fn find_then_pairs(rest: &[String]) -> Result<Value> {
    Ok(match rest.first() {
        Some(find) if !find.contains('=') => {
            let mut args = pairs(&rest[1..])?;
            args["find"] = json!(find);
            args
        }
        _ => pairs(rest)?,
    })
}

/// `key=value` words, each value read as JSON where it parses as one.
fn pairs(rest: &[String]) -> Result<Value> {
    let mut object = serde_json::Map::new();
    for pair in rest {
        let (key, value) =
            pair.split_once('=').ok_or_else(|| Error::Config(format!("expected key=value, got {pair:?}")))?;
        let parsed = serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned()));
        object.insert(key.to_owned(), parsed);
    }
    Ok(Value::Object(object))
}

fn one(name: &'static str, rest: &[String]) -> Result<String> {
    match rest {
        [only] => Ok(only.clone()),
        _ => Err(needs(name, "one argument")),
    }
}

fn needs(name: &str, what: &str) -> Error {
    Error::Config(format!("{name} needs {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(name: &str, rest: &[String]) -> Result<Value> {
        find(name).unwrap_or_else(|| panic!("no such command: {name}")).arguments(rest)
    }

    #[test]
    fn tab_ids_remain_strings_even_when_their_characters_are_all_digits() {
        let id = "12345678901234567890123456789012";
        let options = arguments("tab", &["action=switch".into(), format!("id={id}")]).unwrap();
        assert_eq!(options["id"], id);
        let options = arguments("tab", &["action=switch".into(), format!("id=\"{id}\"")]).unwrap();
        assert_eq!(options["id"], id);
    }

    #[test]
    fn tab_takes_its_action_and_target_as_words_or_pairs() {
        let tab =
            |words: &[&str]| arguments("tab", &words.iter().map(|word| word.to_string()).collect::<Vec<_>>());
        assert_eq!(tab(&["list"]).unwrap(), json!({"action":"list"}));
        assert_eq!(tab(&["switch", "3a9f1c"]).unwrap(), json!({"action":"switch", "id":"3a9f1c"}));
        assert_eq!(tab(&["switch", "482616"]).unwrap(), json!({"action":"switch", "id":"482616"}));
        assert_eq!(tab(&["close", "t1"]).unwrap(), json!({"action":"close", "id":"t1"}));
        assert_eq!(tab(&["action=close", "1"]).unwrap(), json!({"action":"close", "id":"1"}));
        assert_eq!(
            tab(&["keep", "t2", "label=Price page"]).unwrap(),
            json!({"action":"keep", "id":"t2", "label":"Price page"})
        );
        assert_eq!(tab(&["keep"]).unwrap(), json!({"action":"keep"}));
        assert_eq!(
            tab(&["new", "https://a.test/?q=x"]).unwrap(),
            json!({"action":"new", "url":"https://a.test/?q=x"})
        );
        assert_eq!(
            tab(&["new", "url=https://a.test/?q=x"]).unwrap(),
            json!({"action":"new", "url":"https://a.test/?q=x"})
        );
        assert_eq!(tab(&["action=list"]).unwrap(), json!({"action":"list"}));
        assert!(tab(&["switch", "1", "2"]).is_err());
    }

    #[test]
    fn only_a_command_that_acts_on_a_tab_brings_tabs_back_first() {
        let acts = |name: &str, args: Value| find(name).unwrap().acts(&args);
        assert!(!acts("tab", json!({"action":"list"})) && !acts("tab", json!({"action":"check"})));
        assert!(acts("tab", json!({"action":"switch","id":"t1"})) && acts("tab", json!({"action":"tidy"})));
        assert!(!acts("console", json!({})) && !acts("network", json!({})) && !acts("close", json!({})));
        assert!(acts("snapshot", json!({})) && acts("navigate", json!({"url":"https://a.test/"})));
    }

    #[test]
    fn scroll_takes_a_ref_as_a_word() {
        let scroll = |words: &[&str]| {
            arguments("scroll", &words.iter().map(|word| word.to_string()).collect::<Vec<_>>())
        };
        assert_eq!(scroll(&["@e1a.3"]).unwrap(), json!({"ref":"@e1a.3"}));
        assert_eq!(scroll(&["@e1a.3", "down"]).unwrap(), json!({"ref":"@e1a.3", "direction":"down"}));
        assert_eq!(scroll(&["bottom", "@e1a.3"]).unwrap(), json!({"ref":"@e1a.3", "to":"bottom"}));
        assert_eq!(scroll(&["Drop", "here"]).unwrap(), json!({"to":"Drop here"}));
        assert_eq!(scroll(&[]).unwrap(), json!({}));
    }

    #[test]
    fn missing_input_is_distinct_from_intentional_clearing_or_literal_submit_text() {
        for (name, args) in [
            ("click", vec![]),
            ("hover", vec![String::new()]),
            ("type", vec!["@e1a.1".into()]),
            ("select", vec!["@e1a.1".into()]),
        ] {
            assert!(arguments(name, &args).is_err(), "{name}");
        }
        let clear = arguments("type", &["@e1a.1".into(), String::new()]).unwrap();
        assert_eq!((&clear["text"], &clear["submit"]), (&json!(""), &json!(false)));
        let literal = arguments("type", &["@e1a.1".into(), "submit".into()]).unwrap();
        assert_eq!((&literal["text"], &literal["submit"]), (&json!("submit"), &json!(false)));
    }

    #[test]
    fn choose_takes_a_ref_text_and_an_optional_option() {
        let choose = |words: &[&str]| {
            arguments("choose", &words.iter().map(|word| word.to_string()).collect::<Vec<_>>())
        };
        assert_eq!(choose(&["@e1a.2", "Raleigh"]).unwrap(), json!({"ref":"@e1a.2", "text":"Raleigh"}));
        assert_eq!(
            choose(&["@e1a.2", "New", "York", "option=New York, NY, USA"]).unwrap(),
            json!({"ref":"@e1a.2", "text":"New York", "option":"New York, NY, USA"})
        );
        assert!(choose(&["@e1a.2"]).is_err());
        assert!(choose(&[]).is_err());
    }

    #[test]
    fn type_without_a_ref_puts_a_lone_text_at_the_focus() {
        assert!(arguments("type", &[]).is_err());
        assert_eq!(
            arguments("type", &["Toronto".into()]).unwrap(),
            json!({"text":"Toronto", "submit":false})
        );
        assert_eq!(
            arguments("type", &["New York".into(), "submit".into()]).unwrap(),
            json!({"text":"New York", "submit":true})
        );
        assert_eq!(
            arguments("type", &["Search".into(), "kettle".into()]).unwrap(),
            json!({"ref":"Search", "text":"kettle", "submit":false})
        );
    }

    #[test]
    fn malformed_cdp_params_are_not_silently_replaced_with_an_empty_object() {
        for value in ["not JSON", "[]", "42"] {
            assert!(arguments("cdp", &["Browser.close".into(), value.into()]).is_err());
        }
        assert!(arguments("cdp", &["Browser.close".into()]).is_ok());
        assert!(arguments("cdp", &["Browser.close".into(), "{}".into()]).is_ok());
    }

    #[test]
    fn a_lone_json_object_is_passed_through_for_every_command() {
        let given = arguments("click", &[r#"{"ref":"@e1a.1","double":true}"#.into()]).unwrap();
        assert_eq!(given["double"], true);
        assert!(arguments("click", &["{not json".into()]).is_err());
    }

    #[test]
    fn click_shorthand_keeps_documented_options_out_of_the_target() {
        let by_ref =
            arguments("click", &["@e1a.1".into(), "double=true".into(), "again=true".into()]).unwrap();
        assert_eq!(by_ref, json!({"ref":"@e1a.1", "double":true, "again":true}));

        let by_name = arguments("click", &["Buy".into(), "now".into(), "right=true".into()]).unwrap();
        assert_eq!(by_name, json!({"ref":"Buy now", "right":true}));

        let by_point = arguments("click", &["120,40".into(), "right=true".into()]).unwrap();
        assert_eq!(by_point, json!({"x":120.0, "y":40.0, "right":true}));

        let hover = arguments("hover", &["Menu".into(), "right=true".into()]).unwrap();
        assert_eq!(hover, json!({"ref":"Menu right=true"}));
    }

    #[test]
    fn screenshot_and_evidence_take_their_targets_as_words() {
        assert_eq!(arguments("screenshot", &[]).unwrap(), json!({"full_page":false}));
        assert_eq!(
            arguments("screenshot", &["@e3".into()]).unwrap(),
            json!({"full_page":false, "ref":"@e3"})
        );
        assert_eq!(
            arguments("screenshot", &["selector=div.price span".into()]).unwrap(),
            json!({"full_page":false, "selector":"div.price span"})
        );
        assert_eq!(arguments("screenshot", &["full".into()]).unwrap()["full_page"], true);
        assert_eq!(
            arguments("screenshot", &["10,20,300,150".into(), "zoom=2".into()]).unwrap(),
            json!({"full_page":false, "region":[10.0, 20.0, 300.0, 150.0], "zoom":2})
        );
        assert_eq!(
            arguments("evidence", &["$12.50".into(), "in stock".into()]).unwrap(),
            json!({"values":["$12.50", "in stock"]})
        );
        assert!(arguments("evidence", &[]).is_err());
    }

    #[test]
    fn names_are_unique_and_every_command_says_what_it_is_for() {
        for command in CATALOGUE {
            assert!(!command.about.is_empty(), "{} has no description", command.name);
            assert_eq!(
                CATALOGUE.iter().filter(|other| other.name == command.name).count(),
                1,
                "{} is listed twice",
                command.name
            );
            assert!(!LOCAL.contains(&command.name), "{} is both local and remote", command.name);
        }
    }
}

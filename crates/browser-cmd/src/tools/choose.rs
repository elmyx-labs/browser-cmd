//! Choosing one of the suggestions an autocomplete offers: typed, waited for,
//! picked, and checked to have taken.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

use crate::chrome::tab::Tab;
use crate::input::keys::press;
use crate::page::frames::Epoch;
use crate::page::js::{COMMIT, OPTION, SUGGESTIONS};
use crate::session::{Session, after};
use crate::tools::aim::{aim, covered_by};
use crate::tools::args::args;
use crate::tools::keyboard::{enter, select};
use crate::{Error, Result};

/// How long suggestions get to arrive after typing, and when a field that has
/// shown none is nudged once with its last character again.
const ARRIVE: Duration = Duration::from_secs(6);
const NUDGE: Duration = Duration::from_secs(3);
/// How long a list must have stopped changing to be the answer to what was typed.
const QUIET_MS: f64 = 300.0;
const LOOK: Duration = Duration::from_millis(100);
/// How long a pick gets to show that it took.
const COMMIT_WAIT: Duration = Duration::from_secs(2);
/// The score a best match needs to be picked without asking.
const SURE: u32 = 80;
/// How many suggestions a reply names.
const SHOWN: usize = 5;

#[derive(Deserialize)]
struct Suggestion {
    text: String,
    /// The text as compared: no accents, case or punctuation.
    key: String,
    disabled: bool,
}

pub(crate) async fn choose(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args { r#ref: String, text: String, option: Option<String> });
    let reference = args.r#ref.as_str();
    let wanted = args.option.as_deref().unwrap_or(&args.text);
    let node = session.field(reference)?;
    let tab = session.current().await?;
    let start = vec![json!("start"), json!([args.text, wanted])];
    let started = session.document.world.call_on(&tab, node, SUGGESTIONS, start).await?;
    if started["select"] == true {
        return select(session, json!({ "ref": reference, "option": wanted })).await;
    }
    let words: Vec<String> = serde_json::from_value(started["words"].clone())?;
    if let Ok(listed) = serde_json::from_value::<Vec<Suggestion>>(started["datalist"].clone()) {
        let index = sure(&listed, &words[1]).map_err(|close| {
            let names: Vec<String> = close.iter().map(|index| format!("{:?}", listed[*index].text)).collect();
            unsure(reference, wanted, &names, listed.len())
        })?;
        let text = &listed[index].text;
        if let Some(now) = enter(session, &tab, reference, text).await? {
            return Err(Error::Tool(format!("typed {text:?} into {reference}, but it reads {now:?}")));
        }
        let did = format!("Chose {text:?} in {reference} from its list of {}", listed.len());
        return after(session, &tab, &did).await;
    }
    // A key dropped while the page was busy is a different word to search for.
    if enter(session, &tab, reference, &args.text).await?.is_some() {
        enter(session, &tab, reference, &args.text).await?;
    }
    let options = suggestions(session, &tab, node, &args.text, &words).await?;
    if options.is_empty() {
        let limit = session.settings().limits.new_text;
        let said =
            session.document.new_text(&tab, limit).await.map_or_else(String::new, |said| format!("\n{said}"));
        return Err(Error::Tool(format!(
            "no suggestions appeared under {reference} within {}s of typing {:?}. Try the words the site \
             knows it by (a city's name rather than its postcode); a plain search box takes \
             type {reference} \"…\" submit instead.{said}",
            ARRIVE.as_secs(),
            args.text
        )));
    }
    if !options.iter().any(|option| words.iter().any(|word| score(&option.key, word) > 0)) {
        let names = options.iter().take(SHOWN).map(|option| format!("{:?}", option.text)).collect::<Vec<_>>();
        return Err(Error::Tool(format!(
            "the suggestions under {reference} do not match {wanted:?}: {}. Repeat with other text, or with \
             option=\"…\" naming one of them.",
            names.join(", ")
        )));
    }
    let index = match sure(&options, &words[1]) {
        Ok(index) => index,
        Err(close) => {
            // The candidates by their refs, which a fresh state gives them.
            session.document.state(&tab, false, true).await?;
            let mut names = Vec::new();
            for index in close {
                let found =
                    session.document.world.node_result(&tab, None, OPTION, vec![json!(index)]).await?;
                let handle = found.node.and_then(|node| session.document.refs.handle_for(node));
                let handle = handle.map_or_else(String::new, |handle| format!("@{handle} "));
                names.push(format!("{handle}{:?}", options[index].text));
            }
            return Err(unsure(reference, wanted, &names, options.len()));
        }
    };
    let (seen, did) = pick(session, &tab, node, index, &options[index].key, options.len()).await?;
    let (number, count) = (index + 1, options.len());
    let did = match did {
        "keyboard" => format!("picked suggestion {number} of {count} with the keyboard"),
        verb => format!("{verb} suggestion {number} of {count}"),
    };
    let reads = seen["reads"].as_str().unwrap_or_default();
    let took = match seen["how"].as_str() {
        None => {
            let open = if seen["open"] == true { " and the list is still open" } else { "" };
            return Err(Error::Tool(format!(
                "{did} under {reference}, {:?}, but it did not take: the field reads {reads:?}{open}. Click \
                 the option yourself and check the field, or type the whole of it.",
                options[index].text
            )));
        }
        Some("value") => format!("the field reads {reads:?} and the list closed"),
        Some("selected") => "the list marks it selected".to_owned(),
        Some("shown") => "it now shows by the field".to_owned(),
        Some("hidden") => "the form's hidden value for it changed".to_owned(),
        Some(_) => "the page moved on".to_owned(),
    };
    let chose =
        format!("Chose {:?} in {reference} — typed {:?}, {did}; {took}", options[index].text, args.text);
    after(session, &tab, &chose).await
}

/// The suggestions shown once they answer what was typed: changed since the
/// typing began, quiet for a moment, and sharing a word with it — or whatever
/// is there when the wait runs out.
async fn suggestions(
    session: &mut Session,
    tab: &Tab,
    node: i64,
    text: &str,
    words: &[String],
) -> Result<Vec<Suggestion>> {
    let typed = Instant::now();
    let mut nudged = false;
    loop {
        let seen =
            session.document.world.call_on(tab, node, SUGGESTIONS, vec![json!("look"), json!([])]).await?;
        let options: Vec<Suggestion> = serde_json::from_value(seen["options"].clone())?;
        let related = options.iter().any(|option| words.iter().any(|word| score(&option.key, word) > 0));
        if related && seen["fresh"] == true && seen["quiet"].as_f64().is_some_and(|quiet| quiet >= QUIET_MS)
            || typed.elapsed() >= ARRIVE
        {
            return Ok(options.into_iter().filter(|option| !option.key.is_empty()).collect());
        }
        if options.is_empty()
            && !nudged
            && typed.elapsed() >= NUDGE
            && let Some(last) = text.chars().last()
        {
            nudged = true;
            press(tab, &mut session.hand, "Backspace").await?;
            session.hand.type_text(tab, &last.to_string()).await?;
        }
        sleep(LOOK).await;
    }
}

/// Click the suggestion, and if that does not take while the field still has
/// focus, walk the list with ArrowDown to it and press Enter — only once it is
/// the active one, so the Enter picks it rather than sending the form.
async fn pick(
    session: &mut Session,
    tab: &Tab,
    node: i64,
    index: usize,
    key: &str,
    count: usize,
) -> Result<(Value, &'static str)> {
    let option = session.document.world.node_result(tab, None, OPTION, vec![json!(index)]).await?.node;
    let mut did = "could not reach";
    if let Some(option) = option
        && let Ok((x, y)) = aim(session, tab, option).await
    {
        session.hand.move_to(tab, x, y).await?;
        if covered_by(session, tab, option, x, y).await?.is_none() {
            session.hand.press(tab, x, y, 1).await?;
            did = "clicked";
        }
    }
    let wait = if did == "clicked" { COMMIT_WAIT } else { Duration::ZERO };
    let mut seen = taken(session, tab, node, index, key, wait).await?;
    if !seen["how"].is_null() || seen["focused"] != true {
        return Ok((seen, did));
    }
    for _ in 0..=count {
        if seen["active"] == json!(index) {
            press(tab, &mut session.hand, "Enter").await?;
            return Ok((taken(session, tab, node, index, key, COMMIT_WAIT).await?, "keyboard"));
        }
        press(tab, &mut session.hand, "ArrowDown").await?;
        seen = taken(session, tab, node, index, key, Duration::ZERO).await?;
    }
    Ok((seen, did))
}

/// Whether the pick took, looked at until it has or `wait` runs out. A page
/// that went elsewhere has taken it.
async fn taken(
    session: &mut Session,
    tab: &Tab,
    node: i64,
    index: usize,
    key: &str,
    wait: Duration,
) -> Result<Value> {
    let epoch = Epoch::of(tab);
    let deadline = Instant::now() + wait;
    loop {
        let seen = session.document.world.call_on(tab, node, COMMIT, vec![json!(index), json!(key)]).await;
        if epoch != Epoch::of(tab) {
            return Ok(json!({ "how": "navigated" }));
        }
        let seen = seen?;
        if !seen["how"].is_null() || Instant::now() >= deadline {
            return Ok(seen);
        }
        sleep(LOOK).await;
    }
}

/// The suggestion to pick for `query`: an exact match, or the best when it is
/// sure and alone at the top. Otherwise the candidates tied at the top.
fn sure(options: &[Suggestion], query: &str) -> std::result::Result<usize, Vec<usize>> {
    let mut ranked: Vec<(u32, usize)> = options
        .iter()
        .enumerate()
        .filter(|(_, option)| !option.disabled)
        .map(|(index, option)| (score(&option.key, query), index))
        .collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.0));
    match ranked.as_slice() {
        [(100, index), ..] => Ok(*index),
        [(best, index), rest @ ..] if *best >= SURE && rest.first().is_none_or(|next| next.0 < *best) => {
            Ok(*index)
        }
        _ => {
            let top = ranked.first().map_or(0, |best| best.0);
            Err(ranked
                .iter()
                .filter(|(score, _)| *score == top)
                .take(SHOWN)
                .map(|(_, index)| *index)
                .collect())
        }
    }
}

fn unsure(reference: &str, wanted: &str, names: &[String], total: usize) -> Error {
    let more = total - names.len();
    Error::Tool(format!(
        "{wanted:?} fits more than one suggestion under {reference}, or none well: {}{}. Repeat with \
         option=\"…\" naming the one meant, or click one.",
        names.join(", "),
        if more > 0 { format!(" (+{more} more)") } else { String::new() }
    ))
}

/// How well an option's text matches a query, both already compared as words:
/// 100 the same, 90 the option starts with the whole query, 80 each query word
/// begins a word of the option, else 40 for the share of query words that do.
fn score(option: &str, query: &str) -> u32 {
    let words: Vec<&str> = query.split_whitespace().collect();
    if option == query {
        return 100;
    }
    if option.strip_prefix(query).is_some_and(|rest| rest.starts_with(' ')) {
        return 90;
    }
    let begun =
        words.iter().filter(|word| option.split_whitespace().any(|own| own.starts_with(**word))).count();
    match (begun, words.len()) {
        (_, 0) => 0,
        (begun, all) if begun == all => 80,
        (begun, all) => (40 * begun / all) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(texts: &[&str]) -> Vec<Suggestion> {
        texts
            .iter()
            .map(|text| Suggestion { text: (*text).into(), key: (*text).into(), disabled: false })
            .collect()
    }

    #[test]
    fn a_score_ranks_the_same_over_a_prefix_over_word_starts_over_a_share() {
        assert_eq!(score("raleigh nc usa", "raleigh nc usa"), 100);
        assert_eq!(score("raleigh nc usa", "raleigh"), 90);
        assert_eq!(score("raleighwood nc", "raleigh"), 80);
        assert_eq!(score("north raleigh nc", "raleigh nc"), 80);
        assert_eq!(score("raleigh nc", "raleigh ms"), 20);
        assert_eq!(score("boston ma usa", "raleigh"), 0);
        assert_eq!(score("boston", ""), 0);
    }

    #[test]
    fn one_sure_match_is_picked_and_a_tie_or_a_guess_is_asked_about() {
        let places = options(&["raleigh nc usa", "raleigh ms usa", "raleigh hills or usa"]);
        assert_eq!(sure(&places, "raleigh ms"), Ok(1));
        assert_eq!(sure(&places, "raleigh ms usa"), Ok(1));
        assert_eq!(sure(&places, "raleigh"), Err(vec![0, 1, 2]));
        assert_eq!(sure(&places, "durham"), Err(vec![0, 1, 2]));
        let mut disabled = options(&["raleigh nc usa", "raleigh ms usa"]);
        disabled[1].disabled = true;
        assert_eq!(sure(&disabled, "raleigh"), Ok(0));
    }
}

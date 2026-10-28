//! Many pages read in one call, each in a tab of its own, several at a time,
//! so a walk does not spend a model turn on every page.

use serde_json::Value;

use crate::browser::host::Shared;
use crate::chrome::tab::Tab;
use crate::input::hand::Hand;
use crate::page::navigation::{Landing, navigate};
use crate::page::world::World;
use crate::session::Session;
use crate::tabs::book::Book;
use crate::tabs::ledger::Hold;
use crate::tabs::status::TabStatus;
use crate::text::fence::untrusted;
use crate::tools::args::args;
use crate::web::web_url;
use crate::{Error, Result, Settings};

/// Pages loading at once: enough to overlap slow sites, few enough not to
/// starve each other of the network or look like a crawler.
const AT_ONCE: usize = 4;
const MAX_PAGES: usize = 12;
/// What one page carries back unless asked for another amount.
const PAGE_BYTES: usize = 3000;
/// How much of a title and a first line a page's summary line shows.
const SUMMARY_CHARS: usize = 100;
/// What a page read for its text and then closed does not fetch: pictures, fonts
/// and media are most of a page's bytes and none of its words.
const HEAVY: &[&str] = &[
    "*.png*", "*.jpg*", "*.jpeg*", "*.gif*", "*.webp*", "*.avif*", "*.ico*", "*.woff*", "*.ttf*", "*.otf*",
    "*.mp4*", "*.webm*", "*.m3u8*", "*.mp3*",
];

pub(crate) async fn pages(session: &mut Session, args: Value) -> Result<String> {
    let args = args!(args, Args, #[serde(deny_unknown_fields)] {
        urls: Vec<String>,
        find: Option<String>,
        max_bytes: Option<usize>,
        #[serde(default)]
        keep: bool,
        /// The sites' headers, menus, footers and cookie notices as well.
        #[serde(default)]
        full: bool,
    });
    if args.urls.is_empty() || args.urls.len() > MAX_PAGES {
        return Err(Error::Config(format!("pages takes 1 to {MAX_PAGES} URLs")));
    }
    let urls = args.urls.iter().map(|url| web_url(url)).collect::<Result<Vec<_>>>()?;
    let host = session.host.clone();
    let settings = host.settings();
    let shared = host.shared().await?;
    session.book.reconcile(&shared).await?;
    let each = args.max_bytes.unwrap_or(PAGE_BYTES).min(settings.limits.page_text / urls.len()).max(200);
    let find = args.find.as_deref().map(str::trim).filter(|find| !find.is_empty());
    let skip = (!args.full && find.is_none()).then_some("`pages URL full=true` reads them");
    let mut read = Vec::new();
    for batch in urls.chunks(AT_ONCE) {
        let reading = batch.iter().enumerate().map(|(index, url)| {
            one(&shared, settings, &session.book, url.as_str(), (find, skip), each, args.keep, index)
        });
        read.extend(futures_util::future::join_all(reading).await);
    }
    // A tab left open is this session's, kept, by handle.
    let out: Vec<String> = urls
        .iter()
        .zip(read)
        .map(|(url, (kept, rest))| {
            let kept = kept.map_or(String::new(), |(target, title, address, status)| {
                let name = session.book.keep_opened(&target, &address, &title, status);
                format!(" (left open as tab {name})")
            });
            format!("{}{kept}\n{rest}", crate::web::shown(url.as_str()))
        })
        .collect();
    let body = out.iter().enumerate().map(|(n, block)| format!("## {}. {block}", n + 1)).collect::<Vec<_>>();
    let body = body.join("\n");
    if body.len() <= settings.limits.text {
        return Ok(untrusted(&body));
    }
    // One line per page first when they run long, so a reader of only the head still sees every page.
    let summary: Vec<String> = out.iter().enumerate().map(|(n, block)| summary(n + 1, block)).collect();
    Ok(untrusted(&format!("{}\n\n{body}", summary.join("\n"))))
}

fn summary(number: usize, block: &str) -> String {
    let mut lines =
        block.lines().skip(1).filter(|line| !line.trim().is_empty() && !line.starts_with("[Skipped "));
    let title = lines.next().unwrap_or_default().trim_start_matches("Title: ");
    let first = lines
        .next()
        .map_or(String::new(), |line| format!(" — {}", crate::text::clip(line.trim(), SUMMARY_CHARS)));
    format!("{number}. {}{first}", crate::text::clip(title, SUMMARY_CHARS))
}

/// One page in a tab of its own, with the same identity, wall handling and
/// tempo as the session's own tab: what it says, or why it could not be read;
/// and a tab left open, with its title, address and status. Tabs of one batch
/// start a person's moment apart rather than all at once.
#[allow(clippy::too_many_arguments)]
async fn one(
    shared: &Shared,
    settings: &Settings,
    book: &Book,
    url: &str,
    (find, skip): (Option<&str>, Option<&str>),
    limit: usize,
    keep: bool,
    index: usize,
) -> (Option<(String, String, String, TabStatus)>, String) {
    let mut hand = Hand::new(settings.pace);
    let stagger = index as f64 * hand.between(0.6, 1.8) * settings.pace;
    tokio::time::sleep(std::time::Duration::from_secs_f64(stagger)).await;
    let tab =
        match crate::tabs::create(shared, settings, book, book.context.as_deref(), Hold::Transient).await {
            Ok(tab) => tab,
            Err(error) => return (None, format!("[not opened: {error}]")),
        };
    if !keep {
        let _ = crate::chrome::traffic::block(&tab, HEAVY).await;
    }
    let read = read(settings, &tab, &mut hand, url, (find, skip), limit).await;
    if !keep {
        let _ = shared.cdp.close_target(&tab.target).await;
    }
    let (title, address, status, said) = match read {
        Ok((title, address, status, notes, body)) => {
            (title.clone(), address, status, format!("Title: {title}\n{notes}{body}"))
        }
        Err(error) => (String::new(), url.to_owned(), TabStatus::Error, format!("[not read: {error}]")),
    };
    (keep.then(|| (tab.target.clone(), title, address, status)), said)
}

async fn read(
    settings: &Settings,
    tab: &Tab,
    hand: &mut Hand,
    url: &str,
    (find, skip): (Option<&str>, Option<&str>),
    limit: usize,
) -> Result<(String, String, TabStatus, String, String)> {
    match navigate(tab, url, settings.waits.navigate).await? {
        Landing::Committed => {}
        Landing::Failed(reason) => return Err(Error::Tool(format!("navigation failed: {reason}"))),
        Landing::NoAnswer => {
            let wait = settings.waits.navigate.as_secs();
            return Err(Error::Tool(format!("did not start answering within {wait} s")));
        }
    }
    let mut world = World::new();
    let landed = crate::session::land::land(&mut world, tab, &settings.waits, hand).await;
    let document = world.call(tab, crate::page::js::DOCUMENT, Vec::new()).await.unwrap_or(Value::Null);
    let text = crate::page::reading::read(&mut world, tab, false, None, true, skip).await?.render();
    let crate::chrome::tab::TargetInfo { title, url: address, .. } = tab.info().await?;
    let (status, refused) = landed.verdict(&address, &document);
    let notes: String = landed.said.iter().chain(&refused).map(|note| format!("{note}\n")).collect();
    let body = match find {
        Some(find) => crate::text::passages::passages(&text, find, settings.limits.passage, limit, true)?,
        None => {
            let shown = crate::text::head(&text, limit);
            match shown.len() < text.len() {
                true => format!("{shown}\n[{} of {} bytes]", shown.len(), text.len()),
                false => shown.to_owned(),
            }
        }
    };
    Ok((title, address, status, notes, body))
}

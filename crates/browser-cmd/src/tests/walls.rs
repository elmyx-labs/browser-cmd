//! What the wall recogniser calls each page: every vendor is named by words
//! and by marks, and this pins the pairing.

use super::*;
use crate::page::walls::{Wall, detect};

/// Patience short enough for a widget that never draws.
fn short_waits(settings: &mut Settings) {
    settings.waits.widget = std::time::Duration::from_millis(100);
    settings.waits.challenge = std::time::Duration::from_secs(2);
}

/// What `detect` makes of one page.
async fn seen(body: &str) -> Option<Wall> {
    on_page_with(serve(body.to_owned()), short_waits, async |session, _| {
        let tab = session.current().await.unwrap();
        detect(&mut World::new(), &tab, &session.settings().waits).await.unwrap()
    })
    .await
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn every_wall_is_recognised_by_its_marks_and_by_its_words() {
    let cases: &[(&str, Option<Wall>)] = &[
        ("<!doctype html><title>Shop</title><button>Buy</button>", None),
        ("<!doctype html><div id='cf-challenge-running'></div>", Some(Wall::Cloudflare)),
        ("<!doctype html><title>Just a moment…</title><p>Verify you are human</p>", Some(Wall::Cloudflare)),
        ("<!doctype html><div id='datadome-captcha'></div>", Some(Wall::DataDome)),
        ("<!doctype html><p>Powered by DataDome</p>", Some(Wall::DataDome)),
        ("<!doctype html><div id='px-captcha'></div>", Some(Wall::PerimeterX)),
        ("<!doctype html><p>Press &amp; Hold to confirm</p>", Some(Wall::PerimeterX)),
        ("<!doctype html><h1>You have been blocked</h1>", Some(Wall::Blocked)),
        (
            "<!doctype html><script src='http://127.0.0.1:9/captcha.awswaf.com/captcha.js'></script>",
            Some(Wall::AwsWaf),
        ),
        ("<!doctype html><h1>Let's confirm you are human</h1>", Some(Wall::AwsWaf)),
        (
            "<!doctype html><script src='http://127.0.0.1:9/token.awswaf.com/challenge.js'></script>",
            Some(Wall::AwsWafChallenge),
        ),
        // The same integration on a page that already let the visitor through.
        (
            "<!doctype html><h1>Models</h1><script src='http://127.0.0.1:9/sdk.awswaf.com/challenge.js'></script>",
            None,
        ),
        // A widget whose iframe never arrives is no wall to stand at.
        ("<!doctype html><div class='g-recaptcha' data-sitekey='x'></div>", None),
    ];
    for (body, expected) in cases {
        assert_eq!(&seen(body).await, expected, "{body}");
    }
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_filled_token_means_the_wall_is_already_behind_us() {
    let solved = "<!doctype html><div id='cf-challenge-running'></div>\
        <input name='cf-turnstile-response' value='a-solved-token'>";
    assert_eq!(seen(solved).await, None);
    // An empty token is not a solved one.
    let pending = "<!doctype html><div id='cf-challenge-running'></div>\
        <input name='cf-turnstile-response' value=''>";
    assert_eq!(seen(pending).await, Some(Wall::Cloudflare));
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_challenge_that_reloads_itself_has_not_cleared() {
    // The same challenge every time, sending itself to a fresh attempt the way a
    // vendor's does: the address and the document change, the wall does not.
    let address = serve(
        "<!doctype html><title>Just a moment...</title><p>Performing security verification</p>\
         <script>setTimeout(() => location.replace('/?__cf_chl_rt_tk=' + Date.now()), 300)</script>"
            .into(),
    );
    let waits = |settings: &mut Settings| {
        short_waits(settings);
        settings.waits.challenge_auto = std::time::Duration::from_secs(3);
        settings.waits.challenge_poll = std::time::Duration::from_millis(50);
    };
    on_page_with(address, waits, async |_, opened| {
        assert!(!opened.output.contains("cleared"), "{}", opened.output);
        assert!(opened.output.contains("Still behind a Cloudflare challenge"), "{}", opened.output);
    })
    .await;
}

#[tokio::test]
#[ignore = "requires installed Chrome; run with --include-ignored"]
async fn a_token_challenge_is_waited_out_rather_than_shown_as_an_empty_page() {
    // Empty while the token is worked out, then the page it was guarding.
    let address = serve(
        "<!doctype html><script src='http://127.0.0.1:9/token.awswaf.com/challenge.js'></script>\
         <script>setTimeout(() => { document.title = 'Chart'; document.body.innerHTML = '<h1>Top chart</h1>' }, 2500)</script>"
            .into(),
    );
    let waits = |settings: &mut Settings| {
        short_waits(settings);
        settings.waits.challenge = std::time::Duration::from_secs(10);
    };
    on_page_with(address, waits, async |_, opened| {
        assert!(opened.output.contains("An AWS WAF challenge cleared on its own"), "{}", opened.output);
        assert!(opened.output.contains("Top chart"), "{}", opened.output);
    })
    .await;
}

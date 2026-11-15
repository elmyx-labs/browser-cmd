//! The command line a launched Chrome is given.

use std::net::SocketAddr;

use crate::Settings;

/// The virtual monitor a headless window sits on: bigger than the window, with
/// a taskbar so the work area is smaller than the screen — the one signal a
/// display-less browser gets wrong that a wall reads as "no desktop here". A
/// Mac reporting its own GPU with a pixel ratio of 1 describes no real machine.
pub(crate) struct Screen {
    pub(crate) size: (u32, u32),
    pub(crate) scale: f64,
    pub(crate) menubar: u32,
    pub(crate) taskbar: u32,
    /// Bits per pixel: a Mac's own display is a wide-gamut one.
    pub(crate) depth: u32,
}

pub(crate) const SCREEN: Screen = if cfg!(target_os = "macos") {
    Screen { size: (1512, 982), scale: 2.0, menubar: 33, taskbar: 98, depth: 30 }
} else {
    Screen { size: (1920, 1080), scale: 1.0, menubar: 0, taskbar: 48, depth: 24 }
};

/// What every launched Chrome is told, beside what the settings add.
const ARGUMENTS: &[&str] = &[
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-search-engine-choice-screen",
    // An agent reads pages; it never listens. Autoplaying media would play
    // through the person's speakers, headless or not.
    "--mute-audio",
    // Quiet the parts of Chrome that phone home or pop a bubble, none of which
    // a driven browser wants, several of which are a tell.
    "--no-service-autorun",
    "--disable-breakpad",
    "--disable-session-crashed-bubble",
    "--disable-infobars",
    // Small /dev/shm on a Linux server or in Docker crashes a renderer.
    "--disable-dev-shm-usage",
    // Cross-origin iframes render in the page's own process where Chrome
    // allows it, so a frame is read like any same-site one and carries the
    // page's identity; the ones Chrome still gives a process are given it as
    // they start (`chrome::events`).
    // A fresh profile would also fetch Chrome's own models, ~45 MB each time,
    // through a metered proxy when there is one. Its components stay: without
    // them Widevine is gone, which a page can ask about.
    "--disable-features=IsolateOrigins,site-per-process,OptimizationGuideModelDownloading,OptimizationHintsFetching,OptimizationTargetPrediction,OptimizationHints",
    // The system keychain would otherwise ask the person, mid-turn.
    if cfg!(target_os = "macos") { "--use-mock-keychain" } else { "--password-store=basic" },
];

/// `port` is where Chrome listens for DevTools. A port Chrome picks itself
/// (`0`) sets `navigator.webdriver`, as headless does; a named one does not.
pub(super) fn arguments(settings: &Settings, proxy: Option<SocketAddr>, port: u16) -> Vec<String> {
    let mut args = vec![
        format!("--remote-debugging-port={port}"),
        format!("--user-data-dir={}", settings.profile.display()),
        // Native process configuration reaches headed pages and workers too.
        format!("--accept-lang={}", settings.language),
    ];
    args.extend(ARGUMENTS.iter().map(|arg| (*arg).to_owned()));
    args.extend(settings.chrome_args.iter().cloned());
    if let Some(address) = proxy {
        match settings.proxy_hosts.is_empty() {
            // Chromium otherwise bypasses the proxy for loopback names,
            // which would turn a hostile public page into host SSRF.
            true => args.extend([
                format!("--proxy-server=http://{address}"),
                "--proxy-bypass-list=<-loopback>".into(),
            ]),
            false => args.push(format!(
                "--proxy-pac-url={}",
                crate::chrome::proxy::pac(&settings.proxy_hosts, address)
            )),
        }
        args.extend([
            "--disable-quic".into(),
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
            // An upstream is paid by the byte: no video plays until someone starts it.
            "--autoplay-policy=user-gesture-required".into(),
        ]);
    }
    if settings.headless {
        // Headless Chrome opens at 800x600, which no laptop has — and sizing
        // only the window leaves a viewport larger than the screen it is
        // supposedly on, which no machine has either. The virtual screen is
        // told the same size, so the two agree without emulating a device.
        // Its numbers are device pixels, so each is scaled by the ratio.
        let device = |css: u32| (css as f64 * SCREEN.scale).round();
        let screen = (SCREEN.size.0.max(settings.window.0), SCREEN.size.1.max(settings.window.1));
        args.extend([
            "--headless=new".into(),
            // Headless sets `navigator.webdriver`, which no ordinary user's
            // browser does. Only here: where a window is shown the flag puts
            // a warning bar across it, which shortens the page and is seen.
            "--disable-blink-features=AutomationControlled".into(),
            format!("--window-size={},{}", settings.window.0, settings.window.1),
            format!(
                "--screen-info={{{}x{} devicePixelRatio={} colorDepth={} workAreaTop={} workAreaBottom={}}}",
                device(screen.0),
                device(screen.1),
                SCREEN.scale,
                SCREEN.depth,
                device(SCREEN.menubar),
                device(SCREEN.taskbar)
            ),
        ]);
    }
    if settings.runs_behind() {
        args.extend([
            "--disable-background-timer-throttling".into(),
            "--disable-backgrounding-occluded-windows".into(),
            "--disable-renderer-backgrounding".into(),
        ]);
    }
    if settings.no_sandbox {
        args.push("--no-sandbox".into());
    }
    args.push("about:blank".into());
    args
}

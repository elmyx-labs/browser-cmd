# Changelog

## Unreleased

What surrounds the commands: telling builds apart, releasing, and finding out what went wrong.

- `--version` and the daemon name the commit they were built from, so a daemon left running by an older
  build of the same version is replaced by the next command instead of answering it.
- `doctor` says what a command would run into without starting a browser: the build and where it is, the
  Chrome it would launch, the daemon, and whether the skill each agent reads is this build's.
- The agent skill is carried in the binary: `skill` prints it and `skill install` writes it for Claude
  Code, Codex and Pi.
- A tab that left one browser call unanswered gets a short wait for the next ones until it answers
  again, so a command on a stuck tab ends in about half a minute instead of at the 100 s deadline.
- A command that waited behind another one, or found every session taken, says it was not started and
  why, instead of an unconfirmed outcome.
- A failure trace leaves out what `type` typed, is readable by its owner only, and the last 50 are kept;
  writing it no longer waits on a daemon that did not answer.
- The daemon keeps a log of its start and of the error or panic it ended on, in
  `~/.browser-cmd/daemon-ADDRESS.log`, and a daemon that dies at startup points to it.
- No stable Chrome found: the message names the Chromium, Beta or "for Testing" builds it passed over.
  Chrome leaving over its sandbox: the message names `BROWSER_CMD_NO_SANDBOX`.
- Something that is not browser-cmd on the daemon's port is reported as that, and is no longer asked to
  restart.
- Releases run the checks CI runs before building, and start each binary once before publishing it; CI
  runs the real-Chrome tests on Linux.

What a page sees of the browser and of the hand on it, measured against an ordinary Chrome on the same
machine and brought to match it.

- Launch: `Notification` and push are there again, Widevine works, a link's `ping` is sent, and a headed
  window no longer carries Chrome's warning bar about an unsupported flag. Chrome's own components are
  updated as in any Chrome, also through a proxy when one is set.
- Headless: a frame from another site and a window a page opens name the same browser as the page from
  their first request, and the virtual screen has a Mac display's colour depth. A sandboxed frame and a
  shared or service worker can still name themselves headless, and a headless browser still answers a
  permission request, `print()` and fullscreen as no window does: for a site that checks, use a headed
  browser.
- Pointer: a pressed button reports a mouse's pressure, a double click has a hand's gap between its two
  presses, travel speeds up and slows down, a press lands near the middle of its target more often than
  near its edge, and an element out of view is scrolled to with the wheel, in the notches a wheel turns in.
- Keyboard: a key is held as long as a finger holds it, a capital or a symbol presses Shift first, every
  key of a US keyboard is typed as a key, and typing starts a moment after the click that focused the
  field.
- A JavaScript dialog is answered after the time it takes to read it, scaled by `BROWSER_CMD_PACE`.
- `upload` clicks a file input a person could click and answers its chooser; an input a page hides is
  handed its file directly, as before.
- A window a page opens takes the focus from the page that opened it at once.

## 0.1.0

First public release.

- `browser-cmd`: a single binary that drives an installed Google Chrome over the DevTools Protocol through
  a warm daemon, with the page as an accessibility tree with refs that stay valid across commands.
- Commands to navigate, click, hover, drag, type, choose and select, press keys, scroll, read text,
  extract rows, read the page's own network data, take screenshots, check evidence, manage tabs, upload
  files, evaluate JavaScript and send raw DevTools calls; `batch` runs several in one call.
- Attach to a running Chrome (`BROWSER_CMD_CDP`), or to the Chrome you are signed in to
  (`BROWSER_CMD_CHROME=mine`, through Chrome's own Remote Debugging switch in `chrome://inspect`), with
  `tab use active` for the tab in front of you; upstream HTTP and SOCKS5 proxies with credentials; and
  per-workspace sessions.
- A headed browser shows a page what a person's browser would: the agent's tabs each have a window of
  their own behind the person's, shown and running there, the one the agent works in is the one told it
  has the focus, and nothing is brought in front of the person unless they ask (`BROWSER_CMD_FOLLOW`).
  Headless, tabs share a window and are brought to the front freely, as before.
- A daemon of another build of browser-cmd is replaced by the next command, not spoken to.
- Settings in `~/.browser-cmd/config.toml` and `<workspace>/.browser-cmd/config.toml` as well as
  `BROWSER_CMD_*` variables; `browser-cmd restart` applies changed settings.
- `BROWSER_CMD_SITES`: the hosts the agent may act on; elsewhere a page is read, not clicked, typed into
  or scripted.
- The browser-cmd agent skill, installed for Claude Code, Codex and Pi by the installer; also a Claude
  Code plugin and a Pi package.
- Prebuilt binaries for Linux and macOS (x86_64, arm64) and Windows (x86_64), and installers for each.

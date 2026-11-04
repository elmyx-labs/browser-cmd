# Changelog

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

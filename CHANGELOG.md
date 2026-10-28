# Changelog

## 0.1.0

First public release.

- `browser-cmd`: a single binary that drives an installed Google Chrome over the DevTools Protocol through
  a warm daemon, with the page as an accessibility tree with refs that stay valid across commands.
- Commands to navigate, click, hover, drag, type, choose and select, press keys, scroll, read text,
  extract rows, read the page's own network data, take screenshots, check evidence, manage tabs, upload
  files, evaluate JavaScript and send raw DevTools calls; `batch` runs several in one call.
- Attach to a running Chrome (`BROWSER_CMD_CDP`), upstream HTTP and SOCKS5 proxies with credentials, and
  per-workspace sessions.
- The browser-cmd agent skill, as a Claude Code plugin and for Codex.
- Prebuilt binaries for Linux and macOS (x86_64, arm64) and Windows (x86_64), and an installer.

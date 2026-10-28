# Configuration

browser-cmd needs no configuration: it finds Google Chrome, starts a daemon on first use and keeps a
profile under `~/.browser-cmd`. Everything below is optional.

## Environment variables

All variables are read strictly: blank counts as unset, and a value that does not parse is an error,
never a silent default. Yes/no values accept `1`, `true`, `yes`, `on` and `0`, `false`, `no`, `off`.

The daemon reads the browser settings once, when it starts, from the environment of the command that
started it. To change them, stop the daemon (Ctrl+C on `--serve`, or end its process; it closes Chrome
gracefully on SIGINT/SIGTERM) and run a command again.

### Browser (read by the daemon)

| Variable | Default | Effect |
| --- | --- | --- |
| `BROWSER_CMD_PROFILE` | `~/.browser-cmd/browser/default` | Chrome's user data directory: cookies, sign-ins and history a site remembers. Created with owner-only permissions on Unix. |
| `BROWSER_CMD_CDP` | unset | Attach to a running Chrome instead of launching one: its `http://HOST:PORT` DevTools endpoint (preferred: it survives a Chrome restart) or a `ws://` debugger URL. Nothing is launched, overridden or closed, and the launch settings below have no effect. Setup: [your-chrome.md](your-chrome.md). |
| `BROWSER_CMD_EXECUTABLE` | discovered | The Chrome binary to launch. Without it, browser-cmd looks for stable Google Chrome in the standard install locations, then on `PATH`, then (macOS) via Spotlight; Chromium, Beta, Dev, Canary and Chrome for Testing are not picked. |
| `BROWSER_CMD_HEADLESS` | headless only without a display | Yes/no. macOS and Windows always have a display; Linux has one when `DISPLAY` or `WAYLAND_DISPLAY` is set. |
| `BROWSER_CMD_NO_SANDBOX` | no | Yes/no: pass `--no-sandbox`, for containers that cannot run Chrome's sandbox. |
| `BROWSER_CMD_ARGS` | none | Extra Chrome flags, split on whitespace, added after browser-cmd's own. |
| `BROWSER_CMD_WINDOW` | `1440,900` | `WIDTH,HEIGHT` of the headless window in CSS pixels. |
| `BROWSER_CMD_LANGUAGE` | `en-US,en` | Comma-separated language tags for `Accept-Language` and `navigator.languages`. |
| `BROWSER_CMD_TIMEZONE` | the machine's | IANA zone (such as `Europe/Berlin`) Chrome runs in, passed to Chrome as `TZ`. |
| `BROWSER_CMD_PROXY` | none | Upstream proxy: `http://[user:pass@]host[:port]` or `socks5://[user:pass@]host[:port]` (`socks://` is the same; `socks5h://` is refused because names are resolved locally). Chrome reaches it through a forwarder on a loopback port, which adds the credentials; QUIC and non-proxied WebRTC UDP are disabled, and autoplay needs a user gesture. If the upstream fails, requests fail rather than going direct. |
| `BROWSER_CMD_PROXY_HOSTS` | all hosts | Comma-separated hosts that go through the proxy, with their subdomains; everything else goes direct. Requires `BROWSER_CMD_PROXY`. |
| `BROWSER_CMD_DIALOGS` | `alerts` | How JavaScript dialogs are answered: `alerts` accepts alerts and dismisses confirms, prompts and leave-page questions; `accept` accepts all; `dismiss` dismisses all. |
| `BROWSER_CMD_PACE` | `1` | Multiplier on the human pauses between pointer and keyboard steps, from `0` (no pauses, no wandering pointer) to `100`. |
| `BROWSER_CMD_NETWORK` | no | Yes/no: every tab keeps its data responses from its first request, for `network`. Otherwise a tab records from its first `network` command on. Enables the DevTools Network domain. |
| `BROWSER_CMD_NETWORK_NOTES` | no | Yes/no: report requests the page made and did not get. Enables the DevTools Network domain. |
| `BROWSER_CMD_CANVAS_TEXT` | no | Yes/no: record the text pages draw on 2D canvases, so the page state can say it. Wraps the page's canvas functions, which a page can detect. |
| `BROWSER_CMD_AWARENESS` | yes | Yes/no: tell the agent when it is repeating itself or making no progress. |

### Daemon and client

| Variable | Default | Effect |
| --- | --- | --- |
| `BROWSER_CMD_ADDR` | `127.0.0.1:7788` | `HOST:PORT` of the daemon, for both the client and `--serve` without an address. A client only starts a daemon itself on a loopback address. |
| `BROWSER_CMD_SESSION` | one per workspace | Session name. By default every command in the same workspace shares one session (its tabs and refs); a name gives two callers in one workspace separate sessions. |
| `BROWSER_CMD_WORKSPACE` | git work tree, else the working directory | The workspace root: what the default session is named after, and, for the daemon, where its files go (see below). |
| `BROWSER_CMD_RPC_TOKEN` | none | Bearer token the client sends and the daemon requires. Required for a daemon listening off loopback. Visible ASCII only. |
| `BROWSER_CMD_RPC_SECS` | `600` | Client-side deadline for one command, in seconds. The daemon also answers each request within 100 seconds (or this value, if lower) and reports the outcome as unknown when a command runs longer. |
| `BROWSER_CMD_IDLE_SECS` | `7200` | How long a session may sit unused before the daemon lets it go. |
| `BROWSER_CMD_NO_SPAWN` | no | Yes/no: never start a daemon or a browser. A command fails if no daemon answers, and `--serve` refuses to run. For sandboxes where the browser belongs to someone else. |
| `BROWSER_CMD_BRIEF` | no | Yes/no: print one line for each action that worked, as `--brief` does. |
| `BROWSER_CMD_NO_TRACE` | no | Yes/no: do not write failure traces. |

### Other variables

- `TZ`: `browser-cmd status` prints the local date and time zone from `BROWSER_CMD_TIMEZONE`, else `TZ`,
  else the zone `/etc/localtime` links to.
- `HOME`: the daemon's home directory is `$HOME/.browser-cmd` (the current directory's `.browser-cmd` if
  `HOME` is unset).

## Files

| Path | What is there |
| --- | --- |
| `~/.browser-cmd/browser/default/` | The default Chrome profile, plus `browser-cmd-launch`, the command line it was started with (a Chrome left running on the profile is reused only if it matches). |
| `<workspace>/.browser-cmd/` | Session tab records (`tabs-*.json`), long text and network bodies saved whole (`text-NNNN.txt`, …). |
| `<workspace>/media/browser/` | Screenshots and saved documents (such as PDFs). |
| `<workspace>/media/browser/downloads/` | Files the page downloads. |
| `./.browser-cmd/used` | An empty marker that browser-cmd ran a command in this directory. |
| `./.browser-cmd/traces/` | A trace per failed command (the command, the error and the browser log), in the directory the command ran in. |

`<workspace>` is the workspace of the directory the daemon was started from. A daemon started by a
command in one project and then used from another keeps writing to the first one's workspace; give each
project its own `BROWSER_CMD_ADDR` when that matters. `upload` only accepts files inside the workspace.

Add `.browser-cmd/` (and `media/browser/` if you do not want screenshots tracked) to your `.gitignore`.

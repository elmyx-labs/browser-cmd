# Architecture

One Rust binary: a CLI that sends each command to a warm daemon, which holds one Chrome over CDP and one
session per caller. Layers, low to high; a module uses only its own layer and those below
(`crates/browser-cmd/tests/layers.rs` fails the build otherwise). CDP is spoken only by the modules that own
each part of it, never by `session` or `tools` but for the `cdp` command, which sends what its caller names (`tests/cdp_boundary.rs` lists them, and fails on a method named outside its owner or an allowance no file uses):

| layer | modules | owns |
|---|---|---|
| foundation | `error`, `text`, `env`, `http`, `web` | error kinds, bounded text and fences, strict `BROWSER_CMD_*` parsing, HTTP/1.1 framing, URL policy |
| configuration | `settings`, `workspace` | every setting and its default; the workspace root (BROWSER_CMD_WORKSPACE, else the git work tree, else the working directory), names stable across builds, numbered files in the workspace |
| `chrome` | `cdp`, `events`, `traffic`, `tab`, `launch`, `discover`, `proxy` | the CDP connection, per-tab browser events and the browser's pages from Target events, the Chrome process, the upstream proxy forwarder |
| `input` | `hand`, `keys`, `rect` | trusted pointer and keyboard at a person's tempo |
| `page` | `world`, `snapshot/`, `reading`, `frames`, `settle`, `navigation`, `walls`, `view`, `dom`, `canvas`, `overlay`, `js/` | reading a live page in isolated worlds: the ref tree, text, readiness, bot walls; moving it (`navigation`) |
| `browser` | `host`, `identity`, `prepare` | the browser's lifecycle (a Chrome on the profile, whichever daemon started it, closed at a graceful exit; one attached to left as it is), its identity, what every new tab is told; the host keeps an opaque slot for a layer above (the tabs ledger) that outlives its connections |
| `tabs` | `book`, `ledger`, `registry`, `store`, `status`, `context`, `cookies`, `close`, `recover` | a session's tabs: every new one opened by `create` (held from its first moment; in attach mode in a background window of its own), the active one and the one to go to when it goes (`Book::next`), handles (a number is a handle, an id prefix is four hex digits or more), kept and borrowed tabs and their file, each one's typed `TabStatus`, its own browser contexts, followed across connections, and the clean retry into a new one with the cookies it carries (a clean retry replaces one tab; an earlier context stays while a tab lives there and is let go before the next command once none does), following, adopting late windows, reloading crashed and restoring kept tabs only before a command that acts on a tab (`Command::acts`; `tab list`/`check` reopen nothing), taking them back after a daemon restart (`tab restore` for kept ones gone), ending a session (`close`: own tabs closed, borrowed given back, kept left open); `ledger` is every claim on a page, one per host, pruned with the pages: own, borrowed, another session's (refused by name), nobody's (`tab use`), transient; a session dropped — closed, or idle past `BROWSER_CMD_IDLE_SECS` — lets go of its claims; `store` is the versioned file per browser and session, written whole |
| `session` | `mod`, `document`, `state`, `after`, `land`, `wall`, `notes`, `output`, `awareness`, `visual`, `pdf` | one caller's document and refs; what every action answers with |
| `tools` | one file per command family, plus helpers `aim`, `submission`, `args`, `collection` | the commands |
| `api` | `command`, `help`, `cli`, `client`, `server`, `http`, `wire`, `app` | the catalogue, the CLI and the daemon's HTTP endpoint |

Tests: unit tests sit beside their code; tests that need a real Chrome on local fixtures are in `src/tests/`
(`#[ignore]`, run with `--include-ignored`); `tests/` drives the built binary.

Outside the crate: `plugin/` is the Claude Code plugin (the agent skill), `.claude-plugin/` its marketplace,
`docs/` the user documentation, `install.sh` the installer and `.github/workflows/` CI and releases.

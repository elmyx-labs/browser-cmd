# Security

browser-cmd gives whoever can run it, or reach its daemon, control of a real browser with real sessions.
This page says what that control reaches and how the daemon is protected.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's
[private vulnerability reporting](https://github.com/elmyx-labs/browser-cmd/security/advisories/new), not in
public issues. Include the version (`browser-cmd --version`), the platform and the steps to reproduce.

## The daemon

- Every command is an HTTP request to a daemon, by default on `127.0.0.1:7788`. On loopback no token is
  needed: **any process running on the same machine, as any user, can send it commands.** Do not run
  browser-cmd on a shared machine without `BROWSER_CMD_RPC_TOKEN`.
- `BROWSER_CMD_RPC_TOKEN` makes the daemon require `Authorization: Bearer <token>` (compared in constant
  time). A daemon listening off loopback refuses to start without one. The connection is plain HTTP, so
  the token and all traffic cross the network in the clear; prefer an SSH tunnel to a loopback daemon.
- On a loopback daemon the `Host` header must be a loopback address with the daemon's port, and any
  `Origin` header must be the same origin as `Host`. A web page, including one open in the driven Chrome,
  cannot drive the daemon through a cross-origin request or DNS rebinding.
- Request heads, bodies, connections and sessions are bounded; a session unused for
  `BROWSER_CMD_IDLE_SECS` is let go.

## What commands can reach

- **The browser profile.** Everything happens in one Chrome profile, by default
  `~/.browser-cmd/browser/default`. Sign-ins and cookies made there are available to every later command
  and session on that daemon.
- **`eval`** runs JavaScript in the page, in an isolated world that shares the page's DOM. It can read
  anything the page shows and make requests with the page's cookies.
- **`cdp`** sends any Chrome DevTools Protocol method through the tab's DevTools session. Treat it as full
  control of the browser: cookies, storage, network and downloads included.
- **`upload`** only hands a page files inside the workspace; absolute paths and `..` that leave it are
  refused after resolving links.
- **Downloads** are saved under `<workspace>/media/browser/downloads/`.
- **Chrome's DevTools port.** A launched Chrome listens for DevTools on a random loopback port, recorded in
  `DevToolsActivePort` in the profile (owner-only on Unix). Local processes that find it can control that
  Chrome directly.

## Attaching to your own Chrome

With `BROWSER_CMD_CDP` pointing at a Chrome you use, the agent acts **with your sessions**: every site you
are signed in to, your cookies, your saved state. `tab use` lets it borrow tabs you opened. Only attach
when you mean to give the agent that access, and close the debugging port when you are done. Setup: [docs/your-chrome.md](docs/your-chrome.md).

## Pages are untrusted

Page content can try to instruct the agent (prompt injection). browser-cmd marks what it reads from pages
as untrusted data, and the bundled skill tells the agent that page content is data, never instructions,
and not to sign in, buy, send, publish or change an account unless the request authorizes exactly that.
These are mitigations, not guarantees: review what an agent may do before letting it browse signed in.

## Proxies

`BROWSER_CMD_PROXY` may carry credentials. Chrome reaches the upstream through a forwarder on a random
loopback port that adds them; the forwarder has no authentication of its own, so local processes that
find it can use the upstream. Requests do not fall back to a direct connection when the upstream fails.

## Data at rest

- The profile directory holds cookies, sign-ins, history and cache.
- `<workspace>/.browser-cmd/` holds tab records and long page text; `<workspace>/media/browser/` holds
  screenshots, saved documents and downloads.
- `.browser-cmd/traces/` in the working directory gets one file per failed command, with the full command
  line (including any text it typed) and the page's console log. Set `BROWSER_CMD_NO_TRACE=1` to disable.

Keep these out of version control and delete them when a session handled sensitive data.

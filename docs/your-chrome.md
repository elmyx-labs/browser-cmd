# Using your own Chrome

By default browser-cmd launches a Chrome of its own with a fresh profile. You can instead attach it to a
Chrome you started and signed in to: the agent then works with your sign-ins and saved state, as you
would. This is the way to give an agent sites that need an account, or that trust a browser with history
more than a new one.

The agent acts **with your sessions** in that Chrome. Use a profile made for the purpose, sign in only
where the agent should act, and read [SECURITY.md](../SECURITY.md#attaching-to-your-own-chrome).

## 1. Start Chrome with a debugging port

Chrome opens a DevTools port only for a profile directory other than its default one, so give it a
directory of its own. The profile is kept there between runs, sign-ins included.

macOS:

```sh
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --remote-debugging-port=9222 --user-data-dir="$HOME/browser-cmd-chrome"
```

Linux:

```sh
google-chrome --remote-debugging-port=9222 --user-data-dir="$HOME/browser-cmd-chrome"
```

Windows (PowerShell):

```powershell
& "C:\Program Files\Google\Chrome\Application\chrome.exe" `
  --remote-debugging-port=9222 --user-data-dir="$env:USERPROFILE\browser-cmd-chrome"
```

A regular Chrome window opens. Use the same command each time; quitting and starting it again keeps the
profile.

## 2. Sign in

In that window, sign in to the sites the agent should use and accept their cookie banners. Leave the
window open.

Check that the port answers:

```sh
curl -s http://127.0.0.1:9222/json/version
```

It prints a JSON object with a `webSocketDebuggerUrl`.

## 3. Point browser-cmd at it

```sh
export BROWSER_CMD_CDP=http://127.0.0.1:9222
browser-cmd navigate https://example.com
```

Set the variable in the shell that starts your agent (Claude Code or Codex), so the commands it runs
inherit it. The daemon reads it once, when it starts: if a daemon is already running with its own Chrome,
stop it first (end its process), and the next command starts one attached to yours. The `http://`
endpoint is preferred to a `ws://` URL because it still works after Chrome restarts.

## What happens in your Chrome

- browser-cmd launches, closes and reconfigures nothing: the launch settings (`BROWSER_CMD_PROFILE`,
  `BROWSER_CMD_HEADLESS`, `BROWSER_CMD_PROXY` and the rest) have no effect, and traffic goes out on
  your own network.
- The agent opens tabs of its own. Your tabs are listed as no session's, and it touches one only when
  it borrows it with `tab use`.
- `retry-clean` opens the page again in a fresh isolated context, without your sign-ins unless it is
  given `keep_signin=true`, which carries over that site's cookies.
- `browser-cmd close` ends the session: its tabs close, borrowed ones go back to you and kept ones stay
  open. Your browser keeps running.

## Stop

Quit that Chrome, or close the window, and unset `BROWSER_CMD_CDP`. Chrome listens on `127.0.0.1` only;
while the port is open, any program on this computer can control that browser, so do not leave it
running unattended and never expose the port beyond localhost.

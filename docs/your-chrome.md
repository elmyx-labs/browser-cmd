# Using your own Chrome

By default browser-cmd launches a Chrome of its own with a fresh profile. It can instead work in the
Chrome you are signed in to: the agent then has your sign-ins and saved state, as you would, and can
borrow the tab you have open. This is the way to give an agent sites that need an account, or that
trust a browser with history more than a new one.

The agent acts **with your sessions** in that Chrome. Name the sites it may act on (`sites` below),
close tabs it should not see, and read [SECURITY.md](../SECURITY.md#attaching-to-your-own-chrome).

## Your everyday Chrome (Chrome 144 or later)

1. In that Chrome, open `chrome://inspect/#remote-debugging` and turn on **Remote Debugging**. Chrome
   now notes a DevTools port in your profile directory, and browser-cmd finds it there.
2. Tell browser-cmd to use that Chrome, once, in `~/.browser-cmd/config.toml` (every project) or in a
   project's `.browser-cmd/config.toml`; an agent asked to work "in my Chrome" writes the project one
   itself:

   ```toml
   chrome = "mine"
   sites = "mail.google.com, github.com"   # where the agent may act; elsewhere it only reads
   ```

   For one command, `BROWSER_CMD_CHROME=mine browser-cmd …` does the same.
3. On the agent's first command Chrome comes forward and asks **Allow remote debugging?** Click
   **Allow**; a banner then says the browser is being controlled by automated test software. browser-cmd
   connects once per daemon, so this happens once per session of work. After it nothing brings Chrome
   forward: the agent's tabs open in windows of their own behind yours, and switching, checking and
   screenshots leave your focus where it is. To watch the agent work instead, set `follow = true` in the
   config file: the tab it turns to then comes forward, with Chrome.

The daemon for your Chrome runs beside the one for browser-cmd's own browser (on port 7789 instead of
7788, unless `BROWSER_CMD_ADDR` says otherwise), so a project set to `chrome = "mine"` and one that is
not work at the same time, and switching needs no restart. Turn Remote Debugging off again when the
work is done.

## A Chrome started for the agent

For a Chrome you leave running for the agent, with sign-ins and a profile of its own, start one with a
debugging port and a profile directory of its own (Chrome opens the port only for a directory other
than its default one; the profile is kept there between runs, sign-ins included):

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

Sign in to the sites the agent should use in that window, leave it open, and write
`cdp = "http://127.0.0.1:9222"` into a config file (or export `BROWSER_CMD_CDP`). Use the same command
each time; quitting and starting Chrome again keeps the profile. `curl -s http://127.0.0.1:9222/json/version`
checks that the port answers.

## What happens in your Chrome

- The daemon reads its settings once, when it starts: after changing `cdp`, run `browser-cmd restart`.
  The `http://` endpoint is preferred to a `ws://` URL because it still works after Chrome restarts.
- browser-cmd launches, closes and reconfigures nothing: the launch settings (`BROWSER_CMD_PROFILE`,
  `BROWSER_CMD_HEADLESS`, `BROWSER_CMD_PROXY` and the rest) have no effect, and traffic goes out on
  your own network.
- The agent opens tabs of its own, each in a window of its own behind yours, so that its pages stay
  shown and running without being brought in front of you. Your tabs are listed as no session's, and it
  touches one only when it borrows it: `tab use active` takes the tab in front of you, `tab use N` one
  by its number in `tab list`.
- Outside the `sites` named, the agent can read a page but not click, type, upload or run scripts on
  it. With no `sites` named it may act everywhere.
- `retry-clean` opens the page again in a fresh isolated context, without your sign-ins unless it is
  given `keep_signin=true`, which carries over that site's cookies.
- `browser-cmd close` ends the session: its tabs close, borrowed ones go back to you and kept ones stay
  open. Your browser keeps running.

While a debugging port is open, any program on this computer can control that browser, so do not
leave it on unattended and never expose the port beyond localhost.

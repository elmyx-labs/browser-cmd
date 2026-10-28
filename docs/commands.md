# Commands

Every command below is also documented by the binary itself: `browser-cmd --help` lists them all, and
`browser-cmd <command> --help` prints one command's syntax and description. The sections under
[Browser commands](#browser-commands) are the text of `browser-cmd <command> --help`.

## Invocation

```
browser-cmd <command> [args]     run one browser action against the warm daemon
browser-cmd status               whether the daemon is up, and the local date and time zone
browser-cmd batch <<'EOF'        several commands, one per line, in one call
browser-cmd back | forward       the same as navigate back | navigate forward
browser-cmd --serve [ADDR]       run the daemon in the foreground (default 127.0.0.1:7788)
browser-cmd --help               the command catalogue
browser-cmd <command> --help     one command's syntax
browser-cmd --version            the version
```

The first command starts the daemon in the background when nothing answers at `BROWSER_CMD_ADDR`. The
daemon holds one Chrome and one session per caller: by default one per workspace (the git work tree
around the working directory), or the one named by `BROWSER_CMD_SESSION`.

### Arguments

- Words where the shape is obvious (`click @e1a.3`, `scroll down`, `tab switch t2`), and `key=value`
  otherwise. A value that parses as JSON is passed as JSON (`all=true`, `limit=50`,
  `fields='{"title":"h2"}'`), anything else as a string.
- A lone `'{json}'` argument is passed through whole as the command's arguments, so every option is
  reachable even where the shorthand does not reach it.
- Refs such as `@e4803152.3` come from the latest page state and are opaque: copy them whole. A ref stays
  valid for as long as its document lives, across command processes.

### Output

- Actions (`navigate`, `click`, `type`, `choose`, `select`, `press`, `scroll`, `wait`, `tab`, …) print
  the page state afterwards: the full tree on a new document, and only what changed after an action on
  the document already shown. Reads (`text`, `eval`, `extract`, `markup`, `images`, `evidence`,
  `console`, `network`) print only what they read.
- `--brief`, before or after the arguments, or `BROWSER_CMD_BRIEF=1`, prints one line for an action
  that worked: `command: result | URL: … | Title: …`.
- `--raw` on `eval` and `text` drops the quoting around page content; `eval` prints its value as one line
  of JSON.
- A failed command exits with status 1, prints its error and the page state on stdout, repeats the
  reason on stderr as `browser-cmd: …`, and writes a trace to `.browser-cmd/traces/` in the working
  directory (unless `BROWSER_CMD_NO_TRACE=1`). A command whose outcome is unknown says
  `status=unknown`: check the page before repeating it.

### batch

`batch` reads commands from stdin, one per line, split like a shell line (quotes and backslashes);
blank lines and lines starting with `#` are skipped, and a leading `browser-cmd ` is optional. Each
command but the last prints one line (as with `--brief`); the last prints its whole output; the first
failure stops the batch with its full output. A batch cannot contain another batch.

```sh
browser-cmd batch <<'EOF'
navigate https://example.com
click "More information"
text max_bytes=2000
EOF
```

## Browser commands

### navigate

```
browser-cmd navigate URL [here=true] | back | forward
```

Open an http(s) URL in a real Chrome, or go back or forward. Every command prints the page state afterwards — URL, title, what happened, and the controls with their refs — so the next command needs no snapshot first. A Cloudflare or CAPTCHA box is ticked for you and the state says how that went. From a kept or borrowed tab the URL opens in a new tab, unless here=true.

### snapshot

```
browser-cmd snapshot [@ref] [full] [coords]
```

The page as its accessibility tree: controls and headings with @e1a.31 refs by default, everything readable with full. The whole tree, where an action on the same document prints only what it changed, and one line when it is the tree last shown whole; @ref prints one element and what is inside it, a long tree is saved to a file whose head is shown. coords appends the viewport x,y of each ref. A link to another site shows its host and path, … for a query (text links=true has it whole).

### click

```
browser-cmd click @ref | "name" | X,Y [double=true] [right=true] [again=true]
```

Click an element by ref or by the words on it, or a viewport point by x and y, with a real pointer that travels there first. A name the accessibility tree does not carry falls back to a clickable custom control bearing that visible text. A new tab that opens becomes the active one.

### hover

```
browser-cmd hover @ref | "name" | X,Y
```

Move the pointer onto an element or point without clicking, for menus that open on hover.

### drag

```
browser-cmd drag from='{…}' to='{…}'
```

Press on one element or point, travel to another with the button held, and let go: a slider set, a card moved between columns, a map panned.

### type

```
browser-cmd type [@ref] TEXT… [submit]
```

Replace the text of a field by typing it key by key, then press Enter if asked to submit. A date, time, colour or range field is given the text as its value. An empty string clears the field. A lone TEXT with no ref goes in at the caret of whatever has focus — an editor with no ref, a canvas document, a sheet cell, a frame's rich text — after a click has put it there; a line break in it (from stdin: type - <<'EOF') is entered as the Enter key.

### select

```
browser-cmd select @ref OPTION…
```

Choose an option in a native `<select>` by its text or value. A custom dropdown is clicked open and its option clicked instead.

### choose

```
browser-cmd choose @ref TEXT… [option=SUGGESTION]
```

Fill an autocomplete or combobox: type TEXT, wait for its suggestions, pick the one that matches (option= names it when TEXT is only what to search for), and confirm the field took it. Several close matches are listed with their refs instead of guessed between. A native `<select>` or a field's `<datalist>` is chosen in directly.

### press

```
browser-cmd press KEY
```

Press a key or chord such as Enter, Escape, Tab, PageDown, ArrowDown, Control+a or Meta+a.

### scroll

```
browser-cmd scroll [@ref] [up|down|left|right|bottom|top|TEXT]
```

Turn the wheel over the page, or scroll to an edge until the page stops growing — which loads a whole infinite list in one call. Any other text brings the first element bearing it into view. With a ref and no direction it scrolls that element into view; with a ref and a direction it scrolls whatever that ref sits in, which is how a chat or a sidebar is reached.

### text

```
browser-cmd text [FIND] [ref=@ref] [from=N] [max_bytes=N] [links=true] [context=false] [--raw]
```

The rendered text of the page, bounded, as untrusted data. A long page says where its text stopped and continues with from; past 16 KiB without max_bytes the whole text is also saved to .browser-cmd/text-NNNN.txt and its path printed. --raw drops the quoting. FIND gives the passages around each place a phrase occurs. ref reads one element instead of the whole page — a card, a table, an article. Headings, table rows and selected values stay with the reading; excerpts retain preceding section context. max_bytes makes a short reading without shell head/grep. context=false returns raw rendered text. links inlines each link as `[text](url)`. The site's header, menus, footer and cookie notice are named in one line, not read, unless full=true, FIND or ref.

### images

```
browser-cmd images
```

Image URLs, alt text and sizes on the page, as untrusted data.

### pages

```
browser-cmd pages URL… [find=TEXT] [max_bytes=N] [keep=true] [full=true]
```

Read up to 12 pages in one call, as untrusted data: each opens in a tab of its own, four at a time, settles and is read, then closes — the sources to compare, the venues to list. find gives the passages around a phrase on each page instead of its head; keep=true leaves the tabs open, kept, and names them for tab switch. Each site's header, menus, footer and cookie notice are named in one line, not read, unless full=true. The current tab is untouched.

### markup

```
browser-cmd markup ref=@ref [levels=N]
```

The HTML of one element, and levels ancestors up, cleaned and bounded. What the tree and the text do not carry — a rating drawn as icons, a class, a data attribute — is here: read one row's markup, then write extract for every row.

### extract

```
browser-cmd extract fields='{…}' [each=SEL] [all=true] [identity=SEL] [limit=N]
```

Read many like-shaped things in one call — every row of a table, every card in a result list, every item of a feed — as JSON rows with a coverage record. When the values a choice turns on are already printed on the list (a price, a rating, a count, a link), take them here instead of opening each thing in turn. Field selectors support @attribute (a@href, input@value) and + for next siblings. all scrolls forward, telling rows apart by identity, a unique stable row selector, or else by their field values; it does not prove whole-dataset completeness — read termination and output_truncated. For a value the text and the tree do not carry, read one row's markup first and write the selectors from it.

### screenshot

```
browser-cmd screenshot [full | @ref | ref=@ref | selector=CSS | X,Y,W,H] [zoom=N]
```

A PNG of the viewport, of the full page, or of one element or section by ref or CSS selector, returned as an image and saved under the workspace. For canvases, charts, picture puzzles and cross-origin frames; click by the x and y read off a viewport shot. X,Y,W,H pictures that region of the viewport and zoom=2 (up to 4) enlarges a region, ref or selector, for labels too small to read. Warns when the page is raw JSON or another unrendered document.

### evidence

```
browser-cmd evidence "VALUE"…
```

Which of the given strings are visible in the viewport now, and where: x,y and the element, or how many matches the page has out of view and the nearest page y. Reads the rendered text and field values of the top document; reports facts, never whether an answer is right. Warns when the page is raw JSON or another unrendered document.

### wait

```
browser-cmd wait [SECONDS | TEXT] [gone=TEXT] [url=TEXT]
```

Wait for a page that draws itself: until text appears, until text disappears, until the URL contains a string, or for a number of seconds and the DOM to go quiet.

### verify

```
browser-cmd verify [text=…] [gone=…] [url=…] [ref=@ref]
```

Check that the page is now as expected and fail if it is not. text and gone each take one phrase or a list, and every phrase is its own condition, so a step with several constraints is one checkpoint that names which of them did not hold.

### tab

```
browser-cmd tab list|check|tidy | new [URL] | switch|close|keep|unkeep|give|restore [tN] | use N|ID
```

This session's tabs, each by a handle t1, t2, … it keeps for the session: list them, switch to or close one, or open a new one at an optional url; a window one of them opens later is this session's too. Other sessions' tabs are one count with their names. Tabs no session holds, such as a person's, are listed apart and numbered: use borrows one, which is never closed — close gives it back — and from which navigate URL opens a new tab. give tN to=SESSION hands a tab to another session. keep marks a tab — the current one unless named — as evidence, with an optional label=TEXT, and unkeep undoes it; navigate URL on a kept tab opens a new tab for it, and a kept tab lost with the browser is reopened at its address; one gone since an earlier run of the daemon waits for restore; a tab that crashed is reloaded. check looks at every tab now — open, loaded, an error page or refusal, a wall, empty, still at its kept address, closed or crashed, reopening nothing — and saves a screenshot of each kept one; tidy closes the tabs not kept. action=, id=, url=, label= and to= say it as pairs.

### upload

```
browser-cmd upload ref=@ref path=PATH
```

Attach a file from the workspace to a file input, by ref.

### eval

```
browser-cmd eval EXPRESSION | - [--raw]
```

Evaluate a JavaScript expression in the page and return its JSON value; - reads the expression from stdin, and --raw prints the value as one line of JSON. It runs in a caller-specific isolated world, separate from page globals and from internal helpers; DOM changes are shared and globals persist on the same document. Use the pointer and keyboard commands for trusted input — a scripted write may not notify a framework.

### cdp

```
browser-cmd cdp METHOD ['{json params}']
```

The escape hatch: send a raw Chrome DevTools Protocol method to the tab and get its result. The tab's own session is used, so page domains (DOM, Page, Input, Network, Emulation) are in scope.

### console

```
browser-cmd console [errors]
```

What the current tab logged — errors, blocked requests, CSP violations — as untrusted data, for when a step did not do what was expected. A rolling window; errors keeps only the error lines.

### network

```
browser-cmd network [FIND] [body=true] [max_bytes=N]
```

The data the page fetched for itself — XHR, fetch, event-stream and socket messages — as untrusted data: the JSON behind a map, a chart, a search or a feed, which says more than the pixels drawn from it. Lists the responses whose address has FIND; body=true reads the newest three, saving a long one whole under .browser-cmd/. The first call on a tab starts recording unless BROWSER_CMD_NETWORK=1 already did.

### retry-clean

```
browser-cmd retry-clean [keep_signin=true]
```

Open the current URL again in a fresh isolated browser context with no cookies or cache, when the profile's session, cache or reputation has poisoned the page. keep_signin transfers and verifies this site's cookies; check authentication on the page that comes back.

### close

```
browser-cmd close
```

End this session: close its tabs, give back borrowed ones, and leave kept ones open. The browser and its profile stay warm for the next session.

## Tab actions

| Action | What it does |
| --- | --- |
| `tab list` | This session's tabs by handle (`t1`, `t2`, …), other sessions' tabs as a count, and tabs no session holds, numbered. |
| `tab new [URL]` | Open a new tab, at URL if given. |
| `tab switch tN` | Make tN the active tab. |
| `tab close tN` | Close tN; a borrowed tab is given back and left open. |
| `tab keep [tN] [label=TEXT]` | Mark a tab (the current one unless named) as evidence: `navigate URL` on it opens a new tab, `tidy` and `close` leave it open. |
| `tab unkeep [tN]` | Undo `keep`. |
| `tab check` | Look at every tab now and save a screenshot of each kept one; reopens nothing. |
| `tab tidy` | Close the tabs that are not kept. |
| `tab use N\|ID` | Borrow a tab no session holds, such as one a person opened; it is never closed. |
| `tab give tN to=SESSION` | Hand a tab to another session. |
| `tab restore [tN]` | Reopen a kept tab listed `gone (earlier run)` after the daemon restarted. |

`action=`, `id=`, `url=`, `label=` and `to=` say the same as pairs.

<p align="center">
  <img alt="browser-cmd" src="docs/assets/logo.svg" width="420">
</p>

<p align="center">
  <strong>A real Chrome for AI agents, driven from the shell.</strong>
</p>

<p align="center">
  <a href="https://github.com/elmyx-labs/browser-cmd/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/elmyx-labs/browser-cmd/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/elmyx-labs/browser-cmd/releases"><img alt="Release" src="https://img.shields.io/github/v/release/elmyx-labs/browser-cmd?color=0B0D12&label=release"></a>
  <a href="https://github.com/elmyx-labs/browser-cmd-webvoyager"><img alt="WebVoyager 611/615" src="https://img.shields.io/badge/WebVoyager-99.35%25%20(611%2F615)-0B0D12"></a>
  <a href="https://github.com/elmyx-labs/browser-cmd-bu-bench"><img alt="BU Bench V1 96/100" src="https://img.shields.io/badge/BU%20Bench%20V1-96%2F100-0B0D12"></a>
  <a href="LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-0B0D12"></a>
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#use-it-with-claude-code">Claude Code</a> ·
  <a href="#use-it-with-codex-and-pi">Codex · Pi</a> ·
  <a href="docs/commands.md">Commands</a> ·
  <a href="docs/configuration.md">Configuration</a> ·
  <a href="#results">Results</a>
</p>

---

browser-cmd is a single binary that drives an installed Google Chrome over the DevTools Protocol. Each
command is one browser action and prints the page back as a compact accessibility tree with refs, so an
agent acts by reference instead of guessing selectors or pixels:

```console
$ browser-cmd navigate https://www.apple.com
Navigated.
<browser-state trusted="no">
URL: https://www.apple.com/
Title: Apple
Scroll: 0 of 5071 (top)

- heading "Apple" [level=1]
- link "Store" @ehvj.2
- link "Mac" @ehvj.4
- link "iPhone" @ehvj.8
- button "Search apple.com" @ehvj.24
- heading "MacBook Pro" [level=2]
- link "Learn more, MacBook Pro 14 and 16 inch" @ehvj.26
…
$ browser-cmd click @ehvj.26
```

Coding agents such as Claude Code and Codex already run shell commands, chain them and filter their output.
browser-cmd gives them a real browser through that same interface, together with a compact skill that
teaches the workflow.

## Why browser-cmd

- **One binary, any agent.** A CLI is the one interface every coding agent already has. No SDK, no MCP server, no browser extension.
- **Refs, not selectors.** Every state lists elements with stable refs. A ref taken by one command still names the same element in the next.
- **A warm daemon.** The first command starts a daemon that holds the browser between commands. Each workspace gets its own session and tabs.
- **A real browser.** A stable Google Chrome that presents itself as an ordinary user's browser, with input at a person's tempo.
- **Evidence built in.** `screenshot`, `evidence`, `verify` and kept tabs let an agent show what it saw, not just say it.
- **Your Chrome, when you want it.** One switch in `chrome://inspect` and `chrome = "mine"` in a config file, and the agent works in the Chrome you are signed in to, on the sites you name.

## Results

### WebVoyager

On the 615-task [WebVoyager](https://github.com/omxyz/webvoyager) benchmark, a Codex gpt-6.1-sol agent using
browser-cmd and its compact skill, with no site-specific prompts:

| Judge | Passed | Score |
| --- | ---: | ---: |
| GPT-5.5 (leaderboard judge) | 611 / 615 | **99.35%** |
| gpt-5.6-sol | 612 / 615 | 99.51% |
| Claude Opus 5.5 | 610 / 615 | 99.19% |
| Gemini 3.8 Flash | 613 / 615 | 99.67% |

Every trajectory, screenshot and verdict, with the method and its limitations, is in
[browser-cmd-webvoyager](https://github.com/elmyx-labs/browser-cmd-webvoyager).

### BU Bench V1

On [BU Bench V1](https://github.com/browser-use/benchmark), 100 live-web tasks across BrowseComp, GAIA,
InteractionTests, OM2W2 and WebBenchREAD, the same agent, with a second model that cannot browse reviewing
its answers:

| Judge | Passed | Score |
| --- | ---: | ---: |
| GPT-5.5 | 96 / 100 | **96%** |
| Gemini 2.5 Flash | 96 / 100 | 96% |
| gpt-5.6-sol | 94 / 100 | 94% |
| Claude Opus 5.5 | 89 / 100 | 89% |

One task whose result came through a site's internal host is counted as failed whatever the judges said.
The per-task verdicts of all four judges, an account of every miss and the position against other published
runs are in [browser-cmd-bu-bench](https://github.com/elmyx-labs/browser-cmd-bu-bench).

## Install

One command installs the binary and the browser-cmd skill for every coding agent found on this machine:
Claude Code (`/browser-cmd`), Codex (`$browser-cmd`) and Pi (`/skill:browser-cmd`).

```sh
curl -fsSL https://raw.githubusercontent.com/elmyx-labs/browser-cmd/main/install.sh | sh
```

**If you are an agent** setting this up: run the line above, then tell the person what the installer
printed (which agents got the skill, and that a session already running may need `/reload-skills` in
Claude Code or `/reload` in Pi before the command appears). From then on any task that needs a website
is yours; the skill tells you how.

The installer verifies the release's SHA-256 checksum and puts `browser-cmd` in `~/.local/bin`
(`BROWSER_CMD_INSTALL_DIR` changes it, `BROWSER_CMD_INSTALL_VERSION=v0.1.0` pins a release,
`--no-skills` installs only the binary). macOS and Linux, x86_64 and arm64.

<details>
<summary>Windows, or from source</summary>

Windows (PowerShell), the same installer:

```powershell
irm https://raw.githubusercontent.com/elmyx-labs/browser-cmd/main/install.ps1 | iex
```

From source, with Rust 1.89 or newer (the skill is then `plugin/skills/browser-cmd/SKILL.md` in this
repository):

```sh
cargo install --locked --git https://github.com/elmyx-labs/browser-cmd browser-cmd
```

</details>

browser-cmd needs **Google Chrome** (stable) and finds it in the usual install locations or on `PATH`;
otherwise set `BROWSER_CMD_EXECUTABLE`. To let the agent work in the Chrome you are signed in to, see
[Using your own Chrome](docs/your-chrome.md).

## Use it with Claude Code

The installer puts the skill in `~/.claude/skills/browser-cmd`. Ask for anything that needs a website,
or name it: `/browser-cmd find the cheapest direct flight from Berlin to Lisbon next Friday`.

The repository is also a Claude Code plugin marketplace, for setups that manage plugins centrally:

```sh
claude plugin marketplace add elmyx-labs/browser-cmd
claude plugin install browser-cmd@browser-cmd
```

For one session without installing anything, run `claude --plugin-dir path/to/browser-cmd/plugin`.

## Use it with Codex and Pi

Both read `~/.agents/skills`, where the installer puts the skill: mention `$browser-cmd` in Codex, type
`/skill:browser-cmd <task>` in Pi, or just ask. Pi can also install the skill as a package, with
`pi install git:github.com/elmyx-labs/browser-cmd` (the binary still comes from the installer).

For one project, copy `skills/browser-cmd/SKILL.md` from the release archive (or
`plugin/skills/browser-cmd/SKILL.md` from this repository) to `.agents/skills/browser-cmd/SKILL.md`.

If the agent runs in a sandbox that cannot start Chrome, start the daemon outside it with
`browser-cmd --serve` and give the agent `BROWSER_CMD_NO_SPAWN=1`.

## A short session

```sh
browser-cmd navigate https://example.com      # opens the page, prints its state with refs
browser-cmd snapshot                          # the full accessibility tree
browser-cmd click @e4803152.3                 # refs are opaque: copy them whole from the latest state
browser-cmd type @e4803152.5 "hello" submit   # type into a field and press Enter
browser-cmd text                              # the rendered page text, bounded
browser-cmd extract fields='{"title":"h2","link":"a@href"}' each=article
browser-cmd verify text="Order placed"        # fails unless the page says it
browser-cmd screenshot                        # saved under media/browser/
browser-cmd close                             # end the session; the browser stays warm
```

A failed command exits non-zero, repeats its reason on stderr and leaves a trace under
`.browser-cmd/traces/`. `browser-cmd status` says whether the daemon is up; `browser-cmd --help` lists
every command.

## Documentation

- [Commands](docs/commands.md) — every command, argument and output mode
- [Configuration](docs/configuration.md) — `BROWSER_CMD_*` variables and where files go
- [Using your own Chrome](docs/your-chrome.md) — attach the agent to a Chrome you signed in to
- [Security](SECURITY.md) — what the daemon exposes and what an agent can reach
- [Architecture](ARCHITECTURE.md) — the layers of the binary
- [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md) — working on browser-cmd

## License

[Apache-2.0](LICENSE)

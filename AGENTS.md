# Working on browser-cmd

browser-cmd is one Rust binary (`crates/browser-cmd`) plus the agent skill that teaches an agent to use it
(`plugin/skills/browser-cmd/SKILL.md`, shipped as a Claude Code plugin and copied for Codex).

## Build and check

```sh
cargo build --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked          # no Chrome needed
```

Tests that drive a real Chrome on local fixtures are `#[ignore]`d. Run them only on a machine with Google
Chrome, headless, with a fresh profile:

```sh
BROWSER_CMD_HEADLESS=1 cargo test --workspace --locked -- --include-ignored --skip public_
```

`public_detection_*` tests visit public third-party sites; never run them in CI or by default.

## Layout

- `crates/browser-cmd/src/` by layer; [ARCHITECTURE.md](ARCHITECTURE.md) says what each owns.
  `tests/layers.rs` fails the build when a module uses a layer above its own, and `tests/cdp_boundary.rs`
  when a CDP method is sent outside the module that owns it.
- `crates/browser-cmd/src/api/command.rs` is the one command catalogue: `--help`, argument parsing and
  dispatch all read it. [docs/commands.md](docs/commands.md) is generated from its help text.
- `crates/browser-cmd/src/tests/` real-Chrome tests; `crates/browser-cmd/tests/` tests of the built binary.
- `plugin/` the Claude Code plugin; `.claude-plugin/marketplace.json` lists it.
- `docs/` user documentation; `install.sh` the installer; `.github/workflows/` CI and releases.

## Conventions

- Minimal code: the fewest lines that do the job. Remove dead code; do not comment it out.
- One file, one responsibility. Composition over inheritance.
- Unit tests sit beside their code; test names say the behaviour they check.
- Every `BROWSER_CMD_*` variable is read through `src/env.rs`, strictly: a value that does not parse is an
  error. A new one is listed in [docs/configuration.md](docs/configuration.md).
- A new or changed command updates its `about` text, then `docs/commands.md`, and the skill if an agent
  needs to know.
- The crate version in `Cargo.toml` and `plugin/.claude-plugin/plugin.json` move together; CI checks it.

## Browser hygiene

Close only what you started. Use a fresh profile (`BROWSER_CMD_PROFILE`), an explicit session and a
separate daemon address (`BROWSER_CMD_ADDR`) for browser checks. Never attach to or kill a personal
Chrome, and never read browser profiles or credentials.

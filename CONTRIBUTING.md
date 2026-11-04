# Contributing

Issues and pull requests are welcome. For a security problem, see [SECURITY.md](SECURITY.md) instead.

Before opening a pull request:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Tests that drive a real Chrome on local fixtures are `#[ignore]`d. If your change touches the browser,
run them on a machine with Google Chrome:

```sh
BROWSER_CMD_HEADLESS=1 cargo test --workspace --locked -- --include-ignored --skip public_
```

Keep changes small and the code minimal: the fewest lines that do the job, one file per responsibility,
no commented-out code. [ARCHITECTURE.md](ARCHITECTURE.md) explains the layers (a test enforces them) and
[AGENTS.md](AGENTS.md) the conventions. A change to a command updates its help text in
`crates/browser-cmd/src/api/command.rs` and [docs/commands.md](docs/commands.md); a new environment
variable goes into [docs/configuration.md](docs/configuration.md).

By contributing you agree that your contributions are licensed under the [Apache-2.0](LICENSE) license.

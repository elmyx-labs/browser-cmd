#!/bin/sh
# Installs the browser-cmd binary from GitHub Releases, checked against the release's SHA256SUMS, and its
# agent skill for every coding agent found on this machine (Claude Code, Codex, Pi).
set -eu

REPO=elmyx-labs/browser-cmd
BIN_DIR=${BROWSER_CMD_INSTALL_DIR:-$HOME/.local/bin}
VERSION=${BROWSER_CMD_INSTALL_VERSION:-latest}

usage() {
	cat <<EOF
Usage: install.sh [--no-skills]

Installs browser-cmd into \$BROWSER_CMD_INSTALL_DIR (default ~/.local/bin) and the browser-cmd skill
into ~/.claude/skills (Claude Code) and ~/.agents/skills (Codex, Pi) when those agents are installed.
\$BROWSER_CMD_INSTALL_VERSION picks a release tag such as v0.1.0 (default: the latest).

  --no-skills   install only the binary
EOF
}

fail() {
	echo "install.sh: $*" >&2
	exit 1
}

skills=1
for arg in "$@"; do
	case $arg in
	--no-skills) skills=0 ;;
	-h | --help) usage && exit 0 ;;
	*) usage >&2 && exit 2 ;;
	esac
done

case $(uname -s) in
Linux) os=unknown-linux-musl ;;
Darwin) os=apple-darwin ;;
*) fail "no prebuilt binary for $(uname -s); see https://github.com/$REPO#install" ;;
esac
case $(uname -m) in
x86_64 | amd64) arch=x86_64 ;;
arm64 | aarch64) arch=aarch64 ;;
*) fail "no prebuilt binary for $(uname -m); see https://github.com/$REPO#install" ;;
esac
target=$arch-$os
asset=browser-cmd-$target.tar.gz
if [ "$VERSION" = latest ]; then
	base=https://github.com/$REPO/releases/latest/download
else
	base=https://github.com/$REPO/releases/download/$VERSION
fi

fetch() {
	if command -v curl >/dev/null 2>&1; then
		curl -fsSL -o "$2" "$1"
	else
		wget -qO "$2" "$1"
	fi
}

sha256() {
	if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1
}

# Copies a file into a directory that may be outside what a sandbox lets this script write.
put() {
	mkdir -p "$2" && cp "$1" "$2/$3" || fail "could not write $2/$3; run this installer outside the sandbox, or as the user who owns $2"
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if ! fetch "$base/$asset" "$tmp/$asset" || ! fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS"; then
	cat >&2 <<EOF
install.sh: no prebuilt browser-cmd for $target at $base (no network, or no such release).
Build it from source instead (Rust 1.89 or newer, https://rustup.rs):
  cargo install --locked --git https://github.com/$REPO browser-cmd
EOF
	exit 1
fi
expected=$(awk -v asset="$asset" '$2 == asset { print $1 }' "$tmp/SHA256SUMS")
[ -n "$expected" ] && [ "$expected" = "$(sha256 "$tmp/$asset")" ] || fail "checksum mismatch for $asset"

tar -xzf "$tmp/$asset" -C "$tmp"
unpacked=$tmp/browser-cmd-$target
put "$unpacked/browser-cmd" "$BIN_DIR" browser-cmd
chmod 755 "$BIN_DIR/browser-cmd"
echo "Installed $("$BIN_DIR/browser-cmd" --version) to $BIN_DIR"
case :$PATH: in
*:"$BIN_DIR":*) ;;
*) echo "Add $BIN_DIR to your PATH: export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

# The skill goes where each agent on this machine reads skills: Claude Code in ~/.claude/skills,
# Codex and Pi in ~/.agents/skills. An agent whose directory is absent is not installed here.
if [ $skills = 1 ]; then
	skill=$unpacked/skills/browser-cmd/SKILL.md
	installed=
	if [ -d "$HOME/.claude" ]; then
		put "$skill" "$HOME/.claude/skills/browser-cmd" SKILL.md
		installed="$installed Claude Code (/browser-cmd)"
	fi
	if [ -d "$HOME/.codex" ] || [ -d "$HOME/.pi" ] || [ -d "$HOME/.agents" ]; then
		put "$skill" "$HOME/.agents/skills/browser-cmd" SKILL.md
		[ -d "$HOME/.codex" ] && installed="$installed Codex (\$browser-cmd)"
		[ -d "$HOME/.pi" ] && installed="$installed Pi (/skill:browser-cmd)"
	fi
	if [ -n "$installed" ]; then
		echo "Installed the browser-cmd skill for:$installed"
		echo "A new session sees it; a running one may need /reload-skills (Claude Code) or /reload (Pi)."
	else
		echo "No coding agent found (~/.claude, ~/.codex, ~/.pi); the skill is skills/browser-cmd/SKILL.md in the release archive."
	fi
fi

# browser-cmd finds a stable Google Chrome itself; this only warns early when there is none.
chrome=
if [ "$os" = apple-darwin ]; then
	for app in "/Applications/Google Chrome.app" "$HOME/Applications/Google Chrome.app"; do
		if [ -d "$app" ]; then chrome=$app; fi
	done
else
	for bin in google-chrome google-chrome-stable /opt/google/chrome/chrome; do
		if command -v "$bin" >/dev/null 2>&1; then chrome=$bin; fi
	done
fi
[ -n "$chrome" ] || cat <<EOF
Google Chrome (stable) was not found. browser-cmd drives an installed Chrome: install it from
https://www.google.com/chrome/, or set BROWSER_CMD_EXECUTABLE to a Chrome binary, or BROWSER_CMD_CDP
to the DevTools endpoint of a running one (see https://github.com/$REPO/blob/main/docs/your-chrome.md).
EOF

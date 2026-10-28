#!/bin/sh
# Installs the browser-cmd binary from GitHub Releases, checked against the release's SHA256SUMS.
set -eu

REPO=elmyx-labs/browser-cmd
BIN_DIR=${BROWSER_CMD_INSTALL_DIR:-$HOME/.local/bin}
VERSION=${BROWSER_CMD_INSTALL_VERSION:-latest}

usage() {
	cat <<EOF
Usage: install.sh [--codex] [--claude]

Installs browser-cmd into \$BROWSER_CMD_INSTALL_DIR (default ~/.local/bin).
\$BROWSER_CMD_INSTALL_VERSION picks a release tag such as v0.1.0 (default: the latest).

  --codex   also install the browser-cmd skill for Codex into ~/.agents/skills/browser-cmd
  --claude  also print the commands that install the Claude Code plugin
EOF
}

fail() {
	echo "install.sh: $*" >&2
	exit 1
}

codex=0
claude=0
for arg in "$@"; do
	case $arg in
	--codex) codex=1 ;;
	--claude) claude=1 ;;
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

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if ! fetch "$base/$asset" "$tmp/$asset" || ! fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS"; then
	cat >&2 <<EOF
install.sh: no prebuilt browser-cmd for $target at $base.
Build it from source instead (Rust 1.89 or newer, https://rustup.rs):
  cargo install --locked --git https://github.com/$REPO browser-cmd
EOF
	exit 1
fi
expected=$(awk -v asset="$asset" '$2 == asset { print $1 }' "$tmp/SHA256SUMS")
[ -n "$expected" ] && [ "$expected" = "$(sha256 "$tmp/$asset")" ] || fail "checksum mismatch for $asset"

tar -xzf "$tmp/$asset" -C "$tmp"
unpacked=$tmp/browser-cmd-$target
mkdir -p "$BIN_DIR"
cp "$unpacked/browser-cmd" "$BIN_DIR/browser-cmd"
chmod 755 "$BIN_DIR/browser-cmd"
echo "Installed $("$BIN_DIR/browser-cmd" --version) to $BIN_DIR"
case :$PATH: in
*:"$BIN_DIR":*) ;;
*) echo "Add $BIN_DIR to your PATH." ;;
esac

if [ $codex = 1 ]; then
	skill=$HOME/.agents/skills/browser-cmd
	mkdir -p "$skill"
	cp "$unpacked/skills/browser-cmd/SKILL.md" "$skill/SKILL.md"
	echo "Installed the Codex skill to $skill"
fi

if [ $claude = 1 ]; then
	cat <<EOF
Install the Claude Code plugin with:
  claude plugin marketplace add $REPO
  claude plugin install browser-cmd@browser-cmd
EOF
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
to the DevTools endpoint of a Chrome you started with --remote-debugging-port.
EOF

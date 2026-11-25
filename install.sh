#!/bin/sh
# Installs the newest Sage on a Mac:
#
#   curl -fsSL https://raw.githubusercontent.com/Aakshar-D/dots/master/install.sh | sh
#
# It reads the newest release's `latest.json`, downloads the app from this repository's
# releases and puts it in Applications. A file fetched this way is not marked as coming from
# the internet, so macOS does not stop it the first time it is opened.
#
# Set before running, when wanted:
#   SAGE_VERSION=0.5.4   installs that version, not the newest
#   SAGE_DRY_RUN=1       says what it would download, and stops
set -eu

repo="Aakshar-D/dots"
releases="https://github.com/$repo/releases"

fail() {
  echo "install: $1" >&2
  exit 1
}

# A dry run is also the release's own check, which runs on other systems.
if [ -z "${SAGE_DRY_RUN:-}" ] && [ "$(uname -s)" != "Darwin" ]; then
  fail "this installs Sage on a Mac. On Windows, use install.ps1 (see the README)."
fi

# The release to install: the newest, or the one asked for.
base="$releases/latest/download"
if [ -n "${SAGE_VERSION:-}" ]; then
  base="$releases/download/v${SAGE_VERSION#v}"
fi

# A manifest on disk is for the release's own check (tools/release/install.test.mjs).
if [ -n "${SAGE_MANIFEST_FILE:-}" ]; then
  manifest=$(cat "$SAGE_MANIFEST_FILE")
else
  manifest=$(curl -fsSL "$base/latest.json") ||
    fail "Sage's release could not be read from $base/latest.json. Check your connection, or the version asked for."
fi
# On one line, so a pattern can read across what were lines.
manifest=$(printf '%s' "$manifest" | tr -d '\r\n')

case "$(uname -m)" in
  arm64 | aarch64) platform="darwin-aarch64" ;;
  *) platform="darwin-x86_64" ;;
esac

# The value of `key` in the manifest's entry for `platform`, or of a top-level `key`.
field() {
  printf '%s' "$manifest" | sed -n "s/.*\"$1\"[^}]*\"$2\" *: *\"\([^\"]*\)\".*/\1/p"
}
version=$(printf '%s' "$manifest" | sed -n 's/^[^}]*"version" *: *"\([^"]*\)".*/\1/p')
url=$(field "$platform" url)
[ -n "$url" ] || fail "Sage ${version:-(unknown version)} has no Mac app."

# Only a file of this repository's releases is ever installed.
case "$url" in
  "$releases/download/"*) ;;
  *) fail "the release names a download outside $releases, so nothing was installed." ;;
esac

echo "Sage $version: $url"
if [ -n "${SAGE_DRY_RUN:-}" ]; then
  exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fL --progress-bar "$url" -o "$work/sage.tar.gz" || fail "the download failed."
tar -xzf "$work/sage.tar.gz" -C "$work" || fail "the download could not be unpacked."
app=$(find "$work" -maxdepth 1 -name '*.app' | head -n 1)
[ -n "$app" ] || fail "the download held no app."
name=$(basename "$app")

# Applications for everyone when this account may write there, else this account's own.
target="/Applications"
if [ ! -w "$target" ]; then
  target="$HOME/Applications"
  mkdir -p "$target"
fi

if pgrep -x Sage >/dev/null 2>&1 || pgrep -x dots-app >/dev/null 2>&1; then
  fail "Sage is running. Quit it (the menu bar icon, then Quit), and run this again."
fi

# The Sage already there is set aside, and put back when the new one cannot take its place.
old=""
if [ -e "$target/$name" ]; then
  old="$work/old.app"
  mv "$target/$name" "$old" || fail "the Sage already in $target could not be replaced."
fi
if ! mv "$app" "$target/$name"; then
  [ -z "$old" ] || mv "$old" "$target/$name"
  fail "Sage could not be put in $target."
fi
echo "Sage $version is installed in $target. Opening it."
open "$target/$name"

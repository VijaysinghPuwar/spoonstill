#!/usr/bin/env bash
# spoonstill — one-line installer for macOS (D-087).
#
#   curl -fsSL https://raw.githubusercontent.com/VijaysinghPuwar/spoonstill/master/scripts/install.sh | bash
#
# What it does, in order, and nothing else:
#   1. works out which build this machine needs
#   2. downloads that build, and verifies it against the release's SHA256SUMS.txt
#   3. installs the `still` binary into ~/.local/bin
#   4. installs the window into /Applications, without the Gatekeeper dialog
#   5. checks for FFmpeg, and offers to install it through Homebrew
#
# It never uses sudo and never downloads FFmpeg itself — D-012 forbids a runtime
# binary download, and D-062 forbids shipping the GPL build this project
# develops against. Step 4 writes to /Applications when that is possible
# without sudo and to ~/Applications when it is not (D-098).
#
# Step 4 exists because of one dialog. An unsigned app downloaded through a
# browser carries `com.apple.quarantine`, and on macOS 15 and later Apple
# removed the right-click > Open escape hatch that every instruction on the
# internet still names. What the operator gets instead is
#
#   "spoonstill" Not Opened — Apple could not verify "spoonstill" is free of
#   malware ...            [Move to Trash]  [Done]
#
# where the highlighted button deletes the thing they just downloaded. An
# installer running under their own hand can remove that attribute, and then
# the app simply opens. Signing and notarization (M5) fix it properly.

SKIP_APP="${SPOONSTILL_SKIP_APP:-}"     # set to 1 to install only the CLI

set -euo pipefail

REPO="${SPOONSTILL_REPO:-VijaysinghPuwar/spoonstill}"
INSTALL_DIR="${SPOONSTILL_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${SPOONSTILL_VERSION:-latest}"

bold=$(tput bold 2>/dev/null || printf '')
dim=$(tput dim 2>/dev/null || printf '')
red=$(tput setaf 1 2>/dev/null || printf '')
green=$(tput setaf 2 2>/dev/null || printf '')
reset=$(tput sgr0 2>/dev/null || printf '')

say()  { printf '%s\n' "$*"; }
step() { printf '%s==>%s %s\n' "$bold" "$reset" "$*"; }
warn() { printf '%s!%s   %s\n' "$red" "$reset" "$*" >&2; }
die()  { warn "$*"; exit 1; }

# --- 1. which build ----------------------------------------------------------

case "$(uname -s)" in
  Darwin) ;;
  Linux)
    die "There is no published Linux build yet. The code is portable — build it:
      git clone https://github.com/$REPO.git && cd spoonstill
      cargo build --release -p spoonstill-cli" ;;
  *) die "Unsupported system: $(uname -s). Use scripts/install.ps1 on Windows." ;;
esac

case "$(uname -m)" in
  arm64)  TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *) die "Unsupported architecture: $(uname -m)" ;;
esac

step "Installing spoonstill for macOS ${dim}($TARGET)${reset}"

# --- 2. download and verify --------------------------------------------------

api="https://api.github.com/repos/$REPO/releases/latest"
if [ "$VERSION" = "latest" ]; then
  # The failure is captured, not propagated. `set -euo pipefail` is on, so a
  # curl that exits non-zero — an anonymous rate-limit 403, an offline machine,
  # a repository with no releases — used to kill the script *at this line*, and
  # the message below could never print. What the operator saw was
  # `curl: (56) The requested URL returned error: 403` and nothing else, which
  # is the one thing D-123 says an installer must not do.
  #
  # An API token is used when one happens to be in the environment. Nobody
  # installing this needs one; it is here because the anonymous limit is per
  # IP, and behind one office NAT or on a CI runner that limit is shared with
  # strangers.
  auth=""
  token="${GITHUB_TOKEN:-${GH_TOKEN:-}}"
  [ -n "$token" ] && auth="Authorization: Bearer $token"

  if [ -n "$auth" ]; then
    body=$(curl -fsSL -H "$auth" "$api" 2>/dev/null) || body=""
  else
    body=$(curl -fsSL "$api" 2>/dev/null) || body=""
  fi

  TAG=$(printf '%s' "$body" | sed -n 's/.*"tag_name" *: *"\([^"]*\)".*/\1/p' | head -n1)
  [ -n "$TAG" ] || die "Could not ask GitHub which release is the latest.

  That request is unauthenticated and GitHub limits it per IP address, so this
  is usually a shared network rather than anything wrong with your machine.

  Install a known version instead, which skips the question entirely:
      SPOONSTILL_VERSION=v0.1.5 curl -fsSL <this script> | bash

  Releases: https://github.com/$REPO/releases
  Or build from source: cargo build --release -p spoonstill-cli"
else
  TAG="$VERSION"
fi

case "$TARGET" in
  aarch64-apple-darwin) ASSET="still-macOS-AppleSilicon.tar.gz" ;;
  *)                    ASSET="still-macOS-Intel.tar.gz" ;;
esac
BASE="https://github.com/$REPO/releases/download/${TAG}"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

step "Downloading $ASSET"
curl -fSL --progress-bar -o "$tmp/$ASSET" "$BASE/$ASSET" \
  || die "Could not download $BASE/$ASSET"

# One list for the whole release rather than a `.sha256` beside every asset
# (D-133), so the release page is five downloads and not eleven. `verify` picks
# out this file's line; a name that is not in the list is a failure and not a
# skip, which is the whole point of checking.
SUMS="SHA256SUMS.txt"
curl -fsSL -o "$tmp/$SUMS" "$BASE/$SUMS" \
  || die "Could not download $SUMS. Refusing to install unverified."

verify() {
  line=$(grep -E "[ *]$1\$" "$tmp/$SUMS" || true)
  [ -n "$line" ] \
    || die "$1 is not listed in $SUMS — refusing to install something the release does not vouch for."
  ( cd "$tmp" && printf '%s\n' "$line" | shasum -a 256 -c - >/dev/null ) \
    || die "Checksum mismatch on $1. The download is not the published build — nothing installed."
}

step "Verifying checksum"
verify "$ASSET"

# --- 3. install --------------------------------------------------------------

tar -xzf "$tmp/$ASSET" -C "$tmp"
[ -f "$tmp/still" ] || die "The archive did not contain a 'still' binary."

mkdir -p "$INSTALL_DIR"
# Beside, then over (D-128). `install` opens the destination and truncates it,
# so a write that fails part-way leaves the operator with neither the build they
# had nor the one they asked for. `mv` within one directory replaces in one
# step — the same rule `move_into_place` follows for every artifact (D-119).
install -m 0755 "$tmp/still" "$INSTALL_DIR/still.new"
mv -f "$INSTALL_DIR/still.new" "$INSTALL_DIR/still"
# These builds are unsigned until M5. Clear the download quarantine so the
# first run is not a dialog the operator has no way to interpret.
xattr -d com.apple.quarantine "$INSTALL_DIR/still" 2>/dev/null || true

say "${green}Installed${reset} $INSTALL_DIR/still  ($("$INSTALL_DIR/still" --version 2>/dev/null || echo "$TAG"))"

# --- 4. the window -----------------------------------------------------------

if [ -n "$SKIP_APP" ]; then
  say "${dim}Skipping${reset}  the window (SPOONSTILL_SKIP_APP is set)"
else
  APP="spoonstill-macOS.dmg"
  step "Downloading $APP"
  if curl -fSL --progress-bar -o "$tmp/$APP" "$BASE/$APP"; then

    verify "$APP"

    mnt="$tmp/mnt"
    mkdir -p "$mnt"
    hdiutil attach -nobrowse -quiet -mountpoint "$mnt" "$tmp/$APP" \
      || die "Could not open $APP."

    # Detaching has to happen however this ends, or the volume is left mounted.
    trap 'hdiutil detach "$mnt" -quiet 2>/dev/null || true; rm -rf "$tmp"' EXIT

    src=$(find "$mnt" -maxdepth 1 -name '*.app' -print -quit)
    [ -n "$src" ] || die "$APP did not contain an application."

    apps="/Applications"
    [ -w "$apps" ] || apps="$HOME/Applications"
    mkdir -p "$apps"
    rm -rf "$apps/$(basename "$src")"
    cp -R "$src" "$apps/" || die "Could not copy the app into $apps."

    hdiutil detach "$mnt" -quiet 2>/dev/null || true
    trap 'rm -rf "$tmp"' EXIT

    # The whole reason this step exists. Without it the first launch is a
    # dialog whose brightest button is "Move to Trash".
    xattr -dr com.apple.quarantine "$apps/$(basename "$src")" 2>/dev/null || true

    say "${green}Installed${reset} $apps/$(basename "$src")  ${dim}(opens without a Gatekeeper prompt)${reset}"
  else
    warn "Could not download the window; the command line above is installed and complete."
  fi
fi

# --- 5. FFmpeg ---------------------------------------------------------------

if command -v ffmpeg >/dev/null 2>&1 && command -v ffprobe >/dev/null 2>&1; then
  say "${green}Found${reset}     $(command -v ffmpeg)"
else
  step "FFmpeg is missing — spoonstill cannot render a frame without it"
  if command -v brew >/dev/null 2>&1; then
    say "    Installing it with Homebrew…"
    brew install ffmpeg || warn "Homebrew could not install ffmpeg. Install it yourself, then re-run."
  else
    warn "Homebrew is not installed. Install FFmpeg by hand, then you are done:
      /bin/bash -c \"\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"
      brew install ffmpeg"
  fi
fi

if ! command -v edge-tts >/dev/null 2>&1; then
  say "${dim}Optional:${reset}  'pipx install edge-tts' if you want text read aloud by a neural voice."
fi

# --- PATH --------------------------------------------------------------------

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    profile="$HOME/.zshrc"; [ "${SHELL##*/}" = "bash" ] && profile="$HOME/.bash_profile"
    say ""
    warn "$INSTALL_DIR is not on your PATH. Add it:
      echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> $profile && exec \$SHELL" ;;
esac

cat <<EOF

${bold}Ready.${reset} Make a film out of a folder you already have:

  still new ~/holiday ~/Pictures/trip/*.jpg
  still validate ~/holiday
  still render ~/holiday --out ~/holiday.mp4

  still --help            every command
EOF

#!/usr/bin/env bash
# spoonstill — one-line installer for macOS (D-087).
#
#   curl -fsSL https://raw.githubusercontent.com/VijaysinghPuwar/spoonstill/master/scripts/install.sh | bash
#
# What it does, in order, and nothing else:
#   1. works out which build this machine needs
#   2. downloads that build and the checksum published beside it, and verifies
#   3. installs the `still` binary into ~/.local/bin
#   4. checks for FFmpeg, and offers to install it through Homebrew
#
# It never uses sudo, never writes outside $HOME, and never downloads FFmpeg
# itself — D-012 forbids a runtime binary download, and D-062 forbids shipping
# the GPL build this project develops against.

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
  TAG=$(curl -fsSL "$api" | sed -n 's/.*"tag_name" *: *"\([^"]*\)".*/\1/p' | head -n1)
  [ -n "$TAG" ] || die "No published release found at https://github.com/$REPO/releases.
Until one exists, build from source: cargo build --release -p spoonstill-cli"
else
  TAG="$VERSION"
fi

ASSET="still-${TAG}-${TARGET}.tar.gz"
BASE="https://github.com/$REPO/releases/download/${TAG}"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

step "Downloading $ASSET"
curl -fSL --progress-bar -o "$tmp/$ASSET"        "$BASE/$ASSET" \
  || die "Could not download $BASE/$ASSET"
curl -fsSL              -o "$tmp/$ASSET.sha256"  "$BASE/$ASSET.sha256" \
  || die "Could not download the checksum for $ASSET. Refusing to install unverified."

step "Verifying checksum"
( cd "$tmp" && shasum -a 256 -c "$ASSET.sha256" >/dev/null ) \
  || die "Checksum mismatch. The download is not the published build — nothing installed."

# --- 3. install --------------------------------------------------------------

tar -xzf "$tmp/$ASSET" -C "$tmp"
[ -f "$tmp/still" ] || die "The archive did not contain a 'still' binary."

mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmp/still" "$INSTALL_DIR/still"
# These builds are unsigned until M5. Clear the download quarantine so the
# first run is not a dialog the operator has no way to interpret.
xattr -d com.apple.quarantine "$INSTALL_DIR/still" 2>/dev/null || true

say "${green}Installed${reset} $INSTALL_DIR/still  ($("$INSTALL_DIR/still" --version 2>/dev/null || echo "$TAG"))"

# --- 4. FFmpeg ---------------------------------------------------------------

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

#!/usr/bin/env bash
# Generate the synthetic fixtures of plan.md "Fixtures, committed once and
# reused". Generate what can be generated; commit only what cannot.
#
# Everything here is derived deterministically from ffmpeg's own sources, so
# fixtures/generated/ is gitignored and rebuilt rather than stored.
set -euo pipefail

OUT="fixtures/generated"
mkdir -p "$OUT"
FF="ffmpeg -hide_banner -loglevel error -y"

say() { printf '  %-34s %s\n' "$1" "$2"; }

# --- geometry: one source per shape the motion matrix has to cover -----------
# testsrc2 gives sharp edges and gradients, which is what makes zoom stepping
# (D-032) and black-edge bleed (D-034) visible rather than subtle.
gen_still() { # name WxH
  $FF -f lavfi -i "testsrc2=size=$2:rate=1" -frames:v 1 "$OUT/$1" 2>/dev/null
  say "$1" "$2"
}
gen_still land.jpg   4000x3000   # landscape -> every aspect
gen_still port.jpg   3000x4000   # portrait  -> every aspect
gen_still square.jpg 2000x2000   # square    -> every aspect

# The D-033 SAR trap needs genuinely ODD dimensions, and getting them takes
# three deliberate steps — each of which silently rounds to even if you skip it:
#
#   1. `testsrc2=size=1999x1001`  -> emits 1998x1000. Source filters round.
#   2. `crop=1999:1001`           -> still 1998x1000. crop aligns to chroma
#                                    boundaries unless `exact=1` is set.
#   3. default jpeg pix_fmt       -> yuvj420p cannot represent odd dimensions.
#
# A fixture that quietly comes out even makes the D-033 regression test pass for
# the wrong reason and never catch BUG W2-1 — the exact bug it exists to catch.
# So: crop with exact=1, and encode 4:4:4.
$FF -f lavfi -i "testsrc2=size=2000x1002:rate=1" \
    -vf "crop=1999:1001:exact=1" -pix_fmt yuvj444p -frames:v 1 "$OUT/odd.jpg"
say "odd.jpg" "1999x1001 (genuinely odd — crop exact=1 + yuvj444p)"

# Assert it, because "generated successfully" is not the same as "odd".
odd_dims=$(ffprobe -v error -select_streams v:0 \
    -show_entries stream=width,height -of csv=p=0 "$OUT/odd.jpg")
if [ "$odd_dims" != "1999,1001" ]; then
  echo "FATAL: odd.jpg is $odd_dims, expected 1999,1001." >&2
  echo "       An even fixture makes the D-033 SAR test vacuous." >&2
  exit 1
fi

# --- hostile paths (D-052): argument vectors, never shell strings ------------
cp "$OUT/land.jpg" "$OUT/ünïcode spaced 名前.jpg"
say "ünïcode spaced 名前.jpg" "copy of land.jpg"

# --- truncated image: must fail with a named cause, not a panic --------------
head -c 4096 "$OUT/land.jpg" > "$OUT/truncated.jpg"
say "truncated.jpg" "first 4 KiB of land.jpg"

# --- audio ------------------------------------------------------------------
# A real narration-length track. 3.717 s is the exact duration from
# ffmpeg-findings.md §5, so the D-022 padding arithmetic has a known answer:
# at 30 fps it must produce 112 frames / 3.733333 s.
$FF -f lavfi -i "sine=frequency=220:duration=3.717" -ac 2 -ar 48000 "$OUT/n.wav"
say "n.wav" "3.717 s @ 48 kHz stereo (D-022 reference)"

# VBR MP3 whose header duration lies (D-021). Written with -abr so the header
# frame count and the real stream disagree; the point is that ffprobe on the
# *normalized* artifact is what we trust, never this file's header.
$FF -f lavfi -i "sine=frequency=440:duration=5.3" -c:a libmp3lame -q:a 9 \
    "$OUT/vbr_lying_header.mp3" 2>/dev/null || say "vbr_lying_header.mp3" "SKIPPED (no libmp3lame)"
[ -f "$OUT/vbr_lying_header.mp3" ] && say "vbr_lying_header.mp3" "5.3 s VBR"

# Zero-byte audio: must be rejected with the scene ID attached, not crash.
: > "$OUT/zero_byte.mp3"
say "zero_byte.mp3" "0 bytes"

# --- project folders (D-050) ------------------------------------------------
# The M2 gate renders a project "an operator could have produced by hand", so
# these are built out of the stills and audio above rather than described in
# prose. Both modes, because D-050 has two and the manifest wins.
PROJECTS="fixtures/projects"

echo
echo "  projects:"

# Convention mode: stem-keyed pairing, one scene per source (D-050).
#   001 image + text  -> TTS
#   002 image + audio -> supplied
#   003 image alone   -> silent, default duration
MIXED="$PROJECTS/mixed"
rm -rf "$MIXED"; mkdir -p "$MIXED"
cp "$OUT/land.jpg"   "$MIXED/001.jpg"
printf 'A still photograph, held for as long as this line takes to say.\n' \
     > "$MIXED/001.txt"
cp "$OUT/port.jpg"   "$MIXED/002.jpg"
cp "$OUT/n.wav"      "$MIXED/002.wav"
cp "$OUT/square.jpg" "$MIXED/003.jpg"
say "$MIXED" "3 scenes, convention mode"

# Manifest mode: the same three scenes, spelled out, plus the per-scene motion
# overrides that only a manifest can express.
CSV="$PROJECTS/manifest"
rm -rf "$CSV"; mkdir -p "$CSV/img" "$CSV/audio"
cp "$OUT/land.jpg"   "$CSV/img/opening.jpg"
cp "$OUT/port.jpg"   "$CSV/img/middle.jpg"
cp "$OUT/square.jpg" "$CSV/img/closing.jpg"
cp "$OUT/n.wav"      "$CSV/audio/middle.wav"
cat > "$CSV/project.yaml" <<'YAML'
# An input. spoonstill never writes to this file (D-013).
output: film.mp4
aspect: 16:9
short_edge: 1080
fps: 30
defaults:
  duration: 3.0
YAML
cat > "$CSV/scenes.csv" <<'CSV_ROWS'
image,text,audio_file,voice,duration,zoom_direction,zoom_anchor
img/opening.jpg,A line to be spoken over the opening still.,,,,zoom-in,center
img/middle.jpg,,audio/middle.wav,,,pan-right,north
img/closing.jpg,,,,4.5,zoom-out,south-east
CSV_ROWS
say "$CSV" "3 scenes, manifest mode"

# --- fixtures ffmpeg cannot synthesize --------------------------------------
# CMYK JPEG and EXIF-rotated JPEG need an encoder that writes those tags.
# They are listed in plan.md as commit-only fixtures; recorded here so the gap
# is visible rather than silently missing.
echo
echo "  not generated (commit these by hand, plan.md fixtures table):"
echo "    cmyk.jpg          — needs a CMYK-capable encoder"
echo "    exif_rotated.jpg  — needs an EXIF Orientation tag writer"

echo
echo "fixtures in $OUT:"
ls -la "$OUT" | tail -n +2 | awk '{printf "  %8s  %s\n",$5,$9" "$10" "$11}'

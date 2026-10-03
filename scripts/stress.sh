#!/usr/bin/env bash
# The author's real 431-scene project as a stress test (D-195).
#
#   scripts/stress.sh DIR            fetch it into DIR and render it cold
#   scripts/stress.sh DIR 40         the same photographs and lines, forty
#                                    times over: 17 240 scenes, every one a
#                                    distinct still so every segment encodes
#
# The fixture is the release asset `stress-431.zip` on the `stress-fixture`
# pre-release: 431 photographs, 431 lines, project.yaml. A cold render speaks
# all 431 lines through the voice service — that is the point of it, because
# that is where D-195 was found — so do not run it in a loop. The 40x project
# repeats the same 431 lines, so it speaks nothing more than the 1x one does.
#
# Needs: still (target/release or PATH), curl, unzip, edge-tts, FFmpeg.
set -euo pipefail

dir=${1:?usage: scripts/stress.sh DIR [TIMES]}
times=${2:-1}
url=https://github.com/VijaysinghPuwar/spoonstill/releases/download/stress-fixture/stress-431.zip
still=${STILL:-$(dirname "$0")/../target/release/still}
[ -x "$still" ] || still=still

mkdir -p "$dir"
zip="$dir/stress-431.zip"
[ -f "$zip" ] || curl -fL --retry 3 -o "$zip" "$url"

src="$dir/stress-431"
if [ ! -f "$src/project.yaml" ]; then
  mkdir -p "$src"
  unzip -q -o "$zip" -d "$src"
fi

project=$src
if [ "$times" -gt 1 ]; then
  project="$dir/stress-x$times"
  mkdir -p "$project"
  cp "$src/project.yaml" "$project/"
  n=0
  for ((r = 0; r < times; r++)); do
    for ((i = 1; i <= 431; i++)); do
      n=$((n + 1))
      from=$(printf '%03d' "$i")
      to=$(printf '%05d' "$n")
      [ -f "$project/$to.jpg" ] && continue
      cp "$src/$from.jpg" "$project/$to.jpg"
      # Bytes after the JPEG's end marker: decoders ignore them, the content
      # hash does not, so every copy is its own segment.
      [ "$r" -gt 0 ] && printf 'x%d' "$r" >>"$project/$to.jpg"
      cp "$src/$from.txt" "$project/$to.txt"
    done
  done
fi

"$still" validate "$project"
time "$still" render "$project" --out "$dir/stress-x$times.mp4" \
  --resolution 720p --voice en-US-AndrewMultilingualNeural --subtitles boxed

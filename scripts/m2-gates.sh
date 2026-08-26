#!/usr/bin/env bash
# The M2 exit gates from plan.md, run as written there, plus the three that
# slice 3 added when rendering became parallel.
#
# Companion to m0-gates.sh and m1-gates.sh. Each gate is a command with an
# observable result; none of them paraphrases the promise into something
# easier to satisfy.
#
# Note on the render gate's fixture: plan.md names `fixtures/projects/mixed/`,
# which contains a TTS scene. TTS is M2 slice 4, so until it lands the render
# gate runs against `fixtures/projects/renderable/` — the same shape without a
# spoken line — and gate 7 asserts that `mixed` fails for exactly one reason
# and names it. When slice 4 lands, gate 7 becomes the `mixed` render.
set -uo pipefail

cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

GREEN=$'\033[32m'; RED=$'\033[31m'; DIM=$'\033[2m'; OFF=$'\033[0m'
pass=0; fail=0
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

check() { # description, then a command
  local what="$1"; shift
  if "$@" >"$WORK/out" 2>&1; then
    printf '  %sPASS%s  %s\n' "$GREEN" "$OFF" "$what"; pass=$((pass+1))
  else
    printf '  %sFAIL%s  %s\n' "$RED" "$OFF" "$what"; fail=$((fail+1))
    sed 's/^/          /' "$WORK/out" | tail -12
  fi
}

echo "M2 exit gates — plan.md"
echo

[ -d fixtures/projects/renderable ] || bash scripts/gen-fixtures.sh >/dev/null 2>&1

cargo build --release -p spoonstill-cli >/dev/null 2>&1 || {
  echo "  ${RED}FAIL${OFF}  the CLI does not build"; exit 1; }
STILL=./target/release/still

RENDERABLE=fixtures/projects/renderable
# Every gate starts from a cold project, so a cached artifact from a previous
# run can never be the reason a gate passes.
rm -rf "$RENDERABLE/.spoonstill" fixtures/projects/mixed/.spoonstill

# --- gate 1: validate reports a clean project cleanly -----------------------
gate_validate() {
  local out
  out=$("$STILL" validate fixtures/projects/mixed/) || return 1
  grep -q '3 scenes — 1 narrated, 1 supplied, 1 silent' <<<"$out" || { echo "$out"; return 1; }
  grep -q 'no problems' <<<"$out" || { echo "$out"; return 1; }
}
check "still validate fixtures/projects/mixed/ — 3 scenes, 3 sources, 0 warnings" gate_validate

# --- gate 2: the render, and it is frame-exact ------------------------------
# plan.md: the film's duration equals the sum of the resolved scene durations,
# within one frame. The sum is computed here from the scene list rather than
# from the film, so this gate cannot agree with the renderer by construction.
gate_render() {
  "$STILL" render "$RENDERABLE" --out "$WORK/film.mp4" --jobs 4 >"$WORK/render.log" 2>&1 || {
    cat "$WORK/render.log"; return 1; }

  # The expected total, computed here from the *narration* durations the run
  # reported, with D-022's padding applied independently: each scene holds
  # ceil(narration * fps) frames, so the film is the sum of those, not the sum
  # of the raw narrations. Doing that arithmetic here rather than reading the
  # renderer's own segment durations is the point — a gate that asks the
  # renderer what it produced cannot catch the renderer being wrong.
  local sum count
  sum=$(awk '/^  audio /{
      for (i = NF; i >= 1; i--) if ($i ~ /^[0-9]+\.[0-9]+s$/) {
        sub(/s$/, "", $i)
        frames = int($i * 30); if (frames < $i * 30) frames++
        total += frames / 30
        n++
        break
      }
    } END { printf "%.6f %d", total, n }' "$WORK/render.log")
  count=${sum#* }; sum=${sum%% *}

  # The trap this project has fallen into before: a gate that quietly measures
  # nothing. Six scenes went in, six narration lines must have come out.
  [ "$count" = "6" ] || { echo "read $count narration durations, expected 6"; return 1; }

  local film video
  film=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$WORK/film.mp4")
  video=$(ffprobe -v error -select_streams v:0 -show_entries stream=duration -of csv=p=0 "$WORK/film.mp4")

  # The video track is the frame-exact one. The container's duration is the
  # longest track's, and AAC priming puts the audio track one AAC frame (21 ms)
  # beyond it — measured, ffmpeg-findings.md §10c.
  awk -v a="$video" -v b="$sum" 'BEGIN { d = a - b; if (d < 0) d = -d; exit !(d <= 1/30 + 0.001) }' || {
    echo "video duration $video vs scene sum $sum (tolerance one frame)"; return 1; }
  awk -v a="$film" -v b="$sum" 'BEGIN { d = a - b; if (d < 0) d = -d; exit !(d <= 1/30 + 1024/48000 + 0.001) }' || {
    echo "container duration $film vs scene sum $sum"; return 1; }
}
check "still render — the film's duration is the sum of its scenes, within a frame" gate_render

# --- gate 3: concurrency changes the timing and nothing else ----------------
# The claim the whole pool rests on. Same project, one worker and four, must
# produce the same film byte for byte — motion is seeded before the pool starts
# and results are collected in scene order, so anything else is a bug.
gate_deterministic() {
  rm -rf "$RENDERABLE/.spoonstill"
  "$STILL" render "$RENDERABLE" --out "$WORK/serial.mp4"   --jobs 1 >/dev/null 2>&1 || return 1
  rm -rf "$RENDERABLE/.spoonstill"
  "$STILL" render "$RENDERABLE" --out "$WORK/parallel.mp4" --jobs 4 >/dev/null 2>&1 || return 1

  local a b
  a=$(shasum -a 256 "$WORK/serial.mp4"   | cut -d' ' -f1)
  b=$(shasum -a 256 "$WORK/parallel.mp4" | cut -d' ' -f1)
  [ "$a" = "$b" ] || { echo "--jobs 1 gave $a, --jobs 4 gave $b"; return 1; }
}
check "--jobs 1 and --jobs 4 produce the same film, byte for byte" gate_deterministic

# --- gate 4: a re-run reuses rather than re-encodes (D-042, D-043) ----------
gate_reuse() {
  local out
  out=$("$STILL" render "$RENDERABLE" --out "$WORK/again.mp4" --jobs 4) || return 1
  grep -q '6 narrations from cache, 6 segments reused' <<<"$out" || { echo "$out"; return 1; }
}
check "a second run reuses every narration and every segment" gate_reuse

# --- gate 5: two renders of one project do not interleave -------------------
gate_lock() {
  # Hold the lock the way a crashed run would.
  mkdir -p "$RENDERABLE/.spoonstill"
  echo "pid 999999" > "$RENDERABLE/.spoonstill/render.lock"

  local out
  out=$("$STILL" render "$RENDERABLE" --out "$WORK/locked.mp4" 2>&1)
  local status=$?
  rm -f "$RENDERABLE/.spoonstill/render.lock"

  [ "$status" -ne 0 ] || { echo "a locked project rendered anyway"; return 1; }
  grep -q 'another render is working on this project' <<<"$out" || { echo "$out"; return 1; }
  grep -q -- '--force' <<<"$out" || { echo "$out"; return 1; }
}
check "a second render of one project is refused, and says how to override" gate_lock

# --- gate 6: hostile input survives the whole pipeline ----------------------
# The renderable fixture deliberately contains `odd.jpg` (1999x1001, the D-033
# SAR trap) and a Unicode, spaced filename (D-052). Both have to reach the
# concat list — which is the one text format in the codebase — without
# escaping anything, because segment names are content-addressed.
gate_hostile() {
  local sar
  sar=$(ffprobe -v error -select_streams v:0 -show_entries stream=sample_aspect_ratio \
        -of csv=p=0 "$WORK/film.mp4")
  [ "$sar" = "1:1" ] || { echo "SAR is $sar, not 1:1 (D-033)"; return 1; }
  ls "$RENDERABLE/.spoonstill/segments" | grep -qE '^seg-[0-9]{4}-[0-9a-f]{16}\.mp4$' || {
    ls "$RENDERABLE/.spoonstill/segments"; return 1; }
  # No operator spelling anywhere in the segment directory.
  ! ls "$RENDERABLE/.spoonstill/segments" | grep -qv -E '^seg-[0-9]{4}-[0-9a-f]{16}\.mp4$'
}
check "odd dimensions and a Unicode filename survive the join" gate_hostile

# --- gate 7: TTS is refused by name, not silently muted ---------------------
# D-020: a line somebody wrote must never become silence. Until slice 4 lands,
# `mixed` is the project that proves it.
gate_tts() {
  local out
  out=$("$STILL" render fixtures/projects/mixed/ --out "$WORK/mixed.mp4" 2>&1)
  local status=$?
  rm -rf fixtures/projects/mixed/.spoonstill
  [ "$status" -ne 0 ] || { echo "a TTS scene rendered without TTS"; return 1; }
  grep -q 'text-to-speech' <<<"$out" || { echo "$out"; return 1; }
  grep -q 'slice 4' <<<"$out" || { echo "$out"; return 1; }
  [ ! -f "$WORK/mixed.mp4" ] || { echo "a film was written anyway"; return 1; }
}
check "a TTS scene is refused by name rather than silently muted" gate_tts

# --- gates 8 and 9: the two cargo gates plan.md names -----------------------
check "cargo test -p spoonstill-app validation" \
  cargo test --release -p spoonstill-app validation

check "cargo test -p spoonstill-core path_safety" \
  cargo test --release -p spoonstill-core path_safety

rm -rf "$RENDERABLE/.spoonstill"

echo
total=$((pass+fail))
if [ "$fail" -eq 0 ]; then
  printf '%sM2 SLICES 1-3 COMPLETE%s — %d/%d gates pass\n' "$GREEN" "$OFF" "$pass" "$total"
  printf '%sSlice 4 (TTS) is what closes M2; gate 7 becomes the `mixed` render then.%s\n' "$DIM" "$OFF"
else
  printf '%sM2 INCOMPLETE%s — %d/%d gates pass\n' "$RED" "$OFF" "$pass" "$total"
  printf '%sRun `make test` for detail.%s\n' "$DIM" "$OFF"
  exit 1
fi

#!/usr/bin/env bash
# The M1 exit gates from plan.md, run as written there.
#
# Companion to m0-gates.sh. Each gate is the command plan.md specifies, and the
# assertion is the one plan.md states — not a paraphrase, so that a gate cannot
# drift into checking something easier than what was promised.
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

echo "M1 exit gates — plan.md"
echo

# The fixtures the gates are written against.
[ -f fixtures/generated/land.jpg ] || bash scripts/gen-fixtures.sh >/dev/null 2>&1

cargo build --release -p spoonstill-cli >/dev/null 2>&1 || {
  echo "  ${RED}FAIL${OFF}  the CLI does not build"; exit 1; }
STILL=./target/release/still

# --- gate 1: the single-scene render works and is frame-exact ---------------
gate_one() {
  "$STILL" render-scene --image fixtures/generated/land.jpg \
    --audio fixtures/generated/n.wav --out "$WORK/s.mp4" >/dev/null || return 1
  local probed
  probed=$(ffprobe -v error -select_streams v:0 -count_frames \
    -show_entries stream=nb_read_frames,sample_aspect_ratio,width,height,r_frame_rate \
    -of default=nw=1 "$WORK/s.mp4")
  # narration 3.717021s at 30fps -> ceil = 112 frames (D-022).
  grep -q '^nb_read_frames=112$'      <<<"$probed" || { echo "$probed"; return 1; }
  grep -q '^sample_aspect_ratio=1:1$' <<<"$probed" || { echo "$probed"; return 1; }
  grep -q '^width=1920$'              <<<"$probed" || { echo "$probed"; return 1; }
  grep -q '^height=1080$'             <<<"$probed" || { echo "$probed"; return 1; }
  grep -q '^r_frame_rate=30/1$'       <<<"$probed" || { echo "$probed"; return 1; }
}
check "still render-scene is frame-exact: 112 frames, SAR 1:1, 1920x1080@30" gate_one

# --- gate 2: the motion matrix ----------------------------------------------
check "cargo test -p spoonstill-media --test motion_matrix" \
  cargo test --release -p spoonstill-media --test motion_matrix

# --- gate 3: odd dimensions do not corrupt SAR (the D-033 regression) -------
check "cargo test -p spoonstill-media odd_dimensions_sar" \
  cargo test --release -p spoonstill-media odd_dimensions_sar

# --- gate 4: hostile paths survive ------------------------------------------
gate_four() {
  "$STILL" render-scene --image "fixtures/generated/ünïcode spaced 名前.jpg" \
    --audio fixtures/generated/n.wav --out "$WORK/ünïcode spaced 名前.mp4" \
    --aspect 9:16 --short-edge 360 >/dev/null || return 1
  [ -f "$WORK/ünïcode spaced 名前.mp4" ]
}
check "still render-scene with a Unicode, spaced path" gate_four

# --- gate 5: cancellation is clean ------------------------------------------
# Interrupt the render *once it is demonstrably running*, rather than after a
# fixed sleep.
#
# The fixed sleep was a race: this render takes ~1.24 s and the gate waited
# 1.0 s, so a quiet machine finished the file before the signal arrived and the
# gate failed for a reason that had nothing to do with cancellation. A flaky
# exit gate is worse than a slow one — `make gates` is what this project trusts
# to say where it stands.
#
# So: wait for the renderer's own first progress line, then signal. If the
# render finishes before ever reporting progress, the gate says so instead of
# guessing, because at that point it is not testing cancellation at all.
gate_five() {
  local progress="$WORK/c.progress"
  : > "$progress"
  "$STILL" render-scene --image fixtures/generated/land.jpg \
    --audio fixtures/generated/vbr_lying_header.mp3 --out "$WORK/c.mp4" \
    >/dev/null 2>"$progress" &
  local pid=$!

  # Up to 10 s in 20 ms steps. Reached only if the render is pathologically
  # slow to start, which is itself worth failing on.
  local waited=0
  while [ "$waited" -lt 500 ]; do
    grep -q 'rendering: frame' "$progress" 2>/dev/null && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.02
    waited=$((waited+1))
  done

  if ! grep -q 'rendering: frame' "$progress" 2>/dev/null; then
    wait "$pid" 2>/dev/null
    echo "the render never reported progress, so cancellation was not exercised"
    echo "(use a longer narration fixture rather than relaxing this gate)"
    return 1
  fi

  kill -INT "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
  # D-045: interrupted is a failure, not a quiet success.
  [ $? -ne 0 ] || { echo "an interrupted render reported success"; return 1; }
  # plan.md: "absent, or present and marked partial — never a valid-looking stub".
  [ ! -f "$WORK/c.mp4" ] || { echo "the destination was written"; return 1; }
  # And nothing partial left littering the directory either.
  ! ls -a "$WORK" | grep -q partial
}
check "Ctrl-C leaves no valid-looking stub and no partial file" gate_five

# --- the process boundary rule ----------------------------------------------
check "argument vectors, never shell strings" \
  cargo test --release -p spoonstill-media --test no_shell_strings

# --- D-010 still holds ------------------------------------------------------
check "the D-010 boundary still holds" \
  cargo test --release -p spoonstill-cli --test architecture

# --- the diagnostics bundle -------------------------------------------------
gate_bundle() {
  "$STILL" diagnostics export --project "$WORK" --out "$WORK/bundle.txt" >/dev/null || return 1
  grep -q 'ENVIRONMENT'        "$WORK/bundle.txt" || return 1
  grep -q 'ffmpeg version'     "$WORK/bundle.txt" || return 1
  grep -q 'does NOT contain API keys' "$WORK/bundle.txt" || return 1
  # The whole point: the exact command that ran is in the file.
  grep -q 'zoompan=' "$WORK/bundle.txt"
}
check "still diagnostics export produces a sendable bundle" gate_bundle

echo
total=$((pass+fail))
if [ "$fail" -eq 0 ]; then
  printf '%sM1 COMPLETE%s — %d/%d gates pass\n' "$GREEN" "$OFF" "$pass" "$total"
else
  printf '%sM1 INCOMPLETE%s — %d/%d gates pass\n' "$RED" "$OFF" "$pass" "$total"
  printf '%sRun `make test` for detail.%s\n' "$DIM" "$OFF"
  exit 1
fi

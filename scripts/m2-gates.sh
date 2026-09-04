#!/usr/bin/env bash
# The M2 exit gates from plan.md, run as written there, plus the three that
# slice 3 added when rendering became parallel.
#
# Companion to m0-gates.sh and m1-gates.sh. Each gate is a command with an
# observable result; none of them paraphrases the promise into something
# easier to satisfy.
#
# Note on the two render fixtures: the main render gates run against
# `fixtures/projects/renderable/`, which needs no network, so the bulk of M2 is
# provable on a machine with no voice service. `fixtures/projects/mixed/` adds a
# spoken line and is gate 7's alone.
set -uo pipefail

cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

GREEN=$'\033[32m'; RED=$'\033[31m'; DIM=$'\033[2m'; OFF=$'\033[0m'
pass=0; fail=0
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

check() { # description, then a command
  local what="$1"; shift
  # A gate with no command is a gate that passes by doing nothing. This
  # happened: an insertion split a two-line `check … \` invocation and left the
  # function name orphaned on its own, so gate 4e reported PASS for three
  # sessions while running nothing at all — D-116's trap, in the harness rather
  # than in a test. Refused loudly, because the failure mode is silence.
  if [ "$#" -eq 0 ]; then
    printf '  %sFAIL%s  %s\n' "$RED" "$OFF" "$what"
    printf '          no command was given to `check` — a gate that runs nothing\n'
    fail=$((fail+1)); return
  fi
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
# The fixture generator uses the same bare name; both are dev-only scripts in a
# shell, not the app (D-103 is about a program launched from Finder).
FFMPEG=ffmpeg
FFPROBE=ffprobe
STATE=.spoonstill

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

# --- gate 4b: a re-recorded line invalidates its segment (D-107) ------------
# The other half of gate 4, and the one that was missing. A cache that never
# misses is not a cache, it is a stale film: replacing a recording with a
# different one of the *same duration* used to reuse the old segment, because
# the segment key held the frame count and not the narration's content. The
# operator got their previous voice-over in a film that reported success.
gate_reuse_invalidates() {
  local proj="$WORK/rerecord"
  mkdir -p "$proj"
  printf 'short_edge: 720\nfps: 30\n' > "$proj/project.yaml"
  "$FFMPEG" -y -loglevel error -f lavfi -i "color=c=blue:s=1600x1000" \
    -frames:v 1 "$proj/001.jpg" || return 1
  # Two recordings of exactly the same length and entirely different content.
  "$FFMPEG" -y -loglevel error -f lavfi -i "sine=frequency=440:duration=1" \
    -ar 48000 -ac 1 "$WORK/a440.wav" || return 1
  "$FFMPEG" -y -loglevel error -f lavfi -i "sine=frequency=880:duration=1" \
    -ar 48000 -ac 1 "$WORK/a880.wav" || return 1

  cp "$WORK/a440.wav" "$proj/001.wav"
  "$STILL" render "$proj" --out "$WORK/take1.mp4" >/dev/null 2>&1 || return 1

  local out
  cp "$WORK/a880.wav" "$proj/001.wav"
  out=$("$STILL" render "$proj" --out "$WORK/take2.mp4" 2>&1) || { echo "$out"; return 1; }
  grep -q '0 segments reused' <<<"$out" || {
    echo "a different narration reused the old segment:"; echo "$out"; return 1; }

  local a b
  a=$(shasum -a 256 "$WORK/take1.mp4" | cut -d' ' -f1)
  b=$(shasum -a 256 "$WORK/take2.mp4" | cut -d' ' -f1)
  [ "$a" != "$b" ] || { echo "two narrations produced one film: $a"; return 1; }

  # And the cache still works: the original recording back again must reuse.
  cp "$WORK/a440.wav" "$proj/001.wav"
  out=$("$STILL" render "$proj" --out "$WORK/take3.mp4" 2>&1) || { echo "$out"; return 1; }
  grep -q '1 segment reused' <<<"$out" || {
    echo "the original narration did not reuse its segment:"; echo "$out"; return 1; }
  local c
  c=$(shasum -a 256 "$WORK/take3.mp4" | cut -d' ' -f1)
  [ "$a" = "$c" ] || { echo "the same narration gave two films: $a then $c"; return 1; }
}
check "a narration replaced by a different one of the same length re-renders" \
  gate_reuse_invalidates

# --- gate 4c: identical narrations are made once, not once per worker -------
# D-108. Many scenes resolving to one cache entry is the ordinary case at the
# design point — one recording used throughout, one line repeated, a folder of
# silent stills — and every worker used to check an empty cache and do the
# whole job. Sixteen scenes sharing one narration at --audio-jobs 8 generated
# it eight times. Against a metered provider that is eight times the bill.
#
# Silent scenes are used deliberately: stills with no script and no recording
# all resolve to the same silence, so this provokes the collision with no
# network and no voice service at all.
gate_single_flight() {
  local proj="$WORK/oneflight"
  mkdir -p "$proj"
  local i
  for i in $(seq -w 1 16); do
    cp fixtures/generated/land.jpg "$proj/0$i.jpg" || return 1
  done

  local out
  out=$("$STILL" render "$proj" --out "$WORK/oneflight.mp4" --audio-jobs 8 2>&1) || {
    echo "$out"; return 1; }

  # Sixteen scenes, one unique narration: fifteen must report it as cached.
  # Anything less is that many workers that did the work in parallel.
  grep -q '15 narrations from cache' <<<"$out" || {
    echo "one narration was generated more than once:"; echo "$out"; return 1; }

  local n
  n=$(ls "$proj/.spoonstill/cache/audio" | wc -l | tr -d ' ')
  [ "$n" = "1" ] || { echo "expected 1 cache entry, found $n"; return 1; }
}
check "sixteen scenes sharing one narration generate it once" gate_single_flight

# --- gate 4d: the segment cache is bounded, and flipping back is free -------
# D-109. Nothing swept superseded segments, so a project accumulated one dead
# generation per render forever: the author's own ten-scene folder held 52
# segments, 134 MB, of which at most 10 could be live. At the design point that
# is gigabytes of files nothing will ever read again.
#
# The two halves have to hold together. Bounded is easy on its own (keep only
# what the film used); free-to-flip-back is easy on its own (keep everything).
# Keeping the live set plus two spare generations is what gives both, so the
# gate asserts both.
#
# The scenes are a still, a recording and a `.txt`: D-106 makes the text beside
# a recording the caption, so the theme changes the picture and every segment
# with it — and none of it needs a voice service.
gate_bounded_cache() {
  local proj="$WORK/bounded"
  local seg="$proj/$STATE/segments"
  mkdir -p "$proj"
  local i
  for i in 1 2 3 4; do
    cp fixtures/generated/land.jpg "$proj/00$i.jpg" || return 1
    "$FFMPEG" -y -loglevel error -f lavfi -i "sine=frequency=440:duration=4" \
      -ar 48000 -ac 1 "$proj/00$i.wav" || return 1
    echo "A caption for scene $i, long enough to wrap onto two lines." \
      > "$proj/00$i.txt"
  done

  # Five different films from one project: each theme supersedes the last.
  local t
  for t in classic boxed band card punch; do
    "$STILL" render "$proj" --out "$WORK/bounded.mp4" --subtitles "$t" \
      >/dev/null 2>&1 || return 1
  done

  # Four scenes, so three generations is twelve files. Not five generations.
  local n
  n=$(ls "$seg" | wc -l | tr -d " ")
  [ "$n" -le 12 ] || { echo "cache grew to $n files, bound is 12"; return 1; }

  # And the point of keeping spares: the previous two themes still render for
  # free. A sweep that kept only the live set would re-encode every scene.
  local out
  out=$("$STILL" render "$proj" --out "$WORK/bounded.mp4" --subtitles card 2>&1) || {
    echo "$out"; return 1; }
  grep -q "4 segments reused" <<<"$out" || {
    echo "flipping back to a recent theme re-encoded:"; echo "$out"; return 1; }

  # --keep-cache is the operator's override, and it must sweep nothing.
  out=$("$STILL" render "$proj" --out "$WORK/bounded.mp4" --subtitles minimal \
    --keep-cache 2>&1) || { echo "$out"; return 1; }
  grep -q "swept" <<<"$out" && { echo "--keep-cache swept anyway"; return 1; }

  # A file we did not write is not ours to delete.
  echo "not ours" > "$seg/holiday.mp4"
  "$STILL" render "$proj" --out "$WORK/bounded.mp4" --no-subtitles \
    >/dev/null 2>&1 || return 1
  [ -f "$seg/holiday.mp4" ] || { echo "the sweep deleted a stranger's file"; return 1; }
}
check "the segment cache is bounded, and the last two generations stay free" \
  gate_bounded_cache

# A stand-in for the voice service, for the two gates that need speech.
#
# Not `edge-tts` itself, for two reasons. It is not on the CI runner, where M2
# gate 7 deliberately tests the *other* half of D-020 — and a gate that depends
# on somebody else's service is a gate that fails on their afternoon (D-134's
# rule). And a fixed delay with a fixed answer is what makes both gates
# repeatable: a network is waiting, not computing.
stub_voice_service() { # delay-seconds
  "$FFMPEG" -y -loglevel error -f lavfi -i "sine=frequency=440:duration=2" \
    -ar 24000 -ac 1 -b:a 48k "$WORK/stub.mp3" || return 1
  cat > "$WORK/stub-edge-tts" <<STUB
#!/usr/bin/env bash
for a in "\$@"; do
  if [ "\$a" = "--version" ]; then echo "edge-tts (stand-in)"; exit 0; fi
done
out=""; prev=""
for a in "\$@"; do
  if [ "\$prev" = "--write-media" ]; then out="\$a"; fi
  prev="\$a"
done
sleep $1
cp "\$STUB_MP3" "\$out"
STUB
  chmod +x "$WORK/stub-edge-tts"
  export SPOONSTILL_EDGE_TTS="$WORK/stub-edge-tts" STUB_MP3="$WORK/stub.mp3"
}

# --- gate 4e: a cached segment of the wrong length is not reused ------------
# D-110. The reuse check asserted the segment *profile*, which pins codec,
# geometry and colour and says nothing about length — so a file with a
# segment's name and a segment's shape was reused whatever its duration, and
# the frame count printed for it was the planned one, which nothing had checked.
#
# The film's own assertion did catch the short film, so no wrong film ever
# reached an operator. What it did instead was worse to live with: the bad
# entry stayed in the cache, so **every subsequent render failed the same way**,
# blaming a temporary file, naming no scene, and offering no way out but
# deleting a hidden folder. This asserts the recovery, not just the refusal.
gate_reuse_checks_length() {
  local proj="$WORK/wronglen"
  local seg="$proj/$STATE/segments"
  mkdir -p "$proj"
  cp fixtures/generated/land.jpg "$proj/001.jpg" || return 1
  cp fixtures/generated/land.jpg "$proj/002.jpg" || return 1
  "$FFMPEG" -y -loglevel error -f lavfi -i "sine=frequency=440:duration=2" \
    -ar 48000 -ac 1 "$proj/001.wav" || return 1
  "$FFMPEG" -y -loglevel error -f lavfi -i "sine=frequency=440:duration=8" \
    -ar 48000 -ac 1 "$proj/002.wav" || return 1

  "$STILL" render "$proj" --out "$WORK/wronglen.mp4" >/dev/null 2>&1 || return 1

  # Put the short scene's segment under the long scene's cache name. Same
  # profile in every field the assertion checks; four times the wrong length.
  #
  # Found **by length, not by filename**. The names used to carry the scene
  # index and this globbed `seg-0000-*`; D-153 took the index out, because a
  # file named after where a scene sits cannot be reused when the scene moves.
  # Asking each segment how long it is needs no name at all, which is also why
  # it survives the next rename.
  local short long frames
  for candidate in "$seg"/seg-*.mp4; do
    [ -f "$candidate" ] || continue
    frames=$("$FFPROBE" -v error -select_streams v:0 \
      -show_entries stream=nb_frames -of csv=p=0 "$candidate")
    case "$frames" in
      60)  short="$candidate" ;;
      240) long="$candidate" ;;
    esac
  done
  [ -n "$short" ] && [ -n "$long" ] || {
    echo "no 60-frame and 240-frame segments to swap:"; ls -l "$seg"; return 1; }
  cp "$short" "$long" || return 1

  # It must notice, re-render that scene, and produce the right film.
  local out
  out=$("$STILL" render "$proj" --out "$WORK/wronglen2.mp4" 2>&1) || {
    echo "a wrong-length cache entry was not recovered from:"; echo "$out"; return 1; }
  grep -q "1 segment reused" <<<"$out" || {
    echo "expected exactly the good segment to be reused:"; echo "$out"; return 1; }

  # 60 + 240 frames, actually decoded.
  local n
  n=$("$FFPROBE" -v error -select_streams v:0 -count_frames \
    -show_entries stream=nb_read_frames -of csv=p=0 "$WORK/wronglen2.mp4")
  [ "$n" = "300" ] || { echo "film has $n frames, expected 300"; return 1; }
}
check "a cached segment of the wrong length is re-rendered, not reused" \
  gate_reuse_checks_length

# --- gate 4f: the audio cache is bounded, and a network call is never swept --
# D-147. The author's own folders held 861 MB of normalized WAV protecting
# 26.5 MB of spoken MP3 — 32:1, on a network volume — because D-109 spared the
# audio cache entirely. It was right about the *spoken* layer, which costs a
# network call and under D-014 money, and wrong about the normalized one, which
# is one local FFmpeg pass away from the MP3 still sitting beside it.
#
# Five voices over four scenes is five generations of both layers. The derived
# half must end at three generations; the spoken half must end at five.
gate_bounded_audio() {
  local proj="$WORK/bounded-audio"
  local cache="$proj/$STATE/cache/audio"
  mkdir -p "$proj"
  printf 'output: film.mp4\naspect: 16:9\nshort_edge: 540\nfps: 30\n' > "$proj/project.yaml"
  local i
  for i in 1 2 3 4; do
    cp fixtures/generated/land.jpg "$proj/00$i.jpg" || return 1
    printf 'Scene %s speaks a line.\n' "$i" > "$proj/00$i.txt"
  done

  stub_voice_service 0 || return 1

  local v
  for v in en-US-AvaNeural en-GB-RyanNeural en-AU-NatashaNeural en-IE-EmilyNeural \
           en-CA-LiamNeural; do
    "$STILL" render "$proj" --out "$WORK/ba.mp4" --voice "$v" >/dev/null 2>&1 || return 1
  done

  # Four scenes, so three generations is twelve derived files. Not twenty.
  local derived spoken
  derived=$(ls "$cache" | grep -c '\.wav$')
  [ "$derived" -le 12 ] || {
    echo "the derived cache grew to $derived files, bound is 12"; return 1; }

  # And every one of the twenty network calls is still on disk. This is the
  # half of D-109 that was right, and it must stay right.
  spoken=$(ls "$cache" | grep -c '\.spoken$')
  [ "$spoken" -eq 20 ] || {
    echo "$spoken spoken files of 20 — the sweep took something that cost a call"
    return 1; }

  # Flipping back to a recent voice costs neither a call nor a re-normalize.
  local out
  out=$("$STILL" render "$proj" --out "$WORK/ba.mp4" --voice en-IE-EmilyNeural 2>&1) || {
    echo "$out"; return 1; }
  grep -q "4 narrations from cache" <<<"$out" || {
    echo "flipping back re-resolved the narrations:"; echo "$out"; return 1; }

  # A file we did not write is not ours to delete, in this cache either.
  echo "not ours" > "$cache/holiday.wav"
  "$STILL" render "$proj" --out "$WORK/ba.mp4" --voice en-US-GuyNeural \
    >/dev/null 2>&1 || return 1
  [ -f "$cache/holiday.wav" ] || { echo "the sweep deleted a stranger's file"; return 1; }

  # --keep-cache is the operator's override here too.
  out=$("$STILL" render "$proj" --out "$WORK/ba.mp4" --voice en-US-JennyNeural \
    --keep-cache 2>&1) || { echo "$out"; return 1; }
  grep -q "swept" <<<"$out" && { echo "--keep-cache swept anyway"; return 1; }

  unset SPOONSTILL_EDGE_TTS STUB_MP3
  return 0
}
check "the audio cache is bounded, and nothing that cost a call is swept" gate_bounded_audio

# --- gate 4g: a reorder costs nothing, and an old project is untouched ------
# D-153. Moving one scene used to re-encode the whole film and change the Ken
# Burns move on every scene the operator did not touch (D-140), because the
# scene's *index* was in the motion seed and in the segment's filename.
#
# Both halves are asserted, and the second is the one that made this possible
# to do at all: a project with no `motion_seed:` key is every folder made
# before the key existed, and it must keep behaving exactly as it did.
gate_reorder() {
  local media="$WORK/reorder-media"
  mkdir -p "$media"
  local i
  for i in 1 2 3 4 5 6; do
    "$FFMPEG" -y -loglevel error -f lavfi -i "testsrc2=size=640x360:rate=1" \
      -frames:v 1 -q:v "$i" "$media/p$i.jpg" || return 1
    "$FFMPEG" -y -loglevel error -f lavfi -i "sine=frequency=$((300 + i * 40)):duration=1" \
      -ar 48000 -ac 1 "$media/p$i.wav" || return 1
  done

  # `project_id` is the folder *basename* and it seeds the move (D-035), so the
  # two projects must differ only in their settings — not in their name.
  local new="$WORK/reorder/new/chapter" old="$WORK/reorder/old/chapter"
  rm -rf "$WORK/reorder"; mkdir -p "$new" "$old"
  for i in 1 2 3 4 5 6; do
    cp "$media/p$i.jpg" "$new/00$i.jpg"; cp "$media/p$i.wav" "$new/00$i.wav"
    cp "$media/p$i.jpg" "$old/00$i.jpg"; cp "$media/p$i.wav" "$old/00$i.wav"
  done
  printf 'output: film.mp4\nshort_edge: 360\nmotion_seed: v2\n' > "$new/project.yaml"
  printf 'output: film.mp4\nshort_edge: 360\n'                  > "$old/project.yaml"

  # The new rule: move a scene, and nothing re-encodes.
  "$STILL" render "$new" --out "$WORK/r-new.mp4" --jobs 2 >/dev/null 2>&1 || return 1
  "$STILL" move "$new" 006 1 >/dev/null 2>&1 || return 1
  local out
  out=$("$STILL" render "$new" --out "$WORK/r-new2.mp4" --jobs 2 2>&1) || { echo "$out"; return 1; }
  grep -q "6 segments reused" <<<"$out" || {
    echo "$out"; echo "a reorder re-encoded the film under motion_seed: v2"; return 1; }

  # Six scenes still, in the new order: a reuse that dropped one would pass the
  # line above and produce a shorter film.
  grep -q "6 scenes, 180 frames" <<<"$out" || {
    echo "$out"; echo "the film is not six one-second scenes"; return 1; }

  # The old rule, deliberately unchanged: a project that says nothing about its
  # seed keeps re-rendering, because the alternative is changing a film that
  # has already been made.
  "$STILL" render "$old" --out "$WORK/r-old.mp4" --jobs 2 >/dev/null 2>&1 || return 1
  "$STILL" move "$old" 006 1 >/dev/null 2>&1 || return 1
  out=$("$STILL" render "$old" --out "$WORK/r-old2.mp4" --jobs 2 2>&1) || { echo "$out"; return 1; }
  grep -q "0 segments reused" <<<"$out" || {
    echo "$out"
    echo "a project with no motion_seed: key changed behaviour — an existing film would move"
    return 1; }

  # And `still new` says which rule it made the project under, so a folder is
  # not silently on the old one forever.
  rm -rf "$WORK/reorder-fresh"
  "$STILL" new "$WORK/reorder-fresh" "$media/p1.jpg" >/dev/null 2>&1 || return 1
  grep -q '^motion_seed: v2$' "$WORK/reorder-fresh/project.yaml" || {
    echo "a new project does not declare its motion seed"; return 1; }

  return 0
}
check "moving a scene re-encodes nothing, and an older project is unchanged" gate_reorder

# --- gate 5: two renders of one project do not interleave -------------------
# D-113. The lock is the operating system's, taken with `File::try_lock`, so
# this gate holds a **real** one: a render running in the background. Writing
# `pid 999999` into the file — what this gate used to do — no longer refuses
# anything, and requiring it to was requiring the tool to stay stuck after a
# run the machine lost.
#
# Two properties, and the second is the fix: a live lock refuses a second
# render, and `--force` does not override it either.
gate_lock() {
  local proj="$WORK/locked"
  mkdir -p "$proj"
  local i
  for i in $(seq -w 1 8); do
    cp fixtures/generated/land.jpg "$proj/0$i.jpg" || return 1
  done

  # A real render, long enough to still be going when we ask.
  "$STILL" render "$proj" --out "$WORK/held.mp4" --jobs 1 >"$WORK/held.log" 2>&1 &
  local holder=$!

  # Wait for it to actually hold the lock rather than sleeping and hoping.
  local waited=0
  while [ ! -s "$proj/$STATE/render.lock" ] && [ "$waited" -lt 100 ]; do
    waited=$((waited + 1))
    perl -e 'select(undef,undef,undef,0.1)'
  done
  kill -0 "$holder" 2>/dev/null || { echo "the holding render exited early"; return 1; }

  local out status
  out=$("$STILL" render "$proj" --out "$WORK/locked.mp4" 2>&1); status=$?
  [ "$status" -ne 0 ] || { kill "$holder" 2>/dev/null; echo "a locked project rendered anyway"; return 1; }
  grep -q 'another render is working on this project' <<<"$out" || {
    kill "$holder" 2>/dev/null; echo "$out"; return 1; }

  # The fix: --force must not take a lock a live render holds.
  out=$("$STILL" render "$proj" --out "$WORK/forced.mp4" --force 2>&1); status=$?
  kill -0 "$holder" 2>/dev/null || { echo "the holding render exited before --force was tried"; return 1; }
  [ "$status" -ne 0 ] || {
    kill "$holder" 2>/dev/null
    echo "--force took a lock a running render was holding"; return 1; }
  grep -q -- '--force cannot' <<<"$out" || { kill "$holder" 2>/dev/null; echo "$out"; return 1; }

  wait "$holder" || { echo "the holding render failed"; return 1; }

  # And once it is over, the next render is clean — with the file still there,
  # because the file's existence was never the lock.
  [ -f "$proj/$STATE/render.lock" ] || { echo "the marker file should remain"; return 1; }
  "$STILL" render "$proj" --out "$WORK/after.mp4" >/dev/null 2>&1 || {
    echo "a released lock still refused the next render"; return 1; }
}
check "a live render refuses a second, and --force does not override it" gate_lock

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
  # `seg-<16 hex>.mp4` since D-153 — the scene index left the name, because a
  # file named after where a scene sits cannot be reused when the scene moves.
  ls "$RENDERABLE/.spoonstill/segments" | grep -qE '^seg-[0-9a-f]{16}\.mp4$' || {
    ls "$RENDERABLE/.spoonstill/segments"; return 1; }
  # No operator spelling anywhere in the segment directory. This is what the
  # gate is really for: the concat list is the one text format in the codebase,
  # and it stays ASCII because the names are content-addressed rather than
  # built from whatever the operator called their photograph (D-052).
  ! ls "$RENDERABLE/.spoonstill/segments" | grep -qv -E '^seg-[0-9a-f]{16}\.mp4$'
}
check "odd dimensions and a Unicode filename survive the join" gate_hostile

# --- gate 7: the three sources render as one film ---------------------------
# `mixed` is a photo with a script, a photo with a recording, and a photo with
# neither. D-020's rule is that nothing downstream branches on which is which,
# and this is where that stops being an assertion about the code and becomes
# one about a file.
#
# The spoken scene needs a provider. When `edge-tts` is not installed the gate
# checks the other half of D-020 instead: a line somebody wrote must never
# quietly become silence, so the render must fail and name the missing tool.
gate_tts() {
  local out status
  rm -rf fixtures/projects/mixed/.spoonstill
  out=$("$STILL" render fixtures/projects/mixed/ --out "$WORK/mixed.mp4" 2>&1)
  status=$?
  rm -rf fixtures/projects/mixed/.spoonstill

  if ! command -v edge-tts >/dev/null 2>&1; then
    [ "$status" -ne 0 ] || { echo "a spoken scene rendered with no provider"; return 1; }
    grep -qi 'edge-tts' <<<"$out" || { echo "$out"; return 1; }
    [ ! -f "$WORK/mixed.mp4" ] || { echo "a film was written anyway"; return 1; }
    echo "    (edge-tts absent: checked the refusal, not the render)"
    return 0
  fi

  [ "$status" -eq 0 ] || { echo "$out"; return 1; }
  grep -q '3 scenes' <<<"$out" || { echo "$out"; return 1; }
  [ -f "$WORK/mixed.mp4" ] || { echo "no film was written"; return 1; }
  # The spoken line is cached as the provider returned it *and* normalized, so
  # a re-render never speaks it again (D-081).
  ls fixtures/projects/mixed/.spoonstill/cache/audio 2>/dev/null | grep -q '^tts-' || true
}
check "a script, a recording and a silent still become one film" gate_tts

# --- gate 7b: 2K, 4K and a vertical Short all come out at the size asked for -
# D-143. Geometry was reachable from `project.yaml` and from `render-scene`,
# which renders one segment — so the one command that makes a film could not
# be told what shape or size to make it. This asserts the whole path: the flag
# reaches `OutputSpec`, the filter chain, the profile assertion and the join.
#
# The vertical case is the one worth having: a 4K Short is 2160x3840, not
# 3840x2160, and a chooser that got that backwards would produce a film nobody
# can post. The durations are asserted equal across all four because geometry
# must change the pixels and nothing else — the narration decides the length
# (D-021), and a resize that moved it would mean the frame count is being
# derived from the wrong thing.
gate_sizes() {
  local proj="$WORK/sizes"
  mkdir -p "$proj"
  cp fixtures/generated/land.jpg "$proj/001.jpg" || return 1
  "$FFMPEG" -y -loglevel error -f lavfi -i "sine=frequency=440:duration=2" \
    -ar 48000 -ac 1 "$proj/001.wav" || return 1

  local spec want base
  base=""
  for spec in "16:9 1080p 1920x1080" "16:9 2k 2560x1440" "16:9 4k 3840x2160" \
              "shorts 1080p 1080x1920" "tiktok 4k 2160x3840" "1:1 1440p 1440x1440"; do
    set -- $spec
    local aspect="$1" size="$2" expected="$3"
    "$STILL" render "$proj" --out "$WORK/s.mp4" --aspect "$aspect" \
      --resolution "$size" >/dev/null 2>&1 || {
        echo "--aspect $aspect --resolution $size failed to render"; return 1; }
    local got
    got=$("$FFPROBE" -v error -select_streams v:0 \
      -show_entries stream=width,height -of csv=p=0 "$WORK/s.mp4" | tr ',' 'x')
    [ "$got" = "$expected" ] || {
      echo "--aspect $aspect --resolution $size gave $got, wanted $expected"; return 1; }

    # The length is the narration's, whatever the frame is.
    local seconds
    seconds=$("$FFPROBE" -v error -show_entries format=duration -of csv=p=0 "$WORK/s.mp4")
    if [ -z "$base" ]; then base="$seconds"; fi
    [ "$seconds" = "$base" ] || {
      echo "$aspect $size ran $seconds s against $base s — geometry moved the clock"
      return 1; }
  done

  # 8K is refused rather than mislabelled: past 36864 macroblocks the segment
  # profile would declare an H.264 level no decoder honours (D-114).
  "$STILL" render "$proj" --out "$WORK/s.mp4" --short-edge 4320 >/dev/null 2>&1 \
    && { echo "8K rendered"; return 1; }

  # Two spellings of one setting are refused together rather than one winning.
  "$STILL" render "$proj" --out "$WORK/s.mp4" --resolution 4k --short-edge 1080 \
    >/dev/null 2>&1 && { echo "--resolution and --short-edge both accepted"; return 1; }

  rm -rf "$proj/$STATE"
  return 0
}
check "2K, 4K and a vertical Short each come out at the size asked for" gate_sizes

# --- gate 7c: the pool is sized against the frame, not just the cores -------
# D-144. Four 4K workers froze an 8 GB Windows machine hard enough to need the
# power button, because the pool had only ever been sized from the core count
# and D-076's whole table is 1080p.
#
# The budget is stated rather than measured, so this asserts the rule and not
# whatever RAM the runner happens to have. The *core* count cannot be stated,
# though, and that is what the first version of this gate got wrong: a small CI
# runner derives one worker for every size, so comparing the two automatic
# counts compared 1 against 1 and failed on a fix that was working. What is
# asserted instead is core-independent — the same explicit job count that fits
# at 1080p does not fit at 4K, which is the defect itself.
gate_capacity() {
  local proj="$WORK/capacity"
  mkdir -p "$proj"
  cp fixtures/generated/land.jpg "$proj/001.jpg" || return 1
  "$FFMPEG" -y -loglevel error -f lavfi -i "sine=frequency=440:duration=1" \
    -ar 48000 -ac 1 "$proj/001.wav" || return 1

  render_at() {  # budget_mb resolution [extra args...]
    local budget="$1" size="$2"; shift 2
    SPOONSTILL_MEMORY_BUDGET_MB="$budget" "$STILL" render "$proj" \
      --out "$WORK/c.mp4" --resolution "$size" "$@" 2>&1
  }

  # An 8 GB machine, four workers asked for explicitly. 1080p fits and must say
  # nothing new — a rule that slowed every render down would be a worse defect
  # than the one it fixes.
  local hd
  hd=$(render_at 5600 1080p --jobs 4) || { echo "1080p --jobs 4 failed"; return 1; }
  printf '%s' "$hd" | grep -q "4 at a time" || {
    echo "1080p --jobs 4 did not run four at a time"; return 1; }
  printf '%s' "$hd" | grep -q "warning:" && {
    echo "1080p at 8 GB warned; four 1080p workers fit and must not"; return 1; }

  # The same request at 4K does not fit, and that difference is the whole fix.
  local uhd
  uhd=$(render_at 5600 4k --jobs 4) || { echo "4k --jobs 4 failed"; return 1; }
  printf '%s' "$uhd" | grep -q "warning:" || {
    echo "four 4K workers on an 8 GB machine did not warn — the pool is frame-blind"
    return 1; }
  # Obeyed, not overruled: D-076 says --jobs is not capped in either direction.
  printf '%s' "$uhd" | grep -q "4 at a time" || {
    echo "--jobs 4 was overruled; D-076 says it is obeyed"; return 1; }
  printf '%s' "$uhd" | grep -q "try --jobs 2" || {
    echo "the warning named no smaller number to use"; return 1; }

  # The automatic count never rises with the frame. Weak on a runner whose core
  # rule already says one — which is why it is not the only thing asserted.
  jobs_of() { printf '%s' "$1" | sed -n 's/.*scenes\{0,1\}, \([0-9]*\) at a time.*/\1/p' | head -1; }
  local auto_hd auto_uhd
  auto_hd=$(jobs_of "$(render_at 5600 1080p)")
  auto_uhd=$(jobs_of "$(render_at 5600 4k)")
  [ -n "$auto_hd" ] && [ -n "$auto_uhd" ] || { echo "no worker count reported"; return 1; }
  [ "$auto_uhd" -le "$auto_hd" ] || {
    echo "4K chose $auto_uhd workers against 1080p's $auto_hd"; return 1; }

  # A machine too small for even one 4K worker still renders, rather than
  # refusing: the operator may have closed everything else.
  local tiny
  tiny=$(jobs_of "$(render_at 256 4k)")
  [ "$tiny" = "1" ] || { echo "a tiny machine got '$tiny' workers, wanted 1"; return 1; }

  rm -rf "$proj/$STATE"
  return 0
}
check "four workers fit at 1080p and are warned about at 4K" gate_capacity

# --- gate 7d: a photograph smaller than the frame is said out loud ----------
# D-145. 699 of the author's 741 real scenes are 1376x768 stills going into a
# 1920x1080 frame, and `still validate` reported "no problems" for every one of
# them. Nothing caught it because every fixture in this repo is *larger* than
# the frame — the only stills ever tested at size are the ones the author does
# not use, which is the blind spot this gate exists to close.
#
# The round trip is the assertion: the warning names a `--short-edge`, and
# rendering at that number must produce no warning. A message that names a size
# which does not fix the thing it warns about is worse than no message.
gate_undersized() {
  local proj="$WORK/undersized"
  mkdir -p "$proj"
  # The author's real geometry, twice, plus one still that does cover the frame
  # so the count in the message has to be a count rather than "all of them".
  "$FFMPEG" -y -loglevel error -f lavfi -i "testsrc2=size=1376x768:rate=1" \
    -frames:v 1 "$proj/001.jpg" || return 1
  cp "$proj/001.jpg" "$proj/002.jpg" || return 1
  cp fixtures/generated/land.jpg "$proj/003.jpg" || return 1

  local out
  out=$("$STILL" validate "$proj") || { echo "$out"; return 1; }
  grep -q '2 of 3 stills are smaller than the 1920x1080 frame' <<<"$out" || {
    echo "$out"; echo "no undersized warning at 1080p"; return 1; }
  grep -q 'the smallest is scene 001 at 1376x768' <<<"$out" || {
    echo "$out"; echo "the warning did not name the smallest still"; return 1; }
  grep -q 'nothing here stops a render' <<<"$out" || {
    echo "$out"; echo "an undersized still must be a warning, not an error"; return 1; }

  # The number it offers, read out of the message rather than written here, so
  # the gate cannot agree with the code by construction.
  local edge
  edge=$(sed -n 's/.*--short-edge \([0-9]*\).*/\1/p' <<<"$out" | head -1)
  [ -n "$edge" ] || { echo "$out"; echo "the warning suggested no size"; return 1; }

  printf 'short_edge: %s\n' "$edge" > "$proj/project.yaml"
  out=$("$STILL" validate "$proj") || { echo "$out"; return 1; }
  grep -q 'no problems' <<<"$out" || {
    echo "$out"; echo "--short-edge $edge was suggested and does not silence it"; return 1; }

  # And `still render` prints warnings at all, which it never did before D-145.
  rm -f "$proj/project.yaml"
  "$STILL" render "$proj" --out "$WORK/undersized.mp4" --jobs 2 >"$WORK/u.log" 2>&1 || {
    cat "$WORK/u.log"; return 1; }
  grep -q 'warning:.*smaller than the 1920x1080 frame' "$WORK/u.log" || {
    cat "$WORK/u.log"; echo "still render discarded the warning list"; return 1; }
  # Said before the pool, because a line under five minutes of progress output
  # is a line nobody reads (D-144's rule, applied to this warning).
  local warn_line first_scene
  warn_line=$(grep -n 'warning:' "$WORK/u.log" | head -1 | cut -d: -f1)
  first_scene=$(grep -n 'done\]' "$WORK/u.log" | head -1 | cut -d: -f1)
  [ -n "$first_scene" ] && [ "$warn_line" -lt "$first_scene" ] || {
    cat "$WORK/u.log"; echo "the warning arrived after the pool started"; return 1; }

  rm -rf "$proj/$STATE"
  return 0
}
check "an undersized still warns, names a size, and that size silences it" gate_undersized

# --- gate 7e: narration and rendering overlap -------------------------------
# D-146. In 4531 rows of the author's production log the two phases never once
# overlapped: 78 s of a 320 s render had ten cores idle while `edge-tts` waited
# on the network, and then the network was idle for the 228 s of encoding.
#
# The voice service is stood in for by a script that sleeps and then answers,
# because that is what a network is from this program's point of view — waiting,
# not computing — and because a gate that depends on somebody else's service is
# a gate that fails on their afternoon (D-134's rule).
#
# What is asserted is an *ordering of events*, never a wall-clock number: a
# shared runner is slow for reasons that are not defects.
gate_overlap() {
  local proj="$WORK/overlap"
  mkdir -p "$proj"
  printf 'output: film.mp4\naspect: 16:9\nshort_edge: 540\nfps: 30\n' > "$proj/project.yaml"
  local i
  for i in 1 2 3 4 5 6; do
    cp fixtures/generated/land.jpg "$proj/00$i.jpg" || return 1
    printf 'Scene number %s, spoken aloud.\n' "$i" > "$proj/00$i.txt"
  done

  stub_voice_service 1 || return 1

  "$STILL" render "$proj" --out "$WORK/overlap.mp4" --jobs 2 --audio-jobs 1 \
    >"$WORK/overlap.log" 2>&1 || { cat "$WORK/overlap.log"; return 1; }

  # The barrier, stated as a line number: with two phases, every narration is
  # reported before the first segment. Overlapped, at least one narration is
  # reported after it.
  local first_segment last_audio
  first_segment=$(grep -n 'done\]' "$WORK/overlap.log" | head -1 | cut -d: -f1)
  last_audio=$(grep -n '^  audio ' "$WORK/overlap.log" | tail -1 | cut -d: -f1)
  [ -n "$first_segment" ] && [ -n "$last_audio" ] || {
    cat "$WORK/overlap.log"; echo "no audio or no segment events"; return 1; }
  [ "$last_audio" -gt "$first_segment" ] || {
    cat "$WORK/overlap.log"
    echo "every narration finished before the first segment — the phases did not overlap"
    return 1; }

  # Six scenes went in and six came out: an overlap that dropped one would
  # otherwise pass the ordering check above.
  grep -q '6 scenes, 360 frames' "$WORK/overlap.log" || {
    cat "$WORK/overlap.log"; echo "the film is not six two-second scenes"; return 1; }

  # And the overlap changes the timing and nothing else (D-077). A separate
  # copy, so this is a cold render rather than a cache hit, at a different pair
  # of job counts.
  #
  # The copy keeps the **same folder name** in a different parent: `project_id`
  # is the basename and it seeds the Ken Burns move (D-035), so a twin called
  # `overlap-twin` renders different motion on purpose and this comparison
  # would fail for a reason that is not a defect. Found by writing it wrong.
  local twin="$WORK/twin/overlap"
  rm -rf "$WORK/twin"; mkdir -p "$WORK/twin"; cp -R "$proj" "$twin"
  rm -rf "$twin/$STATE"
  "$STILL" render "$twin" --out "$WORK/overlap-twin.mp4" --jobs 1 --audio-jobs 4 \
    >"$WORK/overlap-twin.log" 2>&1 || { cat "$WORK/overlap-twin.log"; return 1; }
  cmp -s "$WORK/overlap.mp4" "$WORK/overlap-twin.mp4" || {
    echo "two pipelined runs at different job counts produced different films"; return 1; }

  unset SPOONSTILL_EDGE_TTS STUB_MP3
  return 0
}
check "narration and rendering overlap, and the film is unchanged by it" gate_overlap

# --- gate 7f: every command is written down, not just renders ---------------
# D-148. The machine-wide `runs.csv` was built *inside* `render_project`, so for
# five months it held renders and nothing else: `validate`, `new`, `add`,
# `voices`, `doctor`, `remove`, `move` and the entire window wrote to it not at
# all. The failure the author reported on 2026-08-31 (D-141) is absent from a
# file that has 1508 rows for that day.
#
# `HOME` is redirected so this writes into `$WORK` and not into the operator's
# own activity log — the index lives under `~/Library/Application Support` on
# this platform, which is also why this gate is macOS-only like the rest.
gate_journal() {
  local home="$WORK/journal-home" proj="$WORK/journal-project"
  local csv="$home/Library/Application Support/spoonstill/runs.csv"
  rm -rf "$home" "$proj"; mkdir -p "$home" "$proj"
  printf 'output: film.mp4\naspect: 16:9\nshort_edge: 540\nfps: 30\n' > "$proj/project.yaml"
  cp fixtures/generated/land.jpg "$proj/001.jpg" || return 1

  HOME="$home" "$STILL" validate "$proj" >/dev/null 2>&1 || return 1
  HOME="$home" "$STILL" subtitles >/dev/null 2>&1 || return 1
  # A failure, which is the whole reason the file exists.
  HOME="$home" "$STILL" validate "$WORK/no-such-folder" >/dev/null 2>&1 && {
    echo "validate on a missing folder reported success"; return 1; }

  [ -f "$csv" ] || { echo "nothing was written to runs.csv at all"; return 1; }

  # A project command, a machine-level command, and a failure carrying the
  # sentence the operator saw.
  grep -q '","validate","journal-project","command finished"' "$csv" || {
    echo "validate is not in the log:"; cat "$csv"; return 1; }
  grep -q '","subtitles","","command finished"' "$csv" || {
    echo "a machine-level command is not in the log:"; cat "$csv"; return 1; }
  grep -q '"error","validate",.*"command failed"' "$csv" || {
    echo "a failed command is not in the log:"; cat "$csv"; return 1; }

  # Asking about a folder is not adopting it.
  [ -e "$WORK/no-such-folder" ] && { echo "validate created the folder it refused"; return 1; }

  # And a render still writes what D-093 asked for — the wrapper's row *and*
  # the detailed events, in both files.
  HOME="$home" "$STILL" render "$proj" --out "$WORK/journal.mp4" >/dev/null 2>&1 || return 1
  grep -q '","render","journal-project","command finished"' "$csv" || {
    echo "the render command itself is not in the log"; return 1; }
  grep -q '"film complete"' "$csv" || {
    echo "the render's own events stopped reaching runs.csv"; return 1; }
  grep -q '"film complete"' "$proj/$STATE/logs/"*.jsonl || {
    echo "the render's own events stopped reaching the project log"; return 1; }

  return 0
}
check "every command reaches the activity log, not only renders" gate_journal

# --- gate 7g: a render never writes to project.yaml -------------------------
# D-013, and plan.md §M3 asks for exactly this: *"`project.yaml` is never
# written to. A test asserts its mtime and hash are unchanged after a full
# render."* `settings.rs`'s own module note has promised this gate since M2 and
# it did not exist.
#
# It matters more since D-153, not less: `still new` now writes a starter
# `project.yaml`, so "this program does not write that file" became "this
# program writes it exactly once, when it creates the folder, and never again".
# The second half is the one an operator's edits depend on.
gate_settings_untouched() {
  local proj="$WORK/untouched"
  rm -rf "$proj"; mkdir -p "$proj"
  cp fixtures/generated/land.jpg "$proj/001.jpg" || return 1
  cp fixtures/generated/land.jpg "$proj/002.jpg" || return 1
  "$FFMPEG" -y -loglevel error -f lavfi -i "sine=frequency=440:duration=1" \
    -ar 48000 -ac 1 "$proj/001.wav" || return 1

  # Written by hand, with a comment and an ordering no writer of ours would
  # reproduce — so a rewrite is visible even if it happened to keep the values.
  cat > "$proj/project.yaml" <<'YAML'
# the operator's own file, in the operator's own order
short_edge: 360
output: film.mp4
fps: 30
YAML

  local before_hash before_time
  before_hash=$(shasum -a 256 "$proj/project.yaml" | cut -d' ' -f1)
  before_time=$(stat -f %m "$proj/project.yaml")

  # Everything that touches a project: validate, add media, render, render
  # again from cache, and rearrange the scenes.
  "$STILL" validate "$proj" >/dev/null 2>&1 || return 1
  "$STILL" add "$proj" fixtures/generated/square.jpg >/dev/null 2>&1 || return 1
  "$STILL" render "$proj" --out "$WORK/untouched.mp4" --jobs 2 >/dev/null 2>&1 || return 1
  "$STILL" render "$proj" --out "$WORK/untouched.mp4" --jobs 2 >/dev/null 2>&1 || return 1
  "$STILL" move "$proj" 002 1 >/dev/null 2>&1 || return 1
  "$STILL" remove "$proj" 003 >/dev/null 2>&1 || return 1

  local after_hash after_time
  after_hash=$(shasum -a 256 "$proj/project.yaml" | cut -d' ' -f1)
  after_time=$(stat -f %m "$proj/project.yaml")

  [ "$before_hash" = "$after_hash" ] || {
    echo "project.yaml was rewritten:"; diff <(echo "$before_hash") <(echo "$after_hash")
    cat "$proj/project.yaml"; return 1; }
  [ "$before_time" = "$after_time" ] || {
    echo "project.yaml's mtime moved from $before_time to $after_time — something opened it for writing"
    return 1; }

  # And the settings really were read: a gate that passes because the file was
  # ignored would assert nothing at all.
  local out
  out=$("$STILL" validate "$proj" 2>&1) || { echo "$out"; return 1; }
  "$FFPROBE" -v error -select_streams v:0 -show_entries stream=height -of csv=p=0 \
    "$WORK/untouched.mp4" | grep -qx 360 || {
    echo "the film is not 360 high, so short_edge: 360 was never read"; return 1; }

  return 0
}
check "a render never writes to project.yaml" gate_settings_untouched

# --- gates 8 and 9: the two cargo gates plan.md names -----------------------
check "cargo test -p spoonstill-app validation" \
  cargo test --release -p spoonstill-app validation

check "cargo test -p spoonstill-core path_safety" \
  cargo test --release -p spoonstill-core path_safety

rm -rf "$RENDERABLE/.spoonstill"

echo
total=$((pass+fail))
if [ "$fail" -eq 0 ]; then
  printf '%sM2 COMPLETE%s — %d/%d gates pass\n' "$GREEN" "$OFF" "$pass" "$total"
  printf '%sAll four slices: import, validation, the three sources, and speech.%s\n' "$DIM" "$OFF"
else
  printf '%sM2 INCOMPLETE%s — %d/%d gates pass\n' "$RED" "$OFF" "$pass" "$total"
  printf '%sRun `make test` for detail.%s\n' "$DIM" "$OFF"
  exit 1
fi

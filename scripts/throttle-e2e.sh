#!/usr/bin/env bash
# D-205 end to end, both halves of a refusal at the websocket handshake:
#
#   scripts/throttle-e2e.sh WORKDIR [REFUSALS] [SCENES]      429: waited out
#   STATUS=403 scripts/throttle-e2e.sh WORKDIR               403: fails fast
#
# A 429 is a rate limit: the render must wait it out and still make the film.
# A 403 is the service refusing this client, which was measured on 2026-10-15
# to be a stale `edge-tts` (7.2.8 against 7.3.1, released the day before the
# author's nine failed renders) — waiting cannot cure it, so the render must
# say so in seconds and name the upgrade.
#
# Deliberately **not** in `make gates`. What it asserts is that a render
# survives a refusal streak, and surviving one means waiting out D-195's
# cool-down ladder — 15 s, then 30, then 60 — so an honest run of it costs
# about two minutes of sleeping. That is the right price for a command run
# when the throttle path changes, and the wrong one on every `make gates`
# (D-134's rule, and D-149's reason for leaving a timing fix ungated).
#
# The service is stood in for, because the real one needs a few hundred lines
# to start refusing and then refuses for up to an hour. Measured here
# 2026-10-15: against the fixed code this renders every scene; against the
# code before D-205 the first line spends its three attempts on refusals and
# the render abandons the project.
#
# Needs: target/release/still (or STILL), FFmpeg.
set -euo pipefail

root=${1:?usage: scripts/throttle-e2e.sh WORKDIR [REFUSALS] [SCENES]}
refusals=${2:-3}
scenes=${3:-2}
status=${STATUS:-429}
here=$(cd "$(dirname "$0")" && pwd)
still=${STILL:-$here/../target/release/still}
[ -x "$still" ] || still=still
ffmpeg=${FFMPEG:-ffmpeg}

# `refusals` has to be at least D-094's attempt budget, or one line never
# exhausts its attempts and the unfixed code passes too — a test that cannot
# fail against the defect is not a test (D-116).
if [ "$refusals" -lt 3 ]; then
  echo "refusals must be at least 3, or this passes against the unfixed code" >&2
  exit 2
fi

rm -rf "$root"
work="$root/work"
proj="$root/proj"
mkdir -p "$work/calls" "$proj"

# One real MP3 for the stand-in to hand back once it relents.
"$ffmpeg" -y -loglevel error -f lavfi -i "sine=frequency=440:duration=2" \
  -ar 24000 -ac 1 -b:a 48k "$work/stub.mp3"

# The stand-in refuses its first $refusals calls with the author's verbatim
# stderr from 2026-10-14, then speaks. The call count is a directory entry per
# call, so concurrent workers cannot lose one.
#
# Written from a quoted heredoc and then substituted, so the Python traceback
# it has to reproduce — single quotes and all — is in the file exactly as the
# service emits it, rather than escaped through three levels of shell.
cat > "$work/stub-edge-tts" <<'STUB'
#!/usr/bin/env bash
for a in "$@"; do
  if [ "$a" = "--version" ]; then echo "edge-tts (stand-in)"; exit 0; fi
  if [ "$a" = "--list-voices" ]; then
    printf 'Name: en-US-AndrewMultilingualNeural\nGender: Male\n\n'
    exit 0
  fi
done
out=""; prev=""
for a in "$@"; do
  [ "$prev" = "--write-media" ] && out="$a"
  prev="$a"
done
: > "@WORK@/calls/$$-$RANDOM"
n=$(find "@WORK@/calls" -type f | wc -l | tr -d ' ')
if [ "$n" -le @REFUSALS@ ]; then
  echo "Traceback (most recent call last):" >&2
  echo "aiohttp.client_exceptions.WSServerHandshakeError: @STATUS@, message='Invalid response status', url='wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1?TrustedClientToken=6A5AA1D4EAFF4E9FB37E23D68491D6F4&ConnectionId=b3e0a86b17614dbaa320567193136816&Sec-MS-GEC=C150A6549A7F6004B2F9A3F7CA58A2F0EC38B989831B04320A3B922926DD5261'" >&2
  exit 1
fi
cp "@WORK@/stub.mp3" "$out"
STUB
# `|` as the delimiter: a temporary path contains `/` and never `|`.
sed -i.bak -e "s|@WORK@|$work|g" -e "s|@REFUSALS@|$refusals|g" \
  -e "s|@STATUS@|$status|g" "$work/stub-edge-tts"
rm -f "$work/stub-edge-tts.bak"
chmod +x "$work/stub-edge-tts"
export SPOONSTILL_EDGE_TTS="$work/stub-edge-tts"

# A project: $scenes stills, each with a line to speak.
i=1
while [ "$i" -le "$scenes" ]; do
  id=$(printf '%03d' "$i")
  "$ffmpeg" -y -loglevel error -f lavfi -i "color=c=gray:s=1600x1200:d=1" \
    -frames:v 1 "$proj/$id.jpg"
  printf 'This is the line for scene %s, and it has words in it.\n' "$id" \
    > "$proj/$id.txt"
  i=$((i + 1))
done

echo "rendering $scenes scenes; the stand-in answers $status to its first $refusals calls"
started=$(date +%s)
code=0
# `--audio-jobs 1` on purpose: the refusals have to land on one line, or they
# are shared out and no line ever reaches its third attempt.
"$still" render "$proj" --out "$root/film.mp4" \
  --resolution 720p --jobs 2 --audio-jobs 1 \
  --voice en-US-AndrewMultilingualNeural > "$root/render.log" 2>&1 || code=$?
elapsed=$(($(date +%s) - started))
calls=$(find "$work/calls" -type f | wc -l | tr -d ' ')

echo "exit=$code  elapsed=${elapsed}s  calls=$calls"
tail -3 "$root/render.log"

if [ "$status" = "403" ] || [ "$status" = "401" ]; then
  # Permanent: the render must refuse quickly and name the tool. The ceiling
  # is generous because a cold `still` start and two FFmpeg probes are in it;
  # what it rules out is the minutes a wait-out would cost.
  if [ "$code" -eq 0 ]; then
    echo "FAIL — a stale tool cannot be waited out, so this must not succeed"
    exit 1
  fi
  if ! grep -q "out of date" "$root/render.log"; then
    echo "FAIL — the render did not name the one thing that fixes it"
    exit 1
  fi
  if [ "$elapsed" -gt 60 ]; then
    echo "FAIL — ${elapsed}s is a wait-out, not a refusal: waiting cannot cure this"
    exit 1
  fi
  echo "PASS — refused in ${elapsed}s and named the upgrade"
  exit 0
fi
if [ "$code" -ne 0 ]; then
  echo "FAIL — the render gave up on a rate limit it should have waited out"
  exit 1
fi
if [ "$calls" -le "$refusals" ]; then
  echo "FAIL — the stand-in never relented, so nothing was waited out"
  exit 1
fi
echo "PASS — every scene rendered through a refusal streak"

# decisions.md — single source of truth for `spoonstill`

**Status:** active. This file wins over every other document in the repo.
**Last updated:** 2026-08-26 (M1: D-012 resolved; D-016 and D-037 added; D-070 and D-071 accepted).

`plan/PROJECT_BRIEF.md` and `plan/BRIEF_RECONCILIATION.md` both carry a
"superseded by `decisions.md`" banner. Until now that file did not exist, so
those banners pointed at nothing. This is that file.

## How to use this file

Each entry is a decision with a stable ID. Cite the ID in code comments, commit
messages, and PR descriptions (`// D-007: prescale is 3x output height`).

- **Accepted** — build to this. Do not re-litigate without changing this file.
- **Open** — needs the author. Do not guess; do not let an assumption leak into
  code. If you must proceed, pick the recorded default and say so in the PR.
- **Superseded** — kept for the audit trail.

Precedence when documents disagree:

```
decisions.md  >  plan.md  >  refergit.md  >  ffmpeg-findings.md (evidence, not policy)
              >  plan/BRIEF_RECONCILIATION.md (retired)
              >  plan/PROJECT_BRIEF.md (retired)
              >  plan/REFERENCES.md (retired)
```

`ffmpeg-findings.md` ranks below policy but **above every claim in the retired
docs and in the reference repos**, because it is measured on this machine.

---

## Product

### D-001 — `spoonstill` is a batch renderer, not an editor · Accepted

Input is `(still image + narration)` pairs. Output is one MP4 with Ken Burns
motion per still, cut on narration boundaries. No timeline, no scrubber, no
drag-to-arrange.

The rule that follows and overrides convenience everywhere: **if a feature
requires the operator to manually align a visual with a sound, it is the wrong
feature. Alignment is computed, never dragged.**

Source: `plan/PROJECT_BRIEF.md` §1, preserved by reconciliation §3.

### D-002 — n=500 is the design point · Accepted

Every design decision is evaluated at 500 scenes, not 5. A construct that is
fine at n=5 and quadratic at n=500 is a defect, not a future optimization.

### D-003 — Non-goals, closed list · Accepted

Rejected for V1, and a PR that adds one is wrong even if it works: timeline UI;
video clips as scene sources (stills only); image generation; script writing;
multi-track mixing beyond one music bed; collaborative/cloud features; mobile;
a server-side render tier.

---

## Architecture

### D-010 — CLI-first Rust workspace; Tauri is a late, thin shell · Accepted

```
crates/
  spoonstill-core     # domain: project, scene, audio source, motion, cache keys.
                 # Depends on nothing concrete. No Tauri, no React, no SDKs.
  spoonstill-media    # ffmpeg/ffprobe process boundary, segment profile, probing
  spoonstill-tts      # provider trait + ElevenLabs/Edge implementations
  spoonstill-state    # SQLite render state, checkpoints, cache index
  spoonstill-app      # application services: import, audio queue, render queue
  spoonstill-cli      # permanent, complete control surface. Ships first.
apps/
  desktop/       # Tauri 2 shell (M4+). Commands + channels only.
```

Required dependency direction — a violation is a build break, not a style note:

```
React -> Tauri adapter -> spoonstill-app -> spoonstill-core
CLI ------------------------^        -> infrastructure traits
```

Rationale: a headless CLI over 500 scenes is testable in CI; a webview is not.
The UI must never own the render queue. Resolves reconciliation §1.3 in favour
of the author's kenburns-batch brief over `plan/REFERENCES.md`.

### D-011 — FFmpeg over the CLI, not C-API bindings · Accepted

`Rust -> ffmpeg/ffprobe child process -> argument vector`. Never a shell string.
Rejected: `zmwangx/rust-ffmpeg` (maintenance-mode, painful cross-platform
builds), `ffmpeg.wasm` (order of magnitude slower, ~4 GB ceiling, no hardware
encode).

Study `plan/ffmpeg-sidecar` (v2.5.2) for the process boundary. It is **blocking
by design** — see D-012.

### D-012 — `ffmpeg-sidecar` is a pattern, not necessarily a dependency · Accepted

Adopt its shape: `FfmpegCommand` builder over `OsStr` args, a retained
`FfmpegChild` exposing `quit()` / `kill()` / `wait()` separately, and typed
events alongside retained raw stderr.

Do not adopt its runtime auto-download (`src/download.rs`) — a commercial
desktop app ships pinned, checksum-verified binaries. Its synchronous iterator
model needs a dedicated blocking worker boundary before it meets an async queue.

Whether to depend on the crate or reimplement ~600 lines is deferred to M1; the
call is cheap either way because the boundary is one module.

> **Resolved at M1, 2026-08-26: reimplement.** `crates/spoonstill-media/src/command.rs`
> is the whole of it — a builder over `OsString`, a retained child with
> `quit()`/`kill()`/`wait()`/`cancel()`, and two retention threads. Three
> reasons, in order of weight:
>
> 1. The auto-download is a default feature and is already rejected above, so
>    the dependency would have to be taken with `default-features = false` and
>    audited to stay that way.
> 2. It has no timeout-bounded `ffprobe`. plan.md M1 names probe hangs on
>    hostile media as a risk and requires a timeout **from the first call**;
>    that is not a wrapper around the crate, it is the part we needed most.
> 3. Its value-add is typed stderr events, and plan.md already rules that a
>    parser-classified log level is not a substitute for exit status plus output
>    validation. We retain raw stderr regardless, so we would be paying for a
>    parser we are not allowed to trust.
>
> The estimate of ~600 lines was fair; the actual boundary came to rather less,
> because the download, the version detection and the event taxonomy were the
> bulk of it and none are wanted.

### D-013 — Two artifacts: human manifest, machine state · Accepted

- `project.yaml` — human-owned. Hand-editable, diffable, copyable to another
  machine. **Never written by the renderer.**
- `.spoonstill/state.db` — SQLite. Machine-owned, disposable, rebuildable from the
  manifest plus cache. Per-scene status, cache keys, resolved durations,
  segment paths, attempt counts, failure detail.

One transaction per state transition. Deleting `.spoonstill/` must lose progress and
nothing else.

> Naming note: settled by D-073. Reconciliation §4.1 says `.kenburns/state.db`,
> after a brief that does not exist on this machine (D-074). The name is
> `.spoonstill/`, and it lives in exactly one constant so it stays cheap to
> change.

### D-014 — Credentials in `keyring-rs`, not Tauri Stronghold · Accepted

Stronghold is a Tauri plugin; the CLI could not reach keys stored there without
dragging in the desktop layer, which contradicts D-010. Behind a trait so the
store is swappable. `plugins-workspace` stays useful for `store` (UI prefs
only), `updater`, and `dialog`.

Resolves reconciliation §1.2. Supersedes `plan/REFERENCES.md`.

### D-015 — Global prefs are separate from project data · Accepted

A project folder copied to another machine must still render. API keys, window
state, and UI preferences are machine-scoped and live outside the project.

### D-016 — Diagnostics are written as they happen, and exported as one file · Accepted

Asked for by the author 2026-08-26: *"if anything fails on any machine ... I can
simply download the logs from the user machine and upload here on the developer
machine to know the exact reason for the failure."*

The constraint that shapes it: by the time a user says "it failed", the FFmpeg
stderr that explained it is gone, and the developer has neither the media nor
the environment. Recording only on failure does not solve this either — the
interesting failures are the ones where the render *succeeded* and the output
is wrong.

So:

- **Every run appends**, whether or not anything goes wrong, to JSON Lines under
  `.spoonstill/logs/`. Append-only and one self-contained record per line, so a
  crash mid-write loses the last line rather than corrupting the file that
  explains the crash.
- **What is recorded is what answers questions**: the exact command executed in
  paste-ready form (the lossless-cut `LastCommands` pattern), the resolved
  measurements, every profile mismatch field by field, and retained raw stderr.
- **`still diagnostics export` writes one text file.** One file, not an archive:
  the operator has to attach it to an email, and "one text file" has the fewest
  ways to go wrong. It carries the environment — OS, arch, FFmpeg version *and
  build configuration* — because "works for us, fails for them" is usually a
  build difference and no log line shows it.
- **Credentials never reach it.** With BYOK (D-014, D-023) the machine holds an
  API key. `spoonstill_core::diagnostics::redact` runs over every value on the
  way in, keyed on both field name and value shape, and is a pure tested
  function rather than a discipline anyone has to remember.
- **The bundle says what it contains**, in plain words, at the top: that it holds
  file paths from this machine, and that it holds no keys and no media. A person
  about to send a diagnostics file is entitled to know what is in it.

Logging never fails a render: a write error is retained and reported once, and
the sink then stops trying rather than turning one full disk into thousands of
errors.

---

## Audio

### D-020 — Exactly one of `text` | `audio` | `duration` per scene · Accepted

```rust
enum AudioSource {
    Tts   { text: String, provider: ProviderId, voice: VoiceId, settings: TtsSettings },
    File  { original_path: PathBuf },
    Silent{ duration: Duration },
}
```

Every variant resolves to `(normalized_audio_path, authoritative_duration)`.
Everything downstream — duration math, motion, segment render, concat —
consumes only that pair and never branches on origin. Adding a fourth source
must not touch the renderer.

Zero sources is a validation error. Two is a validation error. `Silent` exists
because title cards and breathing room are real; it also makes reconciliation
§4.2's objection moot at zero cost.

Rejected: MoneyPrinterTurbo's magic voice-name sentinel for silence
(`is_no_voice()`), which infers a silent track from an estimated text length.

### D-021 — Audio duration is authoritative, and it is measured · Accepted

Duration comes from `ffprobe` on the **resolved, normalized** artifact. Never
from the text length, never from a container header alone.

VBR MP3 headers lie. Normalize on ingest to a known profile (48 kHz stereo),
keep the operator's original file untouched, and probe the normalized copy.
Reject zero/NaN/absurd durations with the scene ID attached.

### D-022 — Narration is padded up to the frame grid, never trimmed · Accepted

`frames = ceil((duration + head_pad + tail_pad) * fps)`; the segment's audio is
`apad`-ed to `frames / fps`. Verified: a 3.717 s narration at 30 fps yields a
112-frame / 3.733333 s segment with audio padded to match, no drift.
See `ffmpeg-findings.md` §5.

Head/tail silence padding is part of the scene's duration, not a post-step.

### D-023 — TTS default differs by distribution · Accepted

Same provider trait, different configured default:

| Build | Default | Why |
|---|---|---|
| Internal team | Edge TTS | Free, and it emits word-boundary events that ElevenLabs does not — needed later for karaoke captions. |
| Sold | ElevenLabs (BYOK) | Edge TTS is a reverse-engineered, undocumented Microsoft endpoint. **No reverse-engineered endpoint may be load-bearing in a shipped product.** |

Resolves reconciliation §1.1. Providers get a per-provider concurrency cap,
retry with backoff, and rate-limit handling — ElevenLabs limits concurrency by
tier, in the low single digits on cheap plans.

No proxying through any server the project operates. Keys never leave the
machine.

---

---

## Motion and encoding

> These four decisions were speculative in every prior document. They are now
> measured on this machine. Evidence and reproduction commands:
> `ffmpeg-findings.md`.

### D-030 — `zoompan` on a single non-looped still input · Accepted

```
-i <image>
-vf "…fit into prescale canvas…,
     zoompan=z='…f(on)…':x='…':y='…':d=<N>:s=<W>x<H>:fps=<FPS>,
     setsar=1,format=yuv420p"
-frames:v <N>
```

`d` is **output frames per input frame**. With one still and no `-loop`,
`d=N` yields exactly N frames and the segment needs no `-t`.

Measured failure modes (`ffmpeg-findings.md` §2):

| Form | Result |
|---|---|
| still, no `-loop`, `d=N` | correct: N frames, exact duration |
| `-loop 1` + `d=N` + `-t` | also correct — `-t` is what bounds it |
| `-loop 1` + `d=N`, **no `-t`** | **runs forever.** 8,400 frames / 312 MB in 25 s and still going. This is the "5-hour hang" `Automated-Video-Generator` documents as BUG W5-2. |
| still, `d=1` | 1 frame, 0.033 s |

Use the first form. It cannot hang, because the frame count is structural.

### D-031 — `zoompan` beats time-based `scale`+`crop`; the reverse claim is wrong · Accepted

Reconciliation §4.4 suggested benchmarking `scale`+`crop` with `t`-expressions
as a likely win. Measured at 1920×1080 / 30 fps / 4 s from a 4000×3000 source:

| Approach | Wall | Peak RSS |
|---|---:|---:|
| `zoompan`, 3× prescale | **1.08 s** | **757 MB** |
| `scale`+`crop`, `eval=frame` | 8.32 s | 1,501 MB |

`scale` with `eval=frame` re-runs a lanczos resample of the full prescaled
image every frame. `zoompan` resamples only the crop window. 7.7× slower and
2× the memory, for the same 120 frames and the same 4.000 s output.

`Automated-Video-Generator` switched to `scale`+`crop` to escape a `zoompan`
**hang**, not because it was faster — and the hang was the unbounded `-loop 1`
form (D-030), which we do not use. Adopt their SAR lesson (D-033), not their
filter.

Keep `scale`+`crop` as the documented fallback if a future FFmpeg regresses
`zoompan`. Re-run the benchmark before switching.

### D-032 — Prescale is 3× output height. `scale=8000:-1` is superseded · Accepted

Measured, 10 s / 10 % zoom at 1080p30 — the worst case for `zoompan`'s per-frame
zoom quantization. Unique frames out of 300 (duplicates = visible stepping):

| Prescale | Unique frames | Peak RSS | Wall (300 f) |
|---|---:|---:|---:|
| 1× (none) | 188 / 300 | 744 MB | 1.22 s |
| 2× | 280 / 300 | 749 MB | — |
| **3×** | **300 / 300** | **761 MB** | **1.96 s** |
| 4× | 300 / 300 | 772 MB | — |
| 6× | 300 / 300 | 805 MB | 4.73 s |

Three conclusions, all against the prior documents:

1. **3× output height is the floor and the ceiling.** Below it, motion steps;
   above it, nothing improves.
2. **Prescale is a CPU cost, not a memory cost.** 1× → 6× moves peak RSS by
   61 MB (8 %) and wall time by 3.9× (288 %). Reconciliation §4.3's "108 MB per
   frame buffer × N workers" memory trap **did not reproduce**; `zoompan` on a
   non-looped still holds one input frame, not `d` of them.
3. `scale=8000:-1` is ~7.4× for 1080p and ~4.2× for 1080×1920 — the same fixed
   number meaning different things per aspect. Derive it: `3 * output_height`.

For reference, a plain static 300-frame encode of the same still peaked at
**1,418 MB** — nearly double every `zoompan` variant. Size the worker pool from
the encoder, not from the motion filter.

### D-033 — `setsar=1` is the last filter, unconditionally · Accepted

Reproduced: a 1999×1001 source (odd dimensions are normal in user photos)
through `scale` + `zoompan` with no trailing `setsar` produces
**SAR 30007:30000**, DAR 30007:16875 instead of 16:9. This is exactly the
`SAR 12160:12159` class of bug `Automated-Video-Generator` records as BUG W2-1.

`setsar=1` placed *before* the motion filters does not survive them. It goes
last, immediately before `format=yuv420p`.

### D-034 — Aspect fit happens **into the prescale canvas, before** `zoompan` · Accepted

```
scale=<3*OUT_W>:<3*OUT_H>:force_original_aspect_ratio=increase,
crop=<3*OUT_W>:<3*OUT_H>,
zoompan=…:s=<OUT_W>x<OUT_H>
```

Cover-fitting first means `zoompan`'s centred `x`/`y` expressions are
structurally incapable of walking off the image, so no black edge can enter.
Verified on a landscape 4000×3000 source rendered to 1080×1920: first, middle,
and last frame top-edge luma minima were 24 / 17 / 22 — never 0.

This is stronger than clamping the pan expressions after the fact, because it
removes the failure mode rather than bounding it.

### D-035 — Motion selection is deterministically seeded · Accepted

Seed from stable project + scene identity (project id, scene index, source
content hash) so a re-render is byte-identical and the cache is stable.
Unseeded `random.choice()` — as in `ffmpeg-ai` — breaks resume and caching.

`zoom: alternate` and every pan anchor obey this.

### D-036 — x264 `-preset medium -crf 18` is the default; hardware encode is opt-in draft · Accepted

VideoToolbox and NVENC band visibly on exactly this content — slow pans across
large smooth gradients. This render is filter-bound, not encoder-bound, so the
hardware path buys less than it appears to. Probe availability at runtime,
expose it as an explicit "fast draft" mode, and always fall back to libx264.

Resolves reconciliation §4.5.

### D-037 — Colour range and matrix are pinned in the filter chain · Accepted

Measured 2026-08-26 while building M1, and **not** previously recorded anywhere.

A JPEG is full-range. That range flag survives `format=yuv420p` all the way into
libx264, which signals it in the bitstream — so `ffprobe` reports the segment's
pixel format as **`yuvj420p`, not `yuv420p`**, from the documented D-030/D-034
chain. Reproduced on `land.jpg` at 1920x1080.

That is a segment-profile mismatch of exactly the D-041 kind: two scenes whose
sources differ in range produce segments that differ in pixel format and in
rendered colour, and the concat demuxer joins them with exit 0 and no warning.
It would surface as "some scenes look washed out", months later, in a finished
render.

So the chain pins colour explicitly, in two places:

```
scale=<3*OUT_W>:<3*OUT_H>:force_original_aspect_ratio=increase:out_range=tv,
crop=<3*OUT_W>:<3*OUT_H>,
zoompan=...,
setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709,
setsar=1,
format=yuv420p
```

`out_range=tv` on the prescale, where the full-range source first meets a
scaler; `setparams` immediately before `setsar=1`, because `-color_range` and
`-color_primaries` as **encoder** options did not survive — measured: the
matrix reached the output, the primaries and transfer did not.

Two consequences worth stating plainly:

- This adds one filter beyond the chain plan.md M1 specifies. plan.md says "in
  this order and no other", and this changes that order, which is why it is a
  decision rather than an implementation detail.
- `setparams` sets metadata only and does not touch SAR, so D-033 still holds
  literally: `setsar=1` remains the last filter before `format=yuv420p`.

`color_range`, `color_space`, `color_primaries` and `color_transfer` are all
pinned fields of the segment profile (D-040) and asserted per segment.

---

## Segments, concat, and state

### D-040 — One segment per scene; concat only after all are valid · Accepted

Pin the canonical segment profile in **one** constant and assert it before
concat: codec / profile / level, pixel format, width, height, SAR, frame rate,
time base, audio codec, sample format, sample rate, channel count and layout,
container and stream order, timestamp policy.

Hard cuts: concat demuxer + stream copy. Crossfades: a separate, explicitly
slower mode, because `xfade` forces a composed filter graph and re-encode whose
cost grows with clip count — unsuitable as the 500-scene default.

### D-041 — FFmpeg will not warn you about a mismatched segment. We must · Accepted

Measured: concatenating a SAR 30007:30000 segment between two SAR 1:1 segments
with `-c copy` produced **exit code 0, no error, no warning**, and an output
declaring SAR 1:1 for the whole file. The middle 60 frames render with the
wrong geometry, silently.

Therefore the uniformity assertion of D-040 is a hard gate in our code, run
against `ffprobe` output for every segment, before the concat list is written.
"FFmpeg didn't complain" is not evidence of a valid join.

### D-042 — Checkpoint only after the segment passes `ffprobe` · Accepted

Order: render → `ffprobe` validate (readable, exact duration, expected streams,
dimensions, codec, pixel format, SAR) → atomic move into the final segment path
→ commit the state transaction. A crash at any point leaves either a complete
valid segment or nothing that looks valid.

Failing at scene 147 of 200 must never discard the first 146.

### D-043 — Cache keys hash content plus every output-affecting setting · Accepted

- audio: `hash(text + provider + voice + settings)` for TTS;
  `hash(file bytes + normalization profile)` for supplied files.
- segment: `hash(image bytes + resolved duration + motion params + seed +
  segment profile + prescale + encoder settings)`.

Never key on a path or URL alone — the mistake in
`Automated-Video-Generator/src/agentic/operations/asset-cache.ts`. With BYOK,
a cache miss costs the operator money, so this is a correctness requirement.

Fixing a typo in scene 47 re-bills and re-renders exactly scene 47 plus the
final concat.

### D-044 — Bounded pools everywhere; two separate queues · Accepted

Unbounded `join_all` / `Promise.all` / `asyncio.gather` over 500 scenes is a
defect. The TTS queue and the render queue are separate, with independent
caps — TTS is network- and rate-limit-bound, rendering is CPU- and RAM-bound.

Render capacity derives from measured available RAM and the segment profile,
not from core count alone. Per D-032 the encoder, not the motion filter,
dominates: budget ~1.5 GB per concurrent segment worker until measured
otherwise on the target machine.

### D-045 — Cancellation is graceful, then forced, then clean · Accepted

Stop admitting new work → ask each active FFmpeg child to quit gracefully →
force-kill after a deadline → delete or mark partial outputs → persist a
resumable state. Supervise only children owned by the current job.

Explicitly rejected: `Automated-Video-Generator`'s `wave-scheduler.ts`
Windows process enumeration that kills unrelated "RAM-hogging" processes.

---

## Input and UX

### D-050 — Mis-pairing is structurally impossible, not operator-verified · Accepted

**Convention mode** — filename stem is the join key:

```
001.png  001.txt   -> image + text  -> TTS
002.png  002.mp3   -> image + supplied audio
003.png            -> image only    -> silent, default duration
```

**Manifest mode** — one CSV, one row per scene:
`image, text, audio_file, voice, duration, zoom_direction, zoom_anchor`.
The manifest wins where both exist.

Bulk import of supplied audio must be exactly as frictionless as text. Mixed
projects are the normal case: scene 3 ElevenLabs, scene 4 a client's recording,
scene 5 a silent title card.

### D-051 — The UI is a review grid, and it stays one · Accepted

Read-only by default: resolved scenes, thumbnails, durations, audio-source
badges, loud warnings for unresolved rows. The operator eyeballs it and hits
Render; the default path edits zero rows. Per-scene overrides exist but every
one of them is a step toward the timeline this project exists to avoid — D-001
is the test.

### D-052 — Hostile input is the normal case · Accepted

Design against, and keep a fixture for each: spaces, Unicode, and emoji in
paths; very long paths; case-insensitive-but-case-preserving volumes; Windows
vs macOS path semantics; truncated images; CMYK JPEGs; EXIF-rotated images;
zero-byte audio; odd frame rates; VBR audio whose header duration lies;
odd-dimension images (D-033).

Surface FFmpeg's actual stderr, mapped to a human cause, against the specific
scene ID. Never build a command line by string concatenation.

### D-053 — Single-scene low-res preview, same recipe · Accepted

The preview uses the same filter graph at reduced scale and cannot mutate final
render state. Never make the operator render 500 scenes to check one.

---

## Reference repositories

### D-060 — `editly` reviewed: reject the architecture, adopt three specifics · Accepted

`plan/editly` **is checked out** (`master` / `dc46674052ea`, 100 files, ~4,900
source lines, MIT). Every prior document says it is missing and unreviewed;
`refergit.md` §2 and §9 are stale on this point.

The reconciliation's premise was also wrong. It called editly "declarative
JSON → ffmpeg, Ken Burns already implemented" and the highest-value reference
for filter graphs. It is not an FFmpeg-filter tool at all: it renders every
frame in Node with fabric/canvas/GL and pipes raw RGBA to
`ffmpeg -f rawvideo -pix_fmt rgba -i -` (`src/index.ts:224-256`). Its Ken Burns
is canvas scaling (`src/util.ts:138-162`), not `zoompan`.

Reject the frame-server architecture for V1: it needs a JS canvas/GL runtime in
the shipping product, renders the whole video in one process with no per-scene
segments — so no resume, contradicting D-042 — and its pan range is a
resolution-blind magic number (`range = zoomAmount * 1000` px).

Adopt: (a) its declarative JSON config shape as prior art for `project.yaml`;
(b) `src/audio.ts` — the `atrim`/`adelay`/`apad`/`amix` mixdown and `loudnorm`
handling is the cleanest local reference for the V1.1 music bed;
(c) `src/easings.ts` — three easing functions, trivially portable to Rust.

Note the frame-server approach is the correct answer if motion ever needs to
exceed what FFmpeg filters express. It is a V2+ escape hatch, recorded here so
it is not rediscovered from scratch.

### D-061 — Remotion: concepts only, and the checkout can go · Accepted

No Remotion runtime, renderer, Player, or Studio in V1; custom commercial
licence; do not copy implementation code. Its useful ideas — motion as a pure
function of scene-local progress, centralized FPS/duration/dimensions, explicit
clamping, transition overlap modelled in duration math — fit in a paragraph and
are already captured in D-030…D-035.

That is 1.2 GB of the workspace, ~70 % of `plan/`, for a paragraph.
Recommend deleting the checkout. Not done unprompted — it is the author's disk.

### D-062 — Licensing boundary · Accepted

Patterns are not permission to copy code.

- `lossless-cut` is **GPL-2.0-only**. Study behaviour and UX; copying
  implementation into a proprietary-capable product is a licensing decision, not
  a code-review one.
- Remotion: custom commercial licence. See D-061.
- MIT/Apache reference code still needs attribution when copied.
- **The FFmpeg binary is the live risk.** This machine's ffmpeg is Homebrew
  8.0.1 built `--enable-gpl --enable-version3` — fine for development,
  **not shippable with a proprietary product**. Before any release, build or
  source an LGPL FFmpeg, and record the exact source, build flags, enabled
  codecs, and notices.

---

## Resolved from the Open list

> These arrived as questions for the author and have been answered. Kept
> together so the answer is as easy to find as the question was.

### D-070 — 16:9, 9:16 and 1:1 are all V1 · Accepted

Was Open with a recorded default of "yes, V1". **Implemented at M1 as the
default, and the default is now the decision.**

The marginal cost was as predicted — test-matrix breadth, not new code. D-034's
cover-fit into the prescale canvas is aspect-agnostic, so all three ratios come
out of one code path, and `spoonstill_core::geometry::Aspect` derives the
dimensions from a single short-edge parameter: 1080 gives 1920x1080, 1080x1920
and 1080x1080, which are the three sizes an operator actually names.

All three are covered in `motion_matrix` against landscape, portrait and square
sources, and all three are reachable from the CLI.

### D-071 — Cross-platform from M1; the numbers are still macOS-only · Accepted

Decided by the author 2026-08-26: *"whatever you are making should be compatible
for both mac and windows."* This supersedes the recorded default of
"macOS-first through M3".

What that means concretely, and what it does not:

**In force from M1 — the code is cross-platform by construction.**

- Paths are `Path`/`OsStr` end to end; no argument is ever built as a `String`.
- Graceful cancellation is `q` on FFmpeg's stdin, not a signal. There is no
  portable way to send SIGINT to a child on Windows, so the portable mechanism
  is the only mechanism — used on both platforms rather than as a fallback.
- Ctrl-C goes through `ctrlc`, which covers Windows console control events. A
  hand-rolled handler would need `unsafe`, which the workspace forbids.
- `fs::rename` replaces silently on Unix and **fails** on Windows when the
  destination exists, so the atomic move removes the destination first.
- `CREATE_NO_WINDOW` on every child, so no console flashes.
- Test media is built in Rust, not by `scripts/gen-fixtures.sh` — the test suite
  does not depend on bash.
- The Windows CI job is enabled.

**Not in force, and not to be claimed.** Nothing in this project has yet been
*run* on Windows, and every number in `ffmpeg-findings.md` is macOS arm64. Peak
RSS, encoder behaviour and path handling all still need measuring there
(`ffmpeg-findings.md` §8). Packaging, signing and an LGPL Windows FFmpeg build
(D-062) remain M4 work. "Compiles and is written correctly for Windows" is what
is true today; "verified on Windows" is not, until CI has run green there.

---

## Open — do not guess

### D-072 — Captions in M-scope? · Open

Default: **SRT in V1.1**, not V1 — the text is already present so it is nearly
free. Word-level karaoke via Edge TTS word boundaries is a genuine
differentiator for this genre and a real project; it is V1.1+ and depends on
D-023's internal build.

### D-073 — Project name is `spoonstill`; binary is `still` · Accepted

Decided by the author 2026-08-24, re-confirmed 2026-08-26. Neither `vidio` nor
`kenburns-batch`: both were rejected in the naming session
(`vidio` reads as a misspelling of "video" and collides with a large Indonesian
streaming service; `kenburns-batch` is a description, not a name, and borrows a
living person's name).

```
crates/spoonstill-core      apps/desktop  (M4+)
       spoonstill-media     binary:  still
       spoonstill-tts       state:   .spoonstill/state.db
       spoonstill-state     project: project.yaml (unchanged)
       spoonstill-app
       spoonstill-cli
```

The long name is the product, the repo, and the crates. The **command is
`still`** — five characters, because that is what gets typed a hundred times a
day. Crate names and binary names are independent, so `still` being taken on
crates.io is irrelevant: `spoonstill-cli` produces a binary named `still`.

Availability verified 2026-08-26: `spoonstill` free on crates.io and npm;
`still` free as a local binary name. `strip` is **not** free — it is real at
`/usr/bin/strip` (binutils) — which is why the short command is `still`.

> **Provenance, because this decision was lost once.** The author chose
> `spoonstill` in session `96be93ed` on 2026-08-24 ("lets go with this if there
> is no error: spoonstill"). That session then replied "your final answer:
> **stillstrip**", contradicting the author's own words, and stopped without
> recording anything — which is why this entry sat Open with `vidio` as its
> default for two days while the folder, all four documents, and D-013's state
> directory carried a name nobody had chosen. The lesson is D-073's real
> content: **a decision that is not written into this file did not happen.**

The workspace directory is still `vidio/` and renaming it is the author's call;
nothing in the build depends on it (D-013's constant is the only coupling).

### D-074 — The `kenburns-batch` master brief does not exist on this machine · Accepted

Searched 2026-08-26: no file matching `*kenburns*` anywhere under `~/Desktop`,
`~/Documents`, `~/Downloads`, or `~/Projects`, and no content match for
`kenburns-batch` in any sibling project. The only mentions on disk are inside
this workspace's own documents, all of them citing it rather than quoting it.

`plan/BRIEF_RECONCILIATION.md` calls it "Authoritative" and supersedes
`plan/PROJECT_BRIEF.md` against it. That comparison was made against a document
no longer available for inspection, so **reconciliation's claims about what the
master brief said are hearsay and rank below this file** — as the precedence
block at the top already states.

Every requirement in this file is traceable to a document that *is* here. If the
master brief resurfaces, reconcile it against this file explicitly; it does not
win on age.

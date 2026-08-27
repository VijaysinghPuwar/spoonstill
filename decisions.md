# decisions.md — single source of truth for `spoonstill`

**Status:** active. This file wins over every other document in the repo.
**Last updated:** 2026-08-26 (M2 slice 4 and the shell: D-080 how a project is
made and filled, D-081 where the TTS provider sits, D-082 the default provider
and the voice override, D-083 the window, D-084 loudness and trim; then D-087,
how a build reaches someone who is not the author).

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

> **Amended by D-076 (M2 slice 3).** The per-worker budget is now measured
> rather than guessed — 780 MB, not 1.5 GB — and the shipped default is derived
> from core count after all, because the speedup curve flattens long before
> memory becomes the binding constraint. RAM-derived capacity remains this
> decision's requirement and lands at M3, where n=500 is the milestone's
> subject. The rest of D-044 stands unchanged: two pools, independent caps,
> nothing unbounded.

### D-045 — Cancellation is graceful, then forced, then clean · Accepted

Stop admitting new work → ask each active FFmpeg child to quit gracefully →
force-kill after a deadline → delete or mark partial outputs → persist a
resumable state. Supervise only children owned by the current job.

Explicitly rejected: `Automated-Video-Generator`'s `wave-scheduler.ts`
Windows process enumeration that kills unrelated "RAM-hogging" processes.

---

## Execution — added at M2 slice 3

### D-075 — Normalized audio is 48 kHz stereo PCM in a content-named directory · Accepted

D-021 requires ingest normalization to "a known profile (48 kHz stereo)" and a
duration measured on the normalized copy. This fixes the rest of it.

**The artifact is `pcm_s16le` in a WAV container**, not compressed. The segment
encodes AAC anyway, so a compressed intermediate would put two lossy
generations between the operator's recording and the film. The cost is disk:
192 KB/s, so a 500-scene project averaging 30 s a scene holds about 2.9 GB of
normalized audio. That is acceptable because the cache is disposable and
rebuildable, and because the alternative degrades the product for every
operator to save space for some of them.

**The cache is a directory of content-named files, with no index**, under
`.spoonstill/cache/audio/`. `<kind>-<16 hex>.wav`, keyed per D-043 on the
source bytes plus the profile string — never on a path. The profile string is
versioned (`pcm_s16le/48000/2/v1`), so changing the profile misses the cache
rather than reusing artifacts made under the old one.

No database: `.spoonstill/state.db` is M3's deliverable, and a cache that needs
one to be readable cannot be inspected with `ls` when something goes wrong. The
directory is enough for M2's guarantee, which is that a re-run of an unchanged
project re-encodes nothing.

**A hit is measured, not trusted.** Every run probes the artifact and asserts
the normalization profile before using it, so a truncated or hand-edited entry
is regenerated instead of silently producing a scene of the wrong length. That
is D-021 applied to the cache rather than only to the ingest.

**Silence is generated, not simulated.** `AudioSource::Silent` writes a real
PCM track of an exact sample count (`atrim=end_sample=`, never `-t seconds`),
so a title card takes the same path through the renderer as a narrated scene.
D-020's test — "adding a fourth source must not touch the renderer" — is the
reason.

### D-076 — The render pool defaults to one worker per two cores, capped at four · Accepted

Requested by the author 2026-08-26: *"make it like that i can make multiple
render at the same time"*, left to this session to size. Measured before
deciding, on this machine (10 cores, 24 GB), twelve 1080p scenes:

| `--jobs` | wall clock | speedup | memory |
|---|---|---|---|
| 1 | 13.23 s | 1.00x | ~780 MB |
| 2 | 8.56 s | 1.55x | ~1.6 GB |
| 3 | 7.24 s | 1.83x | ~2.3 GB |
| 4 | 7.17 s | 1.85x | ~3.1 GB |
| 8 | 6.90 s | 1.92x | ~6.2 GB |

Reproduction and caveats: `ffmpeg-findings.md` §10.

**The curve flattens at three, and memory does not.** x264 at `medium` already
threads internally — at `--jobs 1` the encoder was using 2.7 cores on its own —
so additional workers mostly compete with the threads of the ones already
running. Going from four to eight buys 4% of wall clock for about 3 GB of
resident memory, on a machine the operator is also using.

So: **`available_parallelism() / 2`, clamped to `[1, 4]`**, and `--jobs` is not
capped in either direction — an operator who knows their machine can ask for
sixteen, and it is then a decision about memory rather than about cores.

**Two pools, per D-044.** The audio pool defaults to twice the render pool
because ingest is short and I/O-bound. At slice 4 that number stops being about
this machine at all and becomes the TTS provider's concurrency limit (D-023),
which is the reason the two were never one number.

Not decided here: RAM-derived capacity. It stays D-044's requirement and lands
at M3 with n=500 behind it. What M2 ships is a measured default and a flag.

### D-077 — Concurrency changes the timing and nothing else; one render per project · Accepted

Two guarantees that a parallel renderer has to make explicitly, because both
are easy to lose and neither fails loudly.

**A render is deterministic under any `--jobs`.** Measured: `--jobs 1` and
`--jobs 4` produce byte-identical films, and so does a third run that reuses
every cached artifact. It holds by construction, not by luck:

- every scene's move is seeded from stable identity **before** the pool starts
  (D-035), so no worker's choice depends on which worker it is;
- each worker writes to its own content-addressed segment path, so two workers
  can never target one file;
- results are collected **by input index**, so the concat list is in scene
  order however the workers finished.

This is an exit gate, not a comment. If a future change makes a worker's output
depend on scheduling, the gate fails.

**One render per project at a time**, enforced by `.spoonstill/render.lock`.
Every individual write is already safe — temporary file, then rename — but the
*film* is not: two runs with different settings would interleave segments from
both into one output. The lock names the process holding it, and `--force`
exists because a machine that lost power leaves one behind and an operator
should not have to know where it lives.

Deliberately **not** locked: two renders of *different* projects at the same
time. Nothing is shared between them — separate caches, separate segment
directories, separate outputs — so the only cost is memory, and that is what
`--jobs` is for.

### D-078 — The film is asserted on its video stream, not its container duration · Accepted

plan.md §M2's gate is "the film's duration equals the sum of the resolved scene
durations, within one frame". Measured while building it, that gate as stated
is against the wrong number.

An MP4's container duration is its longest track's, and an AAC track carries
priming samples the video does not. A six-scene film measured 18.054667 s in
the container and **18.033333 s in the video stream** — the video figure being
exactly `541 / 30`, frame-perfect, and the difference being exactly 1024
samples, one AAC frame (`ffmpeg-findings.md` §10c).

So the assertion after the join is, in order:

1. the segment profile, because a stream copy can still write a header that
   disagrees with its input (D-041);
2. the video stream's **declared** frame count against the sum of the segments'
   asserted counts — for MP4 this comes from the sample table the copy just
   wrote, and it is off by a whole segment the moment one is dropped;
3. the video stream's duration, within one frame;
4. the container's duration, within one frame **plus one AAC frame**.

Frames are not re-counted by decoding here, unlike a segment (D-030). Decoding
a 500-scene film to re-derive a number every segment already asserted
individually would cost minutes per run, and the failure it would catch —
a segment that is internally wrong — is caught at the segment.

The offset was constant across two, four and six segments rather than
accumulating per join. That is measured at small n only; if it ever
accumulates, check 4 fails first and loudly, which is the right place for that
surprise to appear.

---

## Brand — added at M2

### D-079 — The mark is three stacked stills, generated from one description · Accepted

The author chose this mark on 2026-08-26 and called it final. It is three
still frames stacked on a diagonal, the front one carrying the image — the
product's own sentence, drawn: many stills become one film.

**There is no wordmark.** The mark stands alone; `spoonstill` is set as ordinary
text beside it when a lockup is needed. Nothing in the identity is a typeface.

**The delivered file is not the asset.** What arrived was a 2048×2048 JPEG:
lossy, no alpha, ringing on every edge, and the black baked in. It is the
*reference*. The asset is vector, rebuilt from measurements taken off that
reference — a 51-unit stroke, a 721×819 still, stills at (0,0), (151,154) and
(303,307), a tight box of exactly 1024×1126, and an inner image inset two
stroke widths from the top and right and flush to the left and bottom. The
delivered steps are (151,154) then (152,153); that one-pixel irregularity is
kept rather than regularised, because forcing a uniform step would move the
artwork off the thing the author approved.

**One ink, three opacities — not three greys.** The ink is `#EFE9E0`. The front
still is 100%, the middle 86%, the back 72%. Each still is an opaque black
plate *under* a translucent stroke, and the plate covers only that still's
cavity, never its own stroke. That is what makes the two rear strokes brighten
to `#E5E1D8` where they cross, and it is the whole reason the stack reads as
depth. Anyone redrawing this with three flat greys will lose the crossing and
not know why it looks dead.

**The mark is defined on black and is not background-agnostic.** Those black
plates are load-bearing, so on any surface that is not near-black they show as
black rectangles. Every square use therefore carries its own black tile. This
is a constraint of the design the author picked, recorded here so nobody
"fixes" it by deleting the plates.

**Generated, not maintained by hand.** `scripts/gen-brand.py` holds the
geometry once and emits every SVG and every raster — `make brand`. Stdlib only,
no Pillow, no ImageMagick, no librsvg: the artwork is axis-aligned rectangles,
so coverage is computed analytically and the 16 px icon is exactly antialiased
rather than downsampled. Outputs are committed, so no build needs Python.
`.icns` needs macOS `iconutil` and is skipped elsewhere rather than failing.

Verified against the reference before anything was wired up: of the ~3.0M
colour channels further than 3 px from any edge, **exactly one** differs by
more than 6/255. All remaining error sits on edges and is the JPEG's ringing,
not ours.

The window palette follows the same construction (`apps/desktop/ui/styles.css`):
the ground is `#000000`, `--paper` is the ink at 100%, and `--grey` / `--dim`
continue the ink's own series at 68% and 46% instead of being separately
invented colours. The stylesheet previously named `#f2ede8 / #a29b94 / #6e6862`
as "the frames" — eyeballed, and describing a three-grey mark that does not
exist.

**Not done, deliberately:** the icon is a full-bleed black square. macOS Big Sur
icons want a rounded mask with padding and Windows wants full bleed, and
`bundle.active` is `false`, so the platform-specific masking is deferred to
whenever bundling is switched on. Revisit this entry then.

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

### D-054 — Project paths are contained, and out-of-bounds answers one thing · Accepted

Decided 2026-08-26, at the start of M2, implemented in
`spoonstill_core::path_safety`.

Every path in a manifest is untrusted (D-052). One function decides what any of
them means:

1. **Containment is judged on canonical paths, component-wise.** Canonicalize
   first — that is what folds symlinks, `..`, duplicate separators and the
   case a volume actually stores into one comparable form — then test
   containment with `Path::starts_with`, which compares components. A string
   prefix test passes `/work/proj-evil` for root `/work/proj`, and there is a
   test named for that.
2. **The project root is the boundary.** A relative cell joins onto the
   canonical root; an absolute cell is held to the same rule. There is no
   trusted-caller escape hatch.
3. **Out of bounds is one answer whether or not the file exists.** A path
   inside the project that is missing says so precisely; anything outside says
   only "outside", for both a file that is there and one that is not. The
   difference between two error messages is otherwise a working existence
   oracle for the whole host filesystem, queried one manifest row at a time.
4. **A missing path is resolved from the deepest ancestor that exists**, then
   finished lexically. Not for ergonomics: finishing lexically from the top
   lets `project/link-to-etc/nope` answer "missing inside the project" while
   its neighbour `project/link-to-etc/passwd` answers "outside" — which is
   rule 3's oracle rebuilt out of the missing-file case. Every component past
   the anchor is known not to exist, so no symlink can be hiding in it.

Prior art, per plan.md §M2: `plan/MoneyPrinterTurbo/app/utils/file_security.py`
`resolve_path_within_directory()` and `app/services/task.py:359`. **Adopt**
realpath-then-contain and the generic out-of-bounds error, which their comment
states outright. **Modify** `commonpath` to `starts_with`, and their
existence-tolerant `realpath` to the ancestor walk above, since Rust's
`canonicalize` requires the path to exist. **Reject** `allow_server_file_input`,
the flag that lets a trusted caller resolve outside the boundary: there is no
second trust level here to spend it on. If a shared media library outside the
project is ever needed, that is a new decision with an explicit list of roots,
not a boolean.

Canonicalization itself is the one fact the domain cannot derive, so it enters
through the `RealPath` trait (D-010). Everything else is pure, which is why the
tests run identically on macOS and Windows with no fixtures (D-071).

### D-055 — Validation reports everything, and guesses nothing · Accepted

Decided 2026-08-26, implemented in `spoonstill_core::project`.

plan.md §M2 asks for every problem at once. Three rules make that real, and
each of them is a place where the obvious shortcut is wrong at n=500 (D-002):

- **Nothing short-circuits.** `validate_drafts` returns the scenes that passed
  *and* every problem found, in input order. An operator with a 500-row
  manifest cannot fix one typo per run.
- **Present-but-wrong is not the same as absent.** A blank `text` cell is its
  own error, not a silent silent-scene. An unparseable `zoom_direction` is an
  error, not a fall back to the seeded default (D-035) — quietly substituting
  for a cell the operator did fill in is how 500 scenes come out wrong in a way
  nobody notices until delivery.
- **Two sources are never resolved by precedence.** D-020 already says two is
  an error; the message names every cell that was filled in rather than
  picking a winner.

A declared `duration` above `MAX_SCENE_SECONDS` (3600 s) is refused as absurd,
which is D-021's "reject absurd values" made concrete. It is a sanity bound,
not a product limit: a cell reading `36000` is a typo for `3.6` far more often
than it is an hour-long title card.

Scene IDs are validated rather than being plain strings, because an ID names
the segment file on disk: blank, `.`/`..`, path separators and control
characters are refused. Spaces, Unicode and emoji are **not** — D-052 says
those are the normal case, and every command is an argument vector (D-011).

### D-056 — What a project folder is, and which file wins · Accepted

Decided 2026-08-26, during M2 slice 2, implemented in
`spoonstill_app::import`.

D-050 fixes the two input modes and says "the manifest wins where both exist".
That sentence admits two readings at n=500, and they render different films, so
it is settled here rather than in code:

- **`project.yaml` holds settings, never scenes.** Rows come from the CSV
  manifest or from the folder — never from three places. 500 rows in YAML is
  not a file anyone hand-edits, and a third source of rows is a third set of
  precedence rules.
- **Everything is optional, including `project.yaml` itself.** A folder of
  images is a valid project. This is what makes D-050's convention mode real
  rather than a documented courtesy.
- **A manifest, when present, is the complete list of scenes.** Not a set of
  per-stem overrides merged onto a folder scan. The rejected reading — merge,
  manifest wins per stem — turns a 3-row manifest in a 500-image folder into a
  500-scene render, which is never what the operator meant by writing three
  rows. Images the manifest does not mention produce a **warning** (D-055's
  severity split), because only the operator knows whether an unlisted image is
  a source asset or a row they forgot.
- **A manifest named explicitly in `project.yaml` must exist.** Falling back to
  convention mode after the operator named a file would render a different film
  from the one they described, silently. The *default* manifest name being
  absent is not an error: that is simply convention mode.
- **Scene order is manifest order, or natural-sorted stems.** Order is the
  film's order, the `scene_index` that seeds motion (D-035) and part of the
  cache key (D-043) — so an unstable order re-rolls every move and misses every
  cache entry. Natural, so `scene2` precedes `scene10`; numbers before text, so
  `001.jpg` precedes `intro.jpg`.
- **The scene ID is the image's file stem in both modes**, so a project can move
  between modes without every scene changing identity.
- **An unpaired image holds for `defaults.duration`, four seconds by default.**
  D-050 says "default duration" without fixing a number; this is the number.

Strictness, following D-055: unknown keys in `project.yaml` and unknown columns
in the CSV are **errors**, not ignored. `apsect: 9:16` would otherwise render
500 scenes at 16:9 with nothing on screen to say why, and `zoom_ancor` would
give every scene the seeded anchor instead of the specified one.

Dependencies, per the licence gate in plan.md: `serde_yaml_ng` 0.10 (MIT) —
`serde_yaml` itself is deprecated and archived, and this is the maintained fork
with the same API; `csv` 1 (Unlicense/MIT). Both sit in `spoonstill-app`, never
in `spoonstill-core`, which still depends on nothing (D-010): the domain model
does not know what a file format is.

Two things abort a load rather than joining the problem list — a `project.yaml`
that will not parse, and a manifest that will not parse — because after either
there is nothing left to validate against. Everything else, from every stage,
lands in one list, ordered project-first and then scene by scene in render
order, so an operator fixes a project row by row rather than stage by stage.

### D-057 — The transition is a setting, and it overlaps silence only · Accepted

Requested by the author 2026-08-26: *"there should be an option to change the
transition effect — the hard cut might not be good, or distracting for the
eye"*, left to this session to decide the shape. D-040 already allowed a
crossfade mode; this decides how an operator reaches it and what it costs.

**One project-level setting, plus a per-scene override.** `transition` and
`transition_duration` in `project.yaml`, and the same two as optional manifest
columns — exactly how `zoom_direction` and `zoom_anchor` already work (D-050),
so there is one pattern to learn rather than two. The per-scene form describes
the join *into* that scene.

**A small closed set**, not a menu: `cut`, `fade` (through black), `dissolve`
(a straight crossfade). `xfade` offers dozens; shipping dozens is the editor
D-001 exists to refuse, and a wipe-hexagonal is a worse default than a good
dissolve. The set can grow when someone names a film that needed one.

**The default stays `cut` until the cost is measured.** Not taste — arithmetic.
A cut is the concat demuxer plus a stream copy; anything else forces a composed
filter graph and a **re-encode of both sides of every join**, and
`ffmpeg-findings.md` §8 lists that curve as asserted and unmeasured. At n=500
(D-002) an unmeasured multiplier is not a default. M3 measures it; if a
0.4 s dissolve costs little, the default is revisited then, with numbers. The
author's point stands either way: a cut is not always the right look, and the
setting is what makes that the operator's call rather than ours.

**The transition overlaps silence, never narration.** This is the part that has
to be decided before any of it is built, because it is the one that quietly
produces a broken film:

- A transition of length `d` between scenes overlaps their video by `d`. If the
  audio overlapped too, two narrations would talk over each other at every
  join — 500 times.
- So the overlap is taken from the **tail pad** of the outgoing scene and the
  **head pad** of the incoming one (D-022's padding, which is silence by
  construction).
- Where the pads are shorter than `d`, they are **extended** rather than the
  narration being cut into. D-022 already says narration is padded up and never
  trimmed; this is the same rule at the join. The scene holds a little longer,
  the film stays frame-exact, and nobody's last syllable is crossfaded away.

Consequence for the duration maths: a scene's frame count is still derived from
its own measured narration (D-021), and the film's total is the sum of the
scenes minus `d` per join — computed, asserted, and not left to `xfade` to
imply. The transition never changes what a scene *says*, only how long it is
held.

Not decided here: whether `dissolve` re-encodes the whole timeline or only the
segments either side of each join. That is an M3 implementation question with a
measurement attached, and it does not change anything an operator sees.

---

## Distribution — added after M2

### D-087 — Preview builds ship from a tag; FFmpeg stays the operator's · Accepted

Decided 2026-08-26, implemented in `.github/workflows/release.yml`,
`scripts/install.sh`, `scripts/install.ps1` and `README.md`.

M2 renders real films. M5 — signing, notarization, a bundled LGPL FFmpeg,
auto-update — is two milestones away. In between, the only person who can run
this tool is someone with a Rust toolchain and this repository, which makes
every milestone until M5 unverifiable by an actual operator. That is the
bottleneck this decision removes, and it removes it without taking one item off
M5's list.

- **A tag publishes a release.** `git tag v0.1.0 && git push origin v0.1.0`
  builds the CLI for `aarch64-apple-darwin`, `x86_64-apple-darwin` and
  `x86_64-pc-windows-msvc`, and the window for the same three, and attaches
  them to one GitHub release. Every leg is a **native** build on its own
  runner, not a cross-compilation: a target whose test suite cannot run there
  is a target we have not tested.
- **Every asset carries a SHA-256 sidecar, and the installers verify before
  they install.** An installer piped from `curl` into `bash` that does not
  check what it just downloaded is a supply chain with no gate in it. A
  mismatch installs nothing and says so.
- **The release is drafted first and published last.** Assets appear one leg at
  a time; a half-uploaded set is never a thing an operator can download.
- **FFmpeg is not in the release, and the app never fetches it.** The installer
  asks Homebrew or winget for it, once, before anything renders. This keeps
  both existing rules intact rather than bending either: D-012 forbids *the
  renderer* downloading a binary at runtime, and it still never does; D-062
  forbids redistributing the GPL build, and we redistribute nothing. What the
  operator installs is the platform's own package, from the platform's own
  package manager, on their own instruction.
- **Unsigned is stated, not hidden.** These builds trip Gatekeeper and
  SmartScreen because they are genuinely unidentified, and the README says
  exactly that, gives the two-click path through each dialog, and offers
  building from source as the alternative for anyone who would rather not.
  Signing certificates cost money and lead time and belong to M5.

Rejected, with reasons:

- **Bundling the Homebrew FFmpeg.** It is built `--enable-gpl
  --enable-version3`. D-062 already settled this; convenience does not reopen
  it.
- **Downloading FFmpeg from the app on first run.** This is precisely
  `ffmpeg-sidecar`'s behaviour, rejected in D-012 for a reason that has not
  changed: a renderer that quietly fetches a build produces output that cannot
  be reproduced. An installer the operator ran on purpose, before any project
  exists, is a different act from a render that fetches its own encoder.
- **Waiting for M5.** It costs two milestones of not knowing whether the thing
  works on a machine that is not this one.

**What this is not:** M5. No notarization, no code signing, no auto-update, no
third-party licence manifest, no pinned checksum-verified bundled FFmpeg. Every
one of those is still in `plan.md` §M5 and none of them is discharged here.
`README.md` is the front door this creates, and it is the one document written
for someone who has never read `decisions.md`.

---

## The window, continued

### D-088 — The title strip drags the window, and that costs one permission · Accepted

Decided 2026-08-26, implemented in `apps/desktop/ui/index.html`,
`apps/desktop/ui/styles.css` and `apps/desktop/capabilities/default.json`.

The window could not be moved. `titleBarStyle: "Overlay"` puts the web content
under the system title bar, so the strip the operator reaches for is the
webview, and the webview said nothing about dragging. The CSS claimed it did —
`-webkit-app-region: drag` on `.titlebar`, `no-drag` on its controls, and a
comment stating "dragging the window works anywhere on this strip". That is
Electron's property. WKWebView **ignores it silently**, which is the whole
reason it survived M4: there is no warning, no console message and no visual
difference between a bar that drags and a bar that is inert.

- **`data-tauri-drag-region="deep"` on the strip.** Tauri's own attribute,
  handled by a script it injects. `deep` means every descendant drags; the
  script blocks `A`, `BUTTON`, `INPUT`, `SELECT`, `TEXTAREA`, `LABEL`,
  `SUMMARY`, anything `contenteditable`, anything with a real `tabindex`, and
  anything carrying an interactive `role`. So the mark and the theme toggle
  keep working, and a control added to that bar later needs no rule written for
  it. The alternative — a `mousedown` handler calling `startDragging()`
  ourselves — is the same behaviour with our own list of what counts as
  clickable, and it loses double-click-to-zoom, which the same script gives us
  free.
- **`core:window:allow-start-dragging` is added to the capability.** It is
  **not** in `core:default` — the default window set is read-only accessors
  plus `internal-toggle-maximize` — so the attribute alone would have invoked a
  command the ACL denies, and the bar would have stayed inert with a message
  only in the console. The permission moves this window. It cannot resize,
  close, position or address any other window, and `windows: ["main"]` still
  scopes it to the one that exists.
- **The dead CSS is deleted, not left as documentation.** Two rules that do
  nothing plus a comment asserting they do is worse than nothing: it is what
  made this take four screens of source to find. The comment that replaces it
  says where the behaviour actually lives.

Not changed here: `resizable` was already `true` and the window resizes from
its edges; nothing in this decision touches size, and the 900×600 minimum
stands.

**The general form, worth more than the fix:** a platform that ignores an
unknown CSS property, and an ACL that denies a command quietly, will together
let a control surface *look* implemented indefinitely. Anything in the window
that cannot be asserted by a test has to be clicked before it is called done.

---

## Reference repositories

### D-080 — A project is made and filled by the program, not by the operator's file manager · Accepted

Decided 2026-08-26, implemented in `spoonstill_app::ingest`, reachable as
`still new` / `still add` and as the window's only two verbs.

**The problem this fixes was ours.** D-050 pairs `001.jpg` with `001.wav` by
stem, and D-056 makes a bare folder of images a valid project. Both are right.
But the operator's camera produces `IMG_2931.HEIC` and their recorder produces
`Voice 014.m4a`, so the price of those rules was that they renamed 120 files by
hand before anything would render — twenty to forty minutes for a sixty-scene
film. We deleted the timeline and handed back a filing job. Against a video
editor that is not a 2–3x saving, which is the bar the author set on the day
this was decided.

So the convention is ours to satisfy. The operator hands over whatever they
have, in any order, and the program names it:

1. **Photos sort naturally and become `001`, `002`, …** — natural, so `IMG_2`
   precedes `IMG_10`. Numbering continues from the highest already present, so
   a second drop appends.
2. **A recording or a script pairs by stem first, then by position.** If they
   wrote `IMG_2931.txt` next to `IMG_2931.HEIC` they have already stated the
   pairing, and guessing again could only get it wrong. Whatever is left over
   falls into the photos that matched nothing, in order.
3. **Copy, never move.** The sources are someone's photo library. A tool that
   empties it because they dropped the wrong folder is a tool they use once.
4. **Never overwrite.** There is no input to ingest that can destroy a file.
5. **Nothing unusable is fatal.** A `.DS_Store`, a README, a PDF: skipped and
   counted, because they dropped a folder, not a curated list.
6. **Nothing writes `project.yaml`.** An absent settings file is a valid
   project and every default works (D-056), so writing one would put a file in
   the folder the operator did not ask for and that the renderer is otherwise
   forbidden to touch (D-013).

A dropped *folder* contributes the media directly inside it and does not
recurse: a dropped home directory must not enumerate a hundred thousand files,
and a photo folder with `thumbnails/` must not import every thumbnail as a
scene.

**More recordings than photos is reported, not truncated.** It means the
operator is missing photos, and silently dropping the tail would produce a film
that ends early with no explanation.

The pairing is a guess, and D-051's grid is where a guess gets caught: every
row is on screen before a frame is encoded.

### D-081 — TTS sits above the process boundary, and Edge TTS is a subprocess · Accepted

Decided 2026-08-26, implemented in `spoonstill-tts`.

D-010's layering said `media`, `tts` and `state` were peers. That was wrong the
moment a provider needed to run a program: `spoonstill-tts` would have had to
grow its own spawn site, and `spoonstill_media::command` is *the one place a
process is spawned* (D-011) — argument vectors, timeouts, retained stderr, a
paste-ready command line for the bundle. Duplicating that to preserve a layer
diagram would have been the diagram winning over the rule it exists to serve.

So `spoonstill-tts` moves to its own layer above `media`, and
`spoonstill-cli/tests/architecture.rs` enforces the new direction:

```text
cli -> app -> tts -> {media, state} -> core
```

`app` still reaches `tts` only through its trait, and the CLI and the shell
reach it only through `spoonstill_app::tts` — neither may name the crate.

**Edge TTS is the `edge-tts` command line tool, not a Rust client.** D-023
already decided what Edge TTS *is* here: the internal and development
provider, never load-bearing in a sold build, because it speaks to a
reverse-engineered endpoint. That endpoint has an anti-abuse token derived from
a clock skew and a shared secret, and it moves. Reimplementing it in Rust means
owning a protocol whose specification is "whatever Edge does this month" and
shipping a build that stops working on a Tuesday. `edge-tts` tracks those
changes and is already installed on the machines that want this provider.

**The text goes in a file, never in an argument.** `--file`, never `--text`,
for three reasons in increasing order of importance: a paragraph can exceed the
argument length limit; arguments are visible to every process through `ps`; and
**the command line is logged** (D-016) and lands in the bundle the operator
sends us. Their script is their content. The only reliable way to keep it out of
our diagnostics is for it never to be an argument.

A provider that speaks HTTP itself, as ElevenLabs will, implements the same
trait and needs none of this.

**The provider's raw output is cached beside the normalized artifact.** The key
is `hash(text, provider, voice, settings, profile)` with every field
length-prefixed — without that, `voice="ab", text="c"` and `voice="a",
text="bc"` would hash alike and share one artifact. A normalization profile that
changes (D-075) must re-normalize every line; it must never re-speak one,
because with BYOK that is the operator's money.

### D-082 — The default provider is the one that works, and a chosen voice is an override · Accepted

Decided 2026-08-26.

D-023 makes the default differ by distribution — Edge internally, ElevenLabs in
a sold build. There is still no build flag, and `edge` is the only provider
that exists, so `DEFAULT_PROVIDER` is `edge`: a project that says nothing gets
the voice service this build can actually reach, rather than a name that fails
on its first spoken scene. When ElevenLabs lands, this constant is where the
distribution switch goes.

**A voice picked in the window is an override for that run.** `project.yaml` is
an input and the renderer never writes to it (D-013), so `--voice` /
`--provider` and the window's picker rewrite the loaded model before the cache
key is computed — switching voices is a cache miss, as it must be, and
switching back is a hit. An operator who wants it to stick writes it into
`project.yaml` themselves, which is also the only way it survives into someone
else's checkout.

### D-083 — The window is five tabs over one review grid, and it can write a scene's words · Accepted

Decided 2026-08-26, against the author's design brief and canvas
(`~/Downloads/Desktop application redesign`), implemented in `apps/desktop/ui`.

**Shape.** A native title bar with the project name; five tabs — Project,
Scenes, Render, Runs, Settings — and one primary action. Scenes is the app: a
dense grid with a sticky header, filter chips that are also counts, a search
box, and a status bar that never scrolls away. Density over whitespace, because
the operator is scanning 500 rows and not browsing six cards.

**A scene's narration is editable, in the grid.** This is the one thing the
window writes into the operator's folder, and it writes `NNN.txt` — their
words, in their file, because they typed them. It is not the renderer writing
state; that is what `.spoonstill/` is for (D-013). Emptying the cell removes
the file and the scene goes back to being silent, which is a real state and not
an error (D-050). **Manifest mode refuses the edit**: there the CSV is the
source of truth and a `.txt` written beside it would manufacture exactly the
two-sources-disagree conflict D-056 rejects.

Without this the window had no way to reach TTS at all — a spoken scene could
only exist if the operator had already made a `.txt` in a file manager, which
is the same filing job D-080 exists to delete.

**A duration is a dash until it is measured.** The grid shows a declared length
for a silent scene and nothing for the other two until the render resolves them
(D-021). A plausible estimate in that column would be the one number in the
window that was a guess.

**The palette is copied from the canvas, not interpreted from it.** The token
block at the top of `styles.css` is the canvas's own, cryptic names and all, so
that the next revision of the canvas is a paste rather than a translation;
every rule below it uses a semantic alias and never a raw `--1` or `--l`.

That block records an agreement reached twice independently. The canvas's first
version set a **blue** accent for the primary action and a blue-grey neutral
ramp (hue 255). This build shipped the mark's warm ink instead, because D-079
says the window carries no colour that is not a failure or a warning. The
author's updated canvas then removed the blue itself — `--a` now equals `--1`,
and the neutrals moved to hue 60–75, the mark's own grey. Colour in this window
means a failure, a warning, or money already spent, and nothing else.

**`spoonstill-desktop DIR` opens that folder.** A file manager's "Open With"
passes a path that way, and it means the window can be driven from a terminal
like every other part of this program — the useful converse of D-010's rule
that the CLI must be able to do everything the window can.

**Nothing in the webview names a path to open.** `open_film` and
`reveal_project` take no arguments and read the paths from Rust-side state. The
capability file therefore grants no filesystem scope and no opener scope at
all. This replaced a real bug: `opener:allow-open-path` enables the command
*without* a scope, so every path was denied and both buttons did nothing,
silently. Thumbnails work the same way — the asset protocol's scope is empty in
the config and granted to one directory when a project is opened.

**Not built yet, from the brief:** the scene detail panel with the motion-path
diagram and single-scene preview (D-053), the pre-render confirmation showing
cached-versus-billed counts, run history as *runs* rather than log lines
(M3 owns the state database that makes a run a record), recent projects on the
launch screen, and grid virtualization — 500 rows of DOM is fine, 5000 is not.

### D-084 — Every scene is levelled, and only a provider's padding is trimmed · Accepted

Decided 2026-08-26 from a full test of a two-scene Edge TTS film, implemented
in `spoonstill_media::audio`.

**Three defects, found by measuring the output rather than by watching it.**

**1. Nothing set the loudness.** `normalize` resampled to 48 kHz stereo and
stopped, so a film's level was whatever each source happened to be: the measured
Edge TTS lines came out at **-23.2 and -25.0 LUFS**, and a phone recording would
have arrived beside them ten decibels louder. At n=500 that is not polish, it is
the operator riding a volume knob scene by scene, or re-doing the film.

Every artifact is now brought to **-16 LUFS**, ceiling **-1.5 dBTP**, by a
**single measured linear gain** — one `loudnorm` analysis pass for the numbers,
then `volume=NdB`. Not `loudnorm`'s own single-pass mode, which reaches the
target by compressing: that changes how a voice sounds and makes the result
depend on FFmpeg's version, and D-077 requires byte-identical reruns.

Linear gain has a consequence worth stating rather than hiding: **a file whose
peaks are already high cannot reach -16 without clipping, so it does not get
there.** The recorded lines had a 17 dB crest factor, so the peak ceiling
allowed +4.88 dB of the +7.22 dB the target wanted, and the film landed at
-18.8 LUFS. That is the honest ceiling for gain that does not touch a sample's
shape. Getting the last three decibels means a limiter, which is a V1.1 decision
with the music bed, not something to smuggle in here.

The gain is clamped to ±24 dB. Without the clamp, a recording made with the
microphone muted — the classic mistake — arrives as 40 dB of amplified hiss at
full volume in a film about to be delivered.

**2. Every spoken scene carried the provider's padding.** Edge TTS pads each
line with about **0.24 s of silence before and 0.35 s after**, so every cut
landed roughly six tenths of a second after the speech stopped. Over 500 scenes
that is five minutes of dead air nobody chose.

Synthesized speech is now trimmed to `tts.trim_head` (default 0.10 s) and
`tts.trim_tail` (default 0.25 s). The settings say how much padding to **keep**,
so a provider that already pads by less is left alone rather than having silence
invented for it; a negative value keeps everything.

**A recording the operator supplied is never trimmed.** Their padding is a
decision and the provider's is an artifact — trimming the former is the "we
fixed this for you" behaviour plan.md §M2 rules out. Internal pauses are never
touched by either: only a silence that starts at the beginning or runs to the
end is padding.

**3. Changing the normalization made us pay to speak the line again.** The raw
provider output was stored at the normalized artifact's path with a different
extension, and that path's key included the normalization profile. So bumping
the profile — or the loudness target, or the trim — moved the raw file's name
too, the miss went all the way back to the provider, and with BYOK that is the
operator's money for audio we already had.

There are now **two keys**: `hash(text, provider, voice, settings)` names the
raw speech, and `hash(that, trim, profile)` names the normalized artifact.
Proven rather than asserted: changing `tts.trim_tail` and re-rendering wrote two
new `.wav`s and left both `.spoken` files untouched, timestamps and all.

**What this cost.** Two FFmpeg runs per unique audio instead of one, plus an
`ffprobe` when trimming — all sub-second, all behind the content cache, so it is
paid once per line ever rather than once per render.

### D-085 — The window's navigation is a rail, and the two render decisions are screens of their own · Accepted

Decided 2026-08-26, from the author's report on the D-083 window, implemented
in `apps/desktop`. It supersedes D-083's tab row and its Settings tab; the rest
of D-083 stands.

**Three complaints, one cause.** The report was: *"where can I change the voice
like a different voice of Edge"*, *"there is no option to see where to save the
file and what folder and what name"*, and *"why can I see Scenes Render Runs
and Settings [like that] — it is difficult to navigate around"*. All three are
the same defect. The window presented itself as a **viewer of a folder** when
the operator uses it as a **maker of a film**, and the two questions that get
asked before every single render — *in whose voice* and *to what file* — had no
home. One was a bare text box on the fifth tab that wanted `en-GB-RyanNeural`
typed from memory; the other was nowhere at all, because `project.yaml`'s
`output:` was displayed as a fact and never as a control.

**A vertical rail, and every destination at full ink.** The pill row put five
unselected tabs at `--2` on a dark ground, and it was read as five disabled
buttons beside one live one. Selection is now carried by a filled ground, a
left bar and weight; contrast is not spent on it. The rail also has room for
what the pill row had none of: a label per destination, the two standing
answers under them, and the primary action at the foot of the same column, so
setting up a render and starting one are one object.

**Voice is a screen.** Provider status, the whole catalogue, a language filter,
a gender filter, a search box, and the chosen voice said once and said large.
Every row auditions on click. **An audition is not a render**: it speaks one
short line through the same cache and the same normalization a scene gets
(D-084), so it sounds like the film will sound and hearing it twice costs
nothing — which is the difference between comparing six voices and settling for
the first one that worked. `spoonstill_app::audio::preview` is that call, and
it is in the app crate rather than the shell, because the shell owns no
business logic (D-010).

**Output is a screen.** A file name, a folder, a Browse button, and the joined
absolute path shown live. **The join happens in Rust** (`resolve_output`),
because a webview that concatenates a folder and a name is a webview that can
be made to concatenate `../..` — the same reasoning as D-052 and D-054. It
refuses a name carrying a separator, a leading dot or a `..`, refuses a folder
that is not there, and adds `.mp4` to a bare name because a film is an MP4
(D-078) and the operator who typed `holiday` did not mean a file their player
will not open. Refusal happens as the box is typed in, not forty minutes into a
render (D-002).

**Both are overrides for one run.** `project.yaml` is an input and this program
never writes to it (D-013). The window's own choices live in the window's own
storage, keyed by project folder, so they survive a reload without becoming a
fact about the project — an operator who wants a voice to stick writes it into
`project.yaml`, which is also the only way it survives into someone else's
checkout (D-082).

**One CSP line was load-bearing.** The audition plays through the asset
protocol, and `media-src` was absent from the policy — so it fell back to
`default-src 'self'` and every audition would have been blocked silently. The
artifact is also granted to the webview file by file rather than by directory:
it is a file the command just produced inside `.spoonstill/`, not a path the
frontend named.

**And one CSS bug that read as a broken button.** `button:hover:not(:disabled)`
outranks `button.primary` on specificity, so the primary action turned dark
grey under the pointer — it looked disabled at the exact moment it was being
aimed at. Restated explicitly. This shipped in D-083 and nobody saw it, which is
the argument for driving the real HTML in a browser rather than reading it.

**Settings is gone.** Everything it held was either a control that belongs on
Voice or Output, or a fact that belongs on Project. A tab whose content is
"things we could not place" is a tab that gets opened by accident.

### D-086 — Home is the operator's projects; a project is the rail · Accepted

Decided 2026-08-26 from a second round of the author's own use of the D-085
window, implemented in `apps/desktop`. It supersedes D-085's Project tab and
D-083's start screen.

**What was reported, verbatim in substance.** *"Here I cannot click on
project"*; *"if this is the home screen there is no need for render button,
Scenes, Output and all — this is not what we needed"*; *"create a proper home
screen where all the projects are there, and then inside the project we have
voice, settings, scenes and render"*; and, of the language filter, *"this is not
clear which is why it is hard to navigate around"*.

**The window had one level where the operator has two.** It was built as a
viewer of *one folder*: it opened on a start screen with two verbs, and the only
route to a project made yesterday was the folder dialog and the operator's own
memory of where they put it. So the Project tab tried to be a home screen from
inside a project, and failed at it — it restated the folder, the film, the
geometry and the voice, all of which the title bar, the rail and Output already
say, and not one line of it did anything when clicked.

**Home is now the projects.** Every folder that has been opened, newest first,
with the path written `~/Downloads/test` and how long ago. Clicking one opens
it. The list is Rust's, kept in the OS config directory — *which projects has
this person opened* is not a fact about any one of them, so it does not go under
a `.spoonstill/` (D-013 governs machine state **for a project**; this is machine
state for the operator). It is written inside `validate_project` rather than by
its own command, so there is no way to open a project and have it not appear.

**A project that has moved is shown and marked, never dropped.** Struck through,
labelled "moved or deleted", with a Forget button. A list that silently loses a
row is a program that appears to have lost the work; forgetting is the
operator's verb, and it removes a line from a list and never a folder from a
disk.

**Settings is app-level and lives on home.** Whether the voice service is
reachable and how to install it if not (D-002), which voice it falls back to,
the theme, and what the program is. The per-project Settings tab D-083 shipped
is gone: everything in it was either a control that belongs on Voice or Output
(D-085) or a fact that belongs beside the thing it describes. A tab whose
contents are "what we could not place" gets opened by accident and never on
purpose.

**"default" is not an answer to "whose voice will I hear".** `tts.voice`
defaults to the literal string `default`, and the window showed that word.
`Provider` gains `default_voice()`, Edge returns its named default, and every
surface now resolves it — the rail, the Voice screen and Settings all say
"Ava · English (United States)". This is a trait method rather than a constant
read in the shell because the shell must not know a provider's internals
(D-010), and because ElevenLabs will answer it differently.

**A locale code is not a language.** The filter listed `af-ZA, am-ET, ar-AE, …`
in the provider's own order, so the screen opened on Afrikaans and the operator
auditioned `af-ZA-WillemNeural` while looking for English. Names now come from
the platform's `Intl.DisplayNames` — no shipped table to go stale — and they are
built **from the tag's parts**, not from the whole tag. Asked for `en-GB` whole,
the platform answers "British English", which files the English voices under A,
B and I; built from its parts it reads "English (United Kingdom)", and every
English sits together. The list is sorted by that name, the code stays visible
beside it because it is what goes in `project.yaml`, and the filter **opens on a
language the operator can read** — the project's own voice decides it, failing
that the one this machine is set to.

Every voice row now reads `Ryan · English (United Kingdom) · Male ·
en-GB-RyanNeural`, in that order: the name they choose by, the language they
filter by, and the id the renderer needs, last.

**What this cost, and what it did not.** Three commands (`recent_projects`,
`forget_project`, and the `default_voice` the status already carried) and one
screen. It did not cost a state database: the list is a JSON file of at most
thirty entries, and M3's `state.db` is per project and stays that way.

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

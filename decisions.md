# decisions.md — single source of truth for `spoonstill`

**Status:** active. This file wins over every other document in the repo.
**Last updated:** 2026-08-26 (M2 slice 4 and the shell: D-080 how a project is
made and filled, D-081 where the TTS provider sits, D-082 the default provider
and the voice override, D-083 the window, D-084 loudness and trim; then D-087,
how a build reaches someone who is not the author, and D-089, the folder
whose name ends in a space).

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

### D-089 — A path is never trimmed, and a disabled button says why · Accepted

Decided 2026-08-26, from a real project that could not be rendered:
`~/Downloads/RANDOM vidoe ` — five valid scenes, zero problems, five lines of
narration ready to speak, and a Render button greyed out with no explanation
anywhere on screen.

The folder's name ends in a space. Finder makes such a folder without comment
and macOS keeps it. `resolve_output` did `PathBuf::from(dir.trim())`, which
named a folder that does not exist, so `is_dir()` was false, so the command
returned an error, so Render was disabled.

Two rules come out of it, and the second matters more than the first:

- **A path is never trimmed.** Whitespace at either end of a path is part of
  the name, not formatting around it. The only `.trim()` in the workspace that
  had ever touched a path was this one — every other one trims a scene ID, a
  voice, a probe value or a line of text, all of which are typed or parsed
  content. A *file name* the operator types into a box is still trimmed, and
  that is the deliberate exception: a trailing space there is a slip, and the
  box is the only place a person types one. The folder comes from the picker or
  from the project root and is used verbatim.
- **A disabled control explains itself where it is.** The reason existed — the
  Output screen would have shown `... is not a folder`. But nothing on Scenes
  said so, and Render is on Scenes. One function now decides whether a render
  can start and the same function writes the reason next to the button, so the
  three copies of `has_errors || outError` cannot drift and cannot go quiet.
  A control that refuses without saying why is indistinguishable from a broken
  program, which is what this one looked like.

A third, smaller one: **a path in an error message is quoted**, so
`"…/RANDOM vidoe " is not a folder` shows the operator the space. Unquoted, it
reads like a sentence that should have worked.

The regression test creates folders named `RANDOM vidoe `, ` leading` and
`  both  ` and resolves a film into each. It was run against the old code
first and fails there — a test for a hazard that has never failed is a test of
nothing (`ffmpeg-findings.md` §8b).

---

### D-090 — A hostile name is only hostile where it is legal · Accepted

Decided 2026-08-26, after the Windows CI job — running on a push for the first
time — refused two of D-052's hostile fixture names outright.

`semi;colon &and& pipe|` and `trailing space ` are ordinary names on macOS.
On Windows `|` is a reserved character and a directory whose name ends in a
space is `InvalidFilename`, error 123: `create_dir_all` fails before any
spoonstill code is reached. A test cannot assert that we survive a name the
operating system will not create.

So the hostile set is split by what each platform can actually make. The
shapes those two names stand for — a shell metacharacter, and awkward
surrounding whitespace — are represented on every platform by
`semi;colon &and& ampersand` and ` leading space`, which are legal everywhere
(Win32 reserves a *trailing* space and period, not a leading one). The two
POSIX-only names still run on POSIX, so **the macOS coverage is unchanged**;
Windows gains the coverage it can hold rather than a test it cannot pass.

This is the mirror of D-089. There, a trailing space in a folder name was a
legitimate thing macOS allowed and our code wrongly trimmed away. Here, the
same trailing space is a thing Windows genuinely forbids. Both follow from one
fact worth stating plainly: **the set of legal names is the platform's to
decide, not ours** — we may neither trim it down nor assume it is the same
everywhere. D-071 said the code targets both platforms; this is the first
concrete place where "both" means "differently".

---

### D-091 — Progress is shown in the film's order, and a selection says the word · Accepted

Decided 2026-08-26, from two screens the author could not read.

**The live panel.** `still render` renders several scenes at once (D-076) and
logged each one as it finished, newest first. On an eleven-scene project that
produced `011, 010, 009, 008, 007, 006, 003, 005, 002, 004, 001` — which reads
as a film assembled in the wrong order. It is not: `pool::run` writes each
result into `results[index]`, so what comes back is input order whatever order
the workers finished in, and `results_come_back_in_input_order` pins that with
a reverse-sleep so completion order is the *opposite* of input order. The film
was right and the screen was wrong.

So the panel is now the film's own order: every scene present from the moment
the render starts, each row updating in place from `waiting` to
`narration ready` to `rendered`. A pool that finishes 003 before 002 can no
longer read as a film that plays them that way, and the panel gained something
the log never had — you can see at a glance which scenes have *not* been done.
The subtitle says it in words too, because a correct picture that needs
explaining is only half a fix.

**The voice list.** A row was highlighted when its ID matched the *effective*
voice — which is the operator's override if they made one, and `project.yaml`'s
voice if they did not. Those are different facts and they looked identical, so
a voice appeared selected before anything had been selected, and clicking a row
changed nothing an operator could see. Each row now says which it is in a word,
`✓ Selected` or `Project default`, the header carries the same distinction as a
tag, clicking sets the status line to
`Andrew Multilingual will read every written line.`, and the current row is
scrolled into view rather than hunted for. The subtitle says that clicking
chooses, which nothing on the screen had ever said.

The rule behind both: **a screen that shows a true thing in a misleading order,
or marks a state without naming it, is a defect of the same kind as a wrong
number.** D-089 is the same rule applied to a disabled button.

---

### D-092 — Settings is where you act, not where you are told what to do · Accepted

Decided 2026-08-26, from an operator reading the Settings screen and finding
nothing on it that did anything.

- **A provider installs itself.** Printing `pip install edge-tts` and leaving
  the operator to find a terminal is the program declining the thing it has
  just asked for. `Provider::install()` is part of the trait; Edge tries
  `pipx`, then `brew`, then a `--user` pip, in that order, because the plain
  `pip install` we used to print is the one that fails on a modern Homebrew
  Python with `error: externally-managed-environment`. **Success is not "the
  installer exited zero"** — it is `availability()` returning Ready
  afterwards, checked before anything is reported, because a `--user` install
  can land somewhere that is not on `PATH`.

  This is not D-012's forbidden runtime download. That rule is about the
  *renderer* quietly fetching an encoder mid-run and producing output nobody
  can reproduce. This runs when a button that says install is pressed, before
  any project is rendered, and it fetches through the platform's own package
  manager rather than pulling a binary from us — the same reasoning D-087
  applies to FFmpeg.

- **The machine gets a fallback voice, and it is a fallback.** Precedence, and
  it is worth writing down because there are now three answers: this run's
  Voice-screen override, then the project's own `tts.voice`, then the
  machine's fallback, then the provider's own. Nothing at any level writes to
  `project.yaml` (D-013).

- **A project is a card.** As rows the only thing between one project and the
  next was a hairline, and a long path made the whole screen one grey block.
  The path now wraps rather than trailing off, because on a card there is room
  and the folder is how an operator tells two projects of the same name apart.

- **Prose that restates the screen is deleted.** A control that needs a
  paragraph is a control that is wrong; a control that has one anyway is a
  screen nobody reads. Gone from Settings, Output and the Voice card: what
  `project.yaml` is, what Edge TTS is, what an MP4 is, that the destination is
  for the next render — all of it either visible in the control beside it or
  written in `decisions.md` where it belongs. What is left is a state line, a
  control, and at most one short line where a control genuinely needs one.

- **The Runs tab is gone.** It showed one project's log inside that project,
  which is the question an operator does not have — they have "something went
  wrong", and the folder is often the thing they are trying to establish.
  D-093's CSV answers that from Settings, and the per-project JSON Lines is
  still on disk and still in the diagnostics bundle, so nothing was lost except
  a screen that could only answer a question you had already answered. The two
  window commands that fed it went with it.

Also recorded here because it was found in the same pass: **the release builds
one universal `.dmg`** rather than one per architecture, and the Intel CLI
cross-compiles from the arm64 runner. GitHub no longer serves `macos-13` — two
release runs sat queued fifteen minutes and never started. D-087 said native
builds only, on the grounds that a target whose tests cannot run there is a
target we have not tested; that reasoning still holds, so the cross-compiled
leg **says so** rather than pretending otherwise: it is checked with `file` for
the architecture it claims, and it is not smoke-tested, because the arm64
runner has no Rosetta to run it with.

---

### D-093 — One CSV of everything, beside the operator's other machine state · Accepted

Decided 2026-08-26, asked for directly: *"all the logs of all the folders
should be inside the CSV file which I can see from the location from the
settings"*.

D-016 puts every event in the project it belongs to, and that stays true — the
diagnostics bundle needs it, and a project that moves takes its own history
with it. What that arrangement cannot answer is **"what went wrong just now"**,
because the answer is spread across every folder the operator has ever used and
the folder is frequently the very thing they are trying to work out.

So every event is *also* appended to one CSV in the machine's config directory,
beside the recent-projects list and for the same reason (D-086): which projects
this person has used is not a fact about any one of them.

- **Every event, not one row per render.** A render summary would have answered
  "what have I made" and not "why did that one sound wrong". Each row carries
  the FFmpeg command line, the filter string, the probe result — the same
  detail the JSON Lines log holds, in a file a spreadsheet sorts.
- **It is a convenience over authority that lives elsewhere**, so every failure
  in that module is silent: a render must never fail because a spreadsheet
  could not be written. `Tee` composes the two sinks rather than folding the
  CSV into `FileLog`, so the per-project log still works on a machine that has
  no config directory.
- **Every field is quoted, always.** A project folder is the operator's to name
  and routinely holds a comma; captured FFmpeg stderr holds commas, quotes and
  newlines together. An unquoted field is a row that silently gains a column
  (D-052 — hostile input is the normal case).
- **The column order is a contract** with a spreadsheet the operator may
  already have open and filtered. New columns go at the end; a test pins it.
- **It rolls at 16 MB**, keeping one previous file, so a diagnostic convenience
  cannot fill a disk.

Reachable from Settings — path, Open, and Show in Finder, all resolved in Rust
so the webview names neither a directory nor a file — and from
`still diagnostics where`, because the CLI can do everything the window can
(D-010).

### D-094 — The one network call is classified, retried, and asked about first · Accepted

Decided 2026-08-26 while hardening the Edge provider. Everything else this
program does is local, deterministic and repeatable. Speaking a line is not: it
crosses a network to a reverse-engineered endpoint (D-023), it is the step most
likely to fail while nobody is watching, and at n=500 it is the step that will
fail *sometimes*. Three rules follow, and they are all in
`spoonstill_tts::edge`.

**1. A failure is classified before it is reported.** `edge-tts` puts a Python
exception on the last line of stderr and exits non-zero — every time, for every
failure mode, verified against 7.2.8 on this machine. That line is matched on
the exception's *class name*, never on its prose, because the sentence after
the colon is written for a human and changes:

| stderr says | verdict | what the operator reads |
|---|---|---|
| `NoAudioReceived` | permanent | the row, and that a line of pure punctuation does this |
| `ValueError: Invalid voice` | permanent | the voice, and `still voices` |
| `aiohttp.*`, `WebSocketError`, `429`, a timeout | transient | nothing — it is tried again |
| anything unrecognised | **permanent** | the last line, verbatim |

Unrecognised defaults to permanent on purpose. An unknown failure is far more
often a wrong argument than a flaky socket, and retrying every unknown failure
three times with backoff turns one bad project into a batch that takes half an
hour to say so.

**2. A transient failure is retried three times with a growing pause**
(0.5 s, then 1.5 s), and a permanent one is attempted exactly once. Three
rather than five: a dropped websocket and a rate limit clear in seconds or not
at all. The backoff is **not jittered** — deterministic keeps the slowest
failing render reproducible, and the pool here is four workers, not four
hundred. The loop is a free function over a closure with the sleep injected, so
it is tested without a network and without waiting; that matters because retry
logic that can only be exercised by unplugging a cable is retry logic nobody
tests.

**3. The service is asked whether it works before the pool starts** — D-002's
oldest rule, which the render path was not actually following. `edge-tts`
missing was discovered by the first scene needing speech, so a project whose
spoken scenes come late paid for the whole run first. `check_voice_service` in
`spoonstill_app::film` now asks once, names the provider, the fix and how many
scenes were about to fail, and refuses before anything is rendered. It asks
**only about work that is actually left**: a line already in the speech cache
needs no service, so a finished project still re-renders on a machine that has
since lost the tool. `crate::audio::speech_key` is shared with `resolve` rather
than reimplemented, because a pre-flight check with its own copy of a cache key
is a second cache with its own bugs.

Three smaller things settled at the same time, each with a test:

- **The ceiling on one line is derived from the line.** Measured here: a short
  line takes 0.6 s and 5 980 characters took 37.6 s — about 6.3 ms/char. The
  old fixed 90 s therefore refused a long paragraph on a slow link while
  calling it "the network is gone". It is now 60 s plus 30 ms/char, capped at
  15 minutes.
- **A voice id is not always `xx-YY-Name`.** Six of the catalogue's 322 are not:
  `iu-Cans-CA-SiqiniqNeural` carries a script subtag that belongs to the locale,
  and `zh-CN-liaoning-XiaobeiNeural` carries a dialect that is not a BCP-47
  subtag at all. Taking the first two segments produced `iu-Cans` — a tag
  D-086's `Intl.DisplayNames` cannot name, listing those voices under no
  language. The segments are now read positionally: language, optional
  four-letter script, optional region, and anything else ends the tag.
- **An empty voice list is an error, not an empty list.** If `--list-voices`
  prints something this build cannot parse, the window would otherwise draw
  three hundred voices as none, with nothing wrong on screen and nothing in the
  log. It now says so and quotes the line it could not read. The real 322-row
  table is checked in at
  `crates/spoonstill-tts/fixtures/edge-list-voices-7.2.8.txt` and every row of
  it is parsed by a test.

**Two test suites, and the split is deliberate.** `tests/edge_retry.rs` fakes
`edge-tts` with a shell script that fails on cue, so the real spawn → classify
→ retry path runs offline, in half a second, inside `make test` (Unix only — a
`.sh` is not a program on Windows, and faking one there would be testing the
fake). `tests/edge_live.rs` provokes each failure against the real tool and the
real service, and is `#[ignore]`d behind `make tts-live`, because `make test`
must give the same answer on a plane as in CI. The live suite exists for one
reason: the classifier is built on recorded stderr, and recorded stderr goes
stale the moment `edge-tts` is upgraded. When it fails, the fixtures are what
is out of date.

One fact worth keeping, measured while doing this: **speaking the same line
twice does not produce the same bytes** — two runs gave files of identical
length differing in 5 916 bytes. Nothing downstream depends on it, since
duration is measured and not assumed (D-021), but it is why D-084's raw speech
cache is load-bearing rather than an optimization.

### D-095 — Long form is split into requests, and a line too long for a scene is refused · Accepted

Decided 2026-08-26, after measuring what this provider actually does with
long-form narration. Three numbers, all from this machine against
`edge-tts 7.2.8`:

| text | one request takes | audio produced |
|---|---|---|
| 5 980 chars | 37.6 s | ~5.8 min |
| 20 000 chars | 128.3 s | 19.2 min |
| 62 000 chars | **245 s** | 59.6 min — a full scene |

**One request works and is still the wrong shape.** A 62 000-character
narration is a four-minute all-or-nothing bet against a reverse-engineered
endpoint: a dropped socket at 3:59 throws away all of it, and the retry of
D-094 then throws away four more minutes. So a line is **split at 9 000
characters** — roughly 520 s of speech and about a minute of generation, a unit
of work worth retrying on its own.

Nine thousand is not ours. The author's own `setuptts` speaks to this same
service in production and settled on 9 200–10 500 after real use; this is the
conservative end of that range. The rest of that program's design was read and
deliberately *not* copied: it splits because it drives the Python library
directly and must respect a websocket payload limit, while we drive the CLI,
which does its own payload splitting and knows the real ceiling better than we
do. **Our limit exists to bound what one failure costs, not to satisfy a
protocol.**

- **The seam goes where a reader would pause** — paragraph, then sentence, then
  word, and a hard cut only for a single word longer than a chunk. Each piece
  is a separate request, so the seam between two pieces is audible.
- **Then the pieces are packed.** Cutting at every sentence is correct and
  useless: an hour of narration is fourteen hundred sentences, and fourteen
  hundred requests is worse than the one request this exists to avoid.
- **The join is byte concatenation**, because MPEG audio is a stream of
  self-describing frames with no container to fix up. No FFmpeg process, no
  transcode. Each piece after the first contributes its encoder's info frame,
  about 26 ms of silence at each seam — a breath, and cheaper than re-encoding
  an hour of speech to remove it. Nothing downstream is misled, because the
  duration that reaches the renderer is measured on the normalized artifact and
  never added up from parts (D-021).
- **Sequentially, not in parallel.** The audio pool already runs several scenes
  at once (D-044); speaking one line's pieces concurrently would multiply the
  request rate against a service that rate-limits, to save time the pool is
  already saving.

**It is also faster.** Measured: 27 000 characters split into three requests
took 66 s — 2.4 ms/char against 6.4 ms/char unsplit, a 2.6× speed-up. That was
not the goal and it is not the reason; it is why there is no trade-off to weigh.

**A line no scene could hold is refused before the first request.** At the
fastest speaking rate observed here — 17.3 characters per second — a scene's
hour is about 62 000 characters.

The rate is not a constant, and the spread matters: 20 000 characters of
flowing prose read at 17.3 chars/s, while **25 000 characters of short
sentences read at 11.9** (measured against the author's own project, whose
narration is clipped: every full stop is a pause). So the limit is built on the
*fast* end deliberately — it then refuses only what could not fit at **any**
rate. Built on the slow end it would refuse lines that would have fitted, which
is a wrong answer given confidently, and worse than the seven wasted minutes it
saves. Anything under the limit that still overruns is caught downstream on its
measured duration, which is the number that governs (D-021). Today a 100 000-character script is spoken for eleven minutes,
normalized by two FFmpeg passes, and *then* refused for its measured duration
of 1.6 hours. The limit is derived from `MAX_SCENE_SECONDS` rather than typed in
beside it, and it is deliberately generous — a line slowed with `--rate -50%`
can still pass here and be caught downstream on what it actually measures. This
is for the line nobody could have meant.

**One correction to D-094, from the same source.** `NoAudioReceived` was
classified as always permanent. `setuptts` retries it at full size once before
re-splitting, which is production evidence that the service also returns it when
a payload does not suit it. So it is permanent for a line of **200 characters or
fewer** — a caption an operator can read at a glance, where "there are no words
in this" is verifiable — and transient above that. A row of punctuation is still
attempted exactly once.

Tested three ways: the splitter's contract (no piece over the limit, no word
lost or invented, cuts at sentence ends) as pure unit tests; the join and the
"one part fails, nothing is left behind" path offline through a fake `edge-tts`
in `edge_retry.rs`; and a real three-request narration against the live service
in `edge_live.rs`, where `ffprobe` has to read the joined file as one continuous
track of the expected length — a join that dropped or repeated a part misses
that by a whole part.

### D-096 — A probe that decodes is timed by what it decodes · Accepted

Found 2026-08-26 by rendering a six-hour film, which is the only way this was
ever going to be found. Six scenes of an hour each, `--jobs 4`. Every one of
the first four failed, at the same moment, with:

```
FAILED 004: no response after 30.0s
  ffprobe … -count_frames -i …/.seg-0003-….mp4.partial
```

Nothing was wrong with the segments. They were correct, complete, and about to
be thrown away by **their own verification**.

`probe_counting_frames` is D-041's gate: a container's frame count is a claim,
so we make `ffprobe` decode the file and report what is actually in it. That is
right, and it means the call's cost is *the length of the video*, not the size
of its header — while the timeout it was given, `DEFAULT_PROBE_TIMEOUT`, is the
one shared with probes that read a header and return.

Measured here: **18 000 frames took 15.4 s**, or 0.85 ms per frame. So a flat
30 s covers about 35 000 frames — nineteen minutes of 30 fps video — and an
hour-long scene needs 92 s on an idle machine. With four workers decoding at
once it needs several times that, which is why all four failed together rather
than one at a time.

So the ceiling is derived from the frames the call is about to read: **30 s plus
5 ms per frame**, capped at twenty minutes. Six times the measured rate, and the
multiple is contention, not padding — `--jobs 4` is the default shape of a
render, and four of these decodes share the same cores. A four-second segment
gains 600 ms it never uses; nothing else about the normal case changes.

Two things worth keeping from how this was found:

- **`scene.rs` is the only place that counts frames.** The film's own assertion
  (`concat.rs`) and the reuse check (`film.rs`) both read headers, so they are
  as fast on a six-hour film as on a four-second one. One call site, one fix —
  worth confirming rather than assuming, because a second one would have failed
  the same way and only at the same scale.
- **A timeout that is a constant is a guess about the input.** The same mistake
  had already been made in the voice provider, where a flat 90 s refused a long
  paragraph (D-094), and it was made here independently. Where a call's cost
  scales with something we know, the ceiling should be computed from that
  something.

**n=500 is the design point, and this is the other axis.** Every gate and every
benchmark in this project so far measures many short scenes; nothing measured
one long one. A six-hour film is six scenes, and it failed at the first.

### D-097 — A pinned toolchain owns the targets, not the action that installed one · Accepted

Found 2026-08-27, cutting the first release. `.github/workflows/release.yml`
does the obvious correct thing:

```yaml
- uses: dtolnay/rust-toolchain@stable
  with:
    targets: ${{ matrix.target }}
```

and it does not work, because `rust-toolchain.toml` pins `1.94.0` (M0: "Rust
stable via rustup. Pin it."). **The pin wins.** The action installs `stable` and
adds the target to *that*; `cargo build` then runs the pinned 1.94.0, which has
only the host's standard library. The failure is:

```
error[E0463]: can't find crate for `std`
error: could not compile `serde_core` (lib)
```

**It is invisible on every native leg and fatal on the one cross-compiled leg.**
`aarch64-apple-darwin` on an arm64 runner and `x86_64-pc-windows-msvc` on a
Windows runner both build fine, because a toolchain always has its own host
target. Only `x86_64-apple-darwin` on an arm64 runner needs a target added, and
that is the only one that failed — three green legs and one red, from one
missing step.

So each job adds the target to whichever toolchain is *actually active*:

```yaml
- run: rustup target add ${{ matrix.target }}
```

The desktop job needs it twice over: a universal `.dmg` is both Mac slices
lipo'd together, so one of them is a cross compile however you look at it.

**This is why the first release never published.** D-087's `publish` job refuses
to undraft below twelve assets, on the grounds that the installers verify a
checksum before they install and a half-populated release would fail on a live
download. That gate did its job perfectly: ten assets, no publish, a draft
nobody could install from. The gate was right and the diagnosis went to the
wrong place — the first run was read as "cancelled", which it was, and the
underlying failure was only visible when the same leg failed again on a run
nobody cancelled.

Two things to keep:

- **A matrix where only one leg cross-compiles is a matrix that tests one leg.**
  The rest are testing that a toolchain has its own host target.
- **`--clobber` on every upload is what made this cheap to fix.** The draft is
  reused and re-run, so a failed leg is re-run into the same release rather than
  needing the tag deleted and re-cut.

### D-098 — A release asset is named for the person downloading it · Accepted

Decided 2026-08-27, comparing this project's first release page against the
author's own `setuptts`. Theirs offers `SetupTTS-macOS.dmg` and
`SetupTTS-Windows-Installer.exe`. Ours offered
`spoonstill_0.1.0_universal.dmg` and
`still-v0.1.0-aarch64-apple-darwin.tar.gz`.

Both are accurate. Only one is answerable by someone standing in front of it:
the person choosing has a Mac, not a triple, and `aarch64-apple-darwin`
requires knowing what Apple called its own processors. The version is in the
tag, the page title and the app itself, so putting it in the filename three
more times buys nothing and costs the reader a scan.

So the CLI matrix carries a `pretty` name and Tauri's bundles are renamed on
the way out: `spoonstill-macOS.dmg`, `spoonstill-Windows-Installer.exe`,
`still-macOS-AppleSilicon.tar.gz`, `still-macOS-Intel.tar.gz`,
`still-Windows.zip`. `scripts/install.sh` and `install.ps1` follow, which is
the part that must not be forgotten — an installer that constructs the old name
404s against the new release.

**And the notes lead with the download.** They used to open with a
`curl | bash`, which is the right first line for a developer and the wrong one
for the person who came to the page because they wanted an app. The one-liner
is still there, under the thing that gets clicked.

### D-099 — Gatekeeper is a dialog whose brightest button deletes your download · Accepted

Found 2026-08-27, the first time the published `.dmg` was opened on the
author's own machine:

> **"spoonstill" Not Opened** — Apple could not verify "spoonstill" is free of
> malware that may harm your Mac or compromise your privacy.
> **[Move to Trash]** [Done]

The default, highlighted, blue button **deletes the thing that was just
downloaded**. That is what an unsigned app looks like on macOS now, and two
things about it are worth writing down.

**The advice everyone gives is wrong.** "Right-click the app > Open" is in our
README, in our release notes, and in every StackOverflow answer — and **Apple
removed it in macOS 15**. On 15 or later it does nothing at all. The path now is
System Settings > Privacy & Security > Open Anyway, or `xattr -dr
com.apple.quarantine`. Our own documentation was confidently telling operators
to do something that has not worked for two releases of the operating system.

**And the installer can simply prevent it.** Quarantine is an extended
attribute applied by the *downloading* program. A shell script the operator ran
deliberately can remove it, and then the app opens like any other. So
`scripts/install.sh` now installs the window as well as the CLI — verify the
checksum, mount the dmg, copy to `/Applications` (or `~/Applications` when that
needs no sudo), clear the attribute, detach. `SPOONSTILL_SKIP_APP=1` opts out.

This is not a way around code signing, and it does not pretend to be: it is the
operator's own machine, their own command, and their own decision, which is
precisely what Gatekeeper's dialog is asking for and doing badly. Signing and
notarization (M5) remove the question entirely.

### D-100 — A scene can be removed and moved, and nothing is ever deleted · Accepted

Decided 2026-08-27, from the author's own use: *"there is no option to delete
and move the scene from one place to another"*. The window could make a project
and fill it, and then do nothing to it but render. A photo imported twice
stayed twice. A scene that belonged third stayed eleventh.

Both are one gesture in every tool anyone has used, and the answer here was to
open Finder and rename eleven files by hand — the same twenty minutes D-080 was
written to delete, reappearing one step later in the workflow.

**The order is the numbers.** Under D-050's convention a scene is every file
sharing a numeric stem, and render order is the natural order of those numbers.
There is nowhere to record "this one is third now": the name *is* the position,
so moving a scene means renaming files. A `position:` column would be a second
source of truth for order, and a second thing to disagree with the folder.

Four rules, each with a test:

- **Only where the convention holds.** A project whose stills are `opening.jpg`
  has an order the operator arranged by hand; renumbering it would be rewriting
  something we did not write. It is refused, and the refusal says how to make
  the project editable. A manifest's order is its CSV's, and that file is the
  operator's (D-050).
- **Nothing is deleted, ever.** A removed scene's files move to `removed/`
  inside the project — which the folder scan never sees, because it reads one
  level and takes only files. The operator drags them back if it was a mistake.
  This program frequently holds the only copy of a photograph, and "delete" has
  to be a promise it can keep. Removing twice after a renumber can produce two
  different photographs both called `003.jpeg`; the second is kept beside the
  first rather than replacing it.
- **Renaming is two passes.** Renumbering in place means writing `001` while the
  old `001` is still there — which silently destroys a file on Unix and fails
  with an error on Windows (D-071 again: the platforms disagree and neither
  outcome is acceptable). Everything moves to a staging name first.
- **The whole scene moves together.** The still, its script and its recording
  share a stem and are renamed as a set. Renaming the image alone would
  unpair the narration and produce a scene that still renders, with the wrong
  voice on it — a wrong answer that looks like a right one.

`still remove DIR SCENE...` and `still move DIR SCENE POSITION` come first,
because the CLI is the complete control surface (D-010) and a window verb with
no command behind it does not exist. `remove` takes several scenes and applies
them **highest-numbered first**, since removing 002 renumbers everything after
it — otherwise `still remove p 002 005` would take 002 and then whatever landed
on 005, which is scene 6.

In the window the controls are quiet until the row is hovered, and Remove arms
before it fires: a modal `confirm()` blocks a webview outright, and one click
that renumbers a whole film is a click nobody meant. The arrows move by the
scene's position **in the film**, not its row in a filtered view — "move up"
while a filter hides the scene above would otherwise move it somewhere the
operator cannot see.

### D-101 — Every column is on screen at every window size · Accepted

Decided 2026-08-27, from the author's own screenshots: the scenes grid ran off
the right edge of the window, narration was cut mid-word with no ellipsis, and
*"you can not see the remove and all of the button at very end so the new user
will find difficulty in navigating around it"*. Reproduced in a browser
harness at the shipped default of 1180x820 — the table was 1458px wide inside a
1318px pane, which put Audio, Resolved and the entire arrange column outside the
window.

**The cause was `table-layout: auto`.** The browser sizes an auto table's
columns from their content, and one line of narration is content three thousand
pixels wide. The table grew, `.grid-wrap` scrolled sideways to accommodate it,
and everything past the narration left the screen. Nothing was broken in a way a
test could see: the DOM was complete, the data was right, the columns simply
were not where the operator was looking. `table-layout: fixed` makes the
declared widths the whole truth, and the sum is always the window.

The consequence to keep in mind when touching that grid: **a fixed table clips
nothing on its own.** Every cell now either ellipsizes or wraps, and one that
does neither will silently run under its neighbour — which is exactly how
`4.000 declared` ended up printed across the arrange buttons before the column
was measured against its own widest string.

What follows from "one screen", in the order things are given up as the window
narrows — and nothing in that order is a control:

| below | what goes |
|---|---|
| 1280px | the voice list's freeform note |
| 1180px | the path in the title bar |
| 1080px | the voice id; Remove becomes ✕, keeping the word as its tooltip and its accessible name |
| 1040px | the still's filename beside each line |
| 980px | tighter audio and resolved columns |
| 820px | the thumbnail; the voice list's gender and selection mark |

The floor is the window's own: `tauri.conf.json` says 900x600, and the
stylesheet said 1100x700 — a whole band of legal sizes nobody had looked at.
Verified with a layout audit (nothing clipped without an ellipsis, nothing past
the viewport, no unintended sideways scroller) across every screen at nine sizes
from 760x560 to 2560x1400, in both themes.

**Three things this changed that are not about width.**

The arrange controls were `opacity: 0` until the row was hovered (D-100). That
is why the author could not find them: you cannot hover a control you do not
know exists. They sit at 0.42 at rest and full ink under the pointer — still
quiet, now discoverable. D-100's reasoning was about not shouting, and this
keeps that; it was the invisibility that was wrong.

Narration is edited in a **textarea that grows to fit**, not a one-line input.
The grid shows one elided line because it is a review grid at 500 rows (D-051) —
but the moment the operator opens a line to read or change it, showing them two
thirds of their own sentence is the same defect as clipping it, and D-095 exists
precisely because a scene can hold an hour of narration.

The render pane had **two scroll regions**: the pane scrolled and the live list
capped itself at 340px inside it. The progress bar left the screen the moment
you looked at a scene, and the list showed eight rows of a five-hundred-scene
film. The card now fills the window and scrolls once.

Two smaller repairs found while measuring. `.row-actions` was a class in the
markup and in nothing else, so Settings' controls were inline-block elements in
normal flow with a select capped at 260px cutting its own voice name in half.
And a path is never trimmed (D-089), but a *heading* built from a folder name
now wraps rather than running off the centred screen.

---

### D-102 — The tag is the version, and a release carries no co-author · Accepted

Two defects of the same shape, both found 2026-08-27 by looking at the
repository the way a stranger sees it rather than the way the author builds it.

**The number was only ever in the tag.** `git tag v0.1.1` and `v0.1.2` both
published twelve assets built from a workspace whose `[workspace.package]`
version had said `0.1.0` since M0. So `still --version` answered `0.1.0` on
every release, and Tauri named its Windows bundle `spoonstill_0.1.0_x64_en-US`
before D-098's rename took the number back off the filename. Nothing failed;
the release simply lied about which build it was, which is the worst kind of
version bug because it surfaces in a bug report months later.

The rule: **the tag, `Cargo.toml` and `apps/desktop/tauri.conf.json` state one
version, and the release workflow refuses the job if they disagree.** The check
is the first step of the `draft` job — before the draft exists and before any
of the four build legs start — because failing at asset eleven of twelve is
D-097's failure mode again. Bumping a version is therefore a commit, and the
tag goes on that commit; a tag is never the first place a number appears.

**A tag can hold a commit the branch does not.** `v0.1.2` pointed at `4bce8ec`,
which carried a `Co-Authored-By: Claude` trailer. `master` carried `33e9408` —
the identical tree, re-committed without the trailer — so `git log` on the
branch was clean while GitHub's contributor list, which reads every commit the
repo can reach including through tags, showed two people. Re-committing does
not remove the old commit; only moving or deleting the ref that keeps it alive
does. The tag was moved onto `33e9408` (identical tree, verified by
`git rev-parse <c>^{tree}` on both) and force-pushed, orphaning the trailer.

The rule that follows: **`git log` on the branch is not the repository.**
Anything reachable from any ref is public — tags, and any branch pushed once
and forgotten. When a commit is rewritten to remove something, check what still
points at the original.

Recording the author's standing preference, which this file is the only place
that survives: **commits in this project carry no co-author or session
trailer.** The author is the only contributor and intends the history to say so.

---

### D-103 — The window looks for FFmpeg where the operator installed it · Accepted

Decided 2026-08-29, from a bug report of the plainest possible kind: *"I open
the application and open the project and it's not opening."*

The application opened. The project opened. `recent-projects.json` recorded the
folder, which only happens after `validate_project` has **succeeded**. What
came back was a project with zero scenes, so the window showed "Choose
photos…" — over a folder holding six photographs.

**The cause is that a macOS app launched from Finder does not get the
operator's `PATH`.** launchd hands a GUI process
`/usr/bin:/bin:/usr/sbin:/sbin`, and Homebrew is on none of it. `Tools::from_env`
returned the bare name `"ffprobe"`, the spawn found nothing, and every still
failed the D-052 probe. The same folder, validated in a terminal one second
later, reported six scenes and no problems — which is the signature of this
whole class of bug and the reason it survived every test we have: the test
suite runs under `cargo`, in a shell, and a shell has Homebrew on its `PATH`.

Three things were wrong, and all three are fixed.

**One — the search.** `Tools::from_env` now resolves each program to an
absolute path: `PATH` first, then the install prefixes of the package managers
the README already tells the operator to use — `/opt/homebrew/bin`,
`/usr/local/bin`, `/opt/local/bin` on macOS, winget's and Chocolatey's and
Scoop's shim directories on Windows. That list is short on purpose. It is not a
hunt for any FFmpeg on the disk; it is the set of directories a GUI process is
missing.

This is not the thing D-012 refuses. D-012 refuses *downloading* a build nobody
chose, because a render made against an unknown binary is not reproducible.
Finding the build the operator did choose is the opposite: resolving to an
absolute path is **more** reproducible than a bare name handed to whatever
`PATH` the process inherited, and the located path is what the diagnostics
bundle now records. When nothing is found the bare name is returned unchanged,
so `MediaError::BinaryMissing` still names what was tried.

**Two — a missing tool is one fact about the machine.** The probe is asked
`ready()` once, before any file is looked at, and a machine with no FFmpeg
produces one project-level `ToolingMissing` problem naming `brew install
ffmpeg`. Before, it produced one error per photograph, each apparently about a
different photograph and none about the thing the operator had to do. The rows
still resolve, so the window shows the film they have been building alongside
the one sentence that explains why it cannot render yet. D-002: a sentence now,
not a failure forty minutes in.

**Three — "Choose photos…" is for a folder with no photos.** The window chose
that screen from `scenes.length === 0`, which is also true of a project whose
every image failed to load — so it offered to add photos to a folder that was
full of them, and discarded the problem list that said why. `ProjectView` now
carries `empty`, decided in Rust (D-010): no scenes, **and** nothing wrong
beyond having none yet. Anything else goes to the grid, where the problems
panel is.

The general lesson, which is the one worth keeping: **a GUI process and a
terminal process are different environments, and every test we own runs in the
second one.** Anything reached by name rather than by path works in `cargo
test` and can fail in the shipped `.app`. Reach for binaries by absolute path.

---

### D-104 — Every binary this program spawns is located, not named · Accepted

Decided 2026-08-29, the same day as D-103 and for the same reason, because
D-103 fixed one binary and the window spawns three kinds.

D-103's closing line is *"reach for binaries by absolute path"*, and it was
applied to `ffmpeg` and `ffprobe` only. `Edge::from_env` still returned the
bare name `"edge-tts"`, and `Provider::install` still spawned bare `pipx`,
`brew` and `python3`. Under launchd's `/usr/bin:/bin:/usr/sbin:/sbin` that is
the identical failure one screen to the left:

- **The voice service reported itself missing on a machine that had it.**
  Measured here: `edge-tts` is at `/opt/homebrew/bin/edge-tts`, and
  `command -v edge-tts` under a launchd `PATH` finds nothing. The window's
  Voice screen showed "not installed" over a working installation, and D-094's
  pre-flight refused the render before the pool started — correctly, on a false
  premise.
- **The Install button that exists to fix that could not.** D-092 added it so
  the window would not print `pip install edge-tts` and send the operator to a
  terminal. Its three candidates were spawned by bare name, so all three
  failed to start and it reported *"`pipx` is not on this machine"* about a
  machine with pipx, brew and python3 on it. The one recovery path in the GUI
  was broken by the same cause as the problem it recovers from.
- **And an install that did work would still have been called a failure.**
  `install()` re-checks `self.availability()` afterwards, using the path
  resolved when the provider was built — which is stale by exactly the amount
  that matters, because the binary did not exist yet when `from_env` looked.

So: `Edge::from_env` uses `spoonstill_media::tools::locate`, every installer is
located before it is spawned, and after a successful install the tool is
**located again** before success is claimed.

**`locate` also searches the per-user prefixes now**, which is the other half.
`brew` writes to `/opt/homebrew/bin`; `pipx` and `pip --user` write under the
home directory — `~/.local/bin`, and `~/Library/Python/3.14/bin`, whose number
changes with Python and is therefore read from the disk rather than spelled
out (`subdirectories`). On Windows the same two are `~\.local\bin` and
`%APPDATA%\Python\Python3xx\Scripts`. This stays inside D-103's rule that
the list is short and is not a hunt: these are the directories the README and
the Install button themselves write to. A GUI process keeps `HOME` /
`USERPROFILE` even when it loses `PATH`, which is what makes them reachable.

**The bundle now records what the process could reach and why.** `edge tooling`
is the provider's own `availability()`, and **`PATH`** is the raw value this
process was handed. That one line settles every report of this shape: a window
launched from Finder shows four directories there and a terminal shows twenty,
and the difference *is* the bug. It is the field that would have answered
D-103 in one reading instead of a bug report saying "it's not opening".

The rule, stated so it covers the next one: **`Command::new` is never given a
bare name in this codebase.** `spoonstill_media::tools::locate` is the only
way a program name becomes something to spawn, and a name that resolves to
nothing is handed back unchanged so the error still says what was looked for.

### D-105 — A missing tool is something to press, not a sentence to read · Accepted

Decided 2026-08-29, from a screenshot of the Voice screen. It is the third
decision in three days about the same class of failure, and the first one about
what the operator is *shown* rather than about what the code looks up.

D-103 and D-104 fixed the finding. This fixes the telling. The report that
prompted it was a photograph of this line, in grey, on an otherwise empty
screen:

```
`edge-tts` is not on this machine. Install it with `pip install edge-tts`
(or `brew install edge-tts`), press Install in Settings, or point
SPOONSTILL_EDGE_TTS at it.
```

Four instructions. Three need a terminal. One is an environment variable. The
fourth — the only one an operator could act on — pointed at a button one level
up, under Settings, on a different screen. Below it, a language filter, a
gender filter and a search box over nothing. **The screen reported a problem it
could have ended.**

That the machine in the screenshot *had* `edge-tts`, at
`/opt/homebrew/bin/edge-tts`, is D-103's bug and was already fixed in the tree —
the installed application was v0.1.0, built before either fix. Both facts
matter and they are different: **the fix existed and had not shipped**, which
is why D-102's version gate and a release are part of this decision rather than
a follow-up.

**A missing tool is now three fields, not one string.**
`spoonstill_core::Remedy` carries:

| field | who reads it | rule |
|---|---|---|
| `need` | the operator | one plain sentence, no paths, no flags, no backticks |
| `install` | the window | a tool id, when this program can fetch it |
| `detail` | the bundle, a disclosure | the path tried, the exit code, the last line of stderr |

`Display` still prints all of it, so a terminal loses nothing — it only loses
the *button*, and a terminal never had one. Nothing is deleted by this change:
`SPOONSTILL_EDGE_TTS` and the exact path moved from the operator's sentence
into `detail`, which is the half nobody has to read.

**The fix lives where the problem is shown.** `drawFix` in `app.js` is the one
component that draws a `Remedy`, and it appears on every screen that can report
a missing tool: the Voice screen, the Render screen, and both Settings cards.
It draws the sentence, an Install button, a Check again button, and the
technical half behind a disclosure — and on success it awaits a caller-supplied
reload, so **the screen the operator is already looking at becomes the screen
that works**. They never navigate anywhere to apply a fix.

**FFmpeg gets the button the voice service already had.** This is the part that
was plainly wrong before: `edge-tts` had an Install button since D-092, while
FFmpeg — which *every* render needs, and whose absence D-103 shows can present
as six broken photographs — offered the string `brew install ffmpeg` and no way
to run it. The screen reporting the more serious problem was the one that could
do less about it. `spoonstill_media::tools::install` mirrors
`Provider::install` exactly, D-104's re-location after a successful install
included.

**It is asked before it can fail, not after.** `ffmpeg_status` runs at project
open and its answer reaches `renderBlocker()`, so a machine without FFmpeg
shows a disabled Render button that explains itself next to the fix — D-089's
rule, applied to the one dependency every render has. A check that cannot run
is deliberately not a blocker: the render itself is still the authority.

**Where it lives, and why.** `spoonstill_app::tooling` owns "which programs
exist, how to check them, how to install them", because D-010 forbids the
window from reaching `spoonstill-media` at all and a webview should not know
which subsystem owns which binary. `still doctor` and `still doctor --install`
are the CLI half, written the same day, because *if the CLI cannot do it, it
does not exist.*

**This does not reopen D-012.** D-012 refuses downloading a build nobody chose,
silently, at render time. Every install here is the platform's own package
manager — Homebrew, MacPorts, winget, Chocolatey, Scoop, pipx, pip — run
because somebody pressed a button that said install. Nothing is fetched from
us, and a test asserts no installer in either table reaches for a URL.

**And the window's two silent failure modes are now tests.**
`apps/desktop/tests/ui_contract.rs` asserts that every `el("id")` the frontend
reaches for exists in the markup, and that every `invoke("cmd")` is registered
in `main.rs`. Both fail invisibly in a webview: a listener attached to `null`
throws and takes every listener declared after it, and the window opens looking
correct with half its controls dead. This change alone deleted two buttons and
renamed one command, so it created three chances to ship exactly that. It is
D-088's rule — *anything in the window a test cannot assert has to be clicked* —
paying for itself by making two more things assertable.

### D-106 — Subtitles are burned in, drawn by us, and chosen from six themes · Accepted

Decided 2026-08-29, asked for by the author: *an option to add subtitles or
not, in the project configuration, with multiple themes to select from.*

**This supersedes D-072's recorded default.** D-072 filed captions as
*"SRT in V1.1"* and reasoned that a sidecar file is nearly free because the
text is already present. That is still true and still worth doing one day. It
is not what was asked for: a sidecar `.srt` is a second file that most places a
film gets posted will ignore. What is on screen has to be *in the picture*.
D-072 stays Open for word-level karaoke, which still depends on provider word
boundaries and is unchanged by this.

#### Why we rasterize the text ourselves

FFmpeg burns subtitles two ways and **this machine's FFmpeg can do neither.**
Measured here on 2026-08-29 against ffmpeg 8.0.1:

```
$ ffmpeg -filters | grep -cE 'subtitles|drawtext'
0
$ ffmpeg -vf "drawtext=text=hi" ...
[AVFilterGraph] No such filter: 'drawtext'
```

Homebrew core split the formula. `brew install ffmpeg` — which is what
`README.md`, `scripts/install.sh`, `still doctor --install` and D-105's Install
button all reach for — now installs the **slim** build. Its own caveat says so:
*"ffmpeg-full includes additional tools and libraries that are not included in
the regular ffmpeg formula."* No `libass`, no `libfreetype`; therefore no
`subtitles` filter and no `drawtext` filter.

So a subtitle feature built on either one would be unavailable to every macOS
operator who followed our own installation instructions, and its remedy would
be *"install a second, much larger FFmpeg"*. Against that, the alternative is:

- **We draw the pixels and FFmpeg composites them with `overlay`**, which is a
  core filter present in every build there has ever been. `spoonstill-core`'s
  `captions` module owns the pure part — themes, cue splitting, timing — and
  `spoonstill_media::caption` owns the rasterizer: wrapping by real glyph
  metrics, outline by disc dilation, shadow by three box blurs, backdrop by a
  signed-distance rounded rectangle, composited source-over.
- **One new dependency, `fontdue`** — pure Rust, no `std` requirement, no
  `parallel` feature (a nested rayon pool would fight the D-076 worker budget
  for the cores that budget exists to ration). It rasterizes a glyph;
  everything above that is ours.
- **Three font weights are bundled**, Inter Regular / SemiBold / Bold, SIL Open
  Font License 1.1, in `crates/spoonstill-media/assets/fonts/`. Bundled rather
  than found on the machine, because a system font makes a theme mean something
  different on macOS than on Windows and makes the film depend on what the
  operator happens to have installed — the opposite of every other thing here,
  where the output is a function of the inputs (D-077).

The cost is a rasterizer we maintain and no complex-script shaping. The benefit
is that **subtitles work on the FFmpeg the operator already has**, which is the
difference between a feature and a feature request.

#### Where it goes in the filter graph

After the motion chain's own tail, untouched:

```
[0:v]<build_filter output>[vbase];
[vbase][2:v]overlay=x=0:y=<y>:enable='gte(t,S)*lt(t,E)'[vcap0];
[vcap0][3:v]overlay=...[v];
```

D-033's `setsar=1` is still the last filter before `format=yuv420p` and D-037's
colour pinning is still where it was. `overlay` inherits SAR and colour and
changes neither — **asserted, not assumed**: the segment still has to pass
`assert_matches_profile` before it is moved into place (D-041), so a subtitled
segment that drifted in pixel format, range, primaries, SAR or timescale never
reaches the concat. That existing gate is why this change needed no new
profile check.

Three details that are each a defect avoided:

- **The window is `gte(t,S)*lt(t,E)`, not `between(t,S,E)`.** `between` is
  closed at both ends, so two consecutive cues are both enabled on the frame
  they share — and since their bands differ in height, the earlier one shows
  under the later one for exactly one frame. Half-open windows tile.
- **A cue is one `rawvideo` input and one `overlay`, not one frame per frame.**
  `overlay`'s default `repeatlast` holds a single-frame overlay for as long as
  its `enable` window says. `MAX_CUES` bounds a scene at 60 so that D-095's
  hour-in-one-row cannot turn one scene into a thousand inputs; past that the
  text is cut into fewer, longer cues rather than truncated.
- **No path ever enters the filter graph.** The bands arrive as *inputs*, via
  the argument vector, and the graph carries only input indices and numbers. A
  filter graph is one string FFmpeg parses itself, where `:` separates options
  and `\` escapes — so `C:\Users\a b\x.rgba` is not a path it can be made to
  read, it is a syntax error with a drive letter in it. This is the tax every
  tool that burns ASS subtitles pays, and `subtitles=filename=` is exactly
  where it is paid. We do not pay it, and
  `no_path_ever_enters_the_filter_graph` is what keeps it that way. **This is
  the main reason the design is the same design on Windows and macOS** (D-071).

#### Where the words come from

A scene is captioned when it has words, and there are three ways it can:

1. an explicit `caption` — a new manifest column, and a new `SceneSpec` field;
2. failing that, the script it speaks (`AudioSource::Tts`'s own text), because
   requiring the operator to type the same sentence twice is the clerical work
   D-080 exists to refuse;
3. and nothing otherwise.

**A `.txt` beside a recording is now the caption, not a conflict.** In
convention mode `001.jpg` + `001.wav` + `001.txt` used to be D-020's
two-source case and was reported as an error. It is not one: the recording is
the narration and the writing beside it is *what the narration says*. D-020
still holds — the recording is still the only audio source — and nothing is
guessed. Without this, an operator who records their own voiceover could never
have subtitles at all, which is most of the author's own work. It turns an
error into a working scene, so no project that renders today can break.

`caption` is deliberately **outside** D-020's exactly-one rule, because it is
not a source of audio; it is what the viewer reads while the audio plays.

#### Two rules that came from rendering a real project

Both found on 2026-08-29 against the author's own ten-scene film, which is
narrated art with **words already drawn into the pictures**.

**A cue never ends on a dangling function word.** The character budget put a
break here: *"Those born with poor aptitude could pour twenty-four hours a day
into training and"* / *"still fall short…"* — leaving the viewer holding an
unfinished phrase across a cut. `carry_weak_endings` moves a trailing
conjunction, preposition or article onto the next cue. One word, never the only
word in a cue, and never a word carrying punctuation, because a word with a
comma after it is ending something rather than dangling. `WEAK_ENDINGS` is a
short closed list of function words on purpose: anything longer starts making
judgements about content, and anything cleverer needs a parser.

**Placement is an override, on both surfaces.** Whether a caption lands on
lettering that is already in the photograph is a fact about *those* pictures,
and the answer changes between batches — so it cannot only live in
`project.yaml`. Two things were wrong when a real project was pointed at this:
the window's position box drove **only the preview**, so an operator could move
the caption off their artwork, render, and get it back exactly where it was
(the D-091 defect, one screen along); and the command line had no override at
all, which breaks *if the CLI cannot do it, it does not exist*. Now
`--subtitle-position top` exists and the window sends what its box says.

On that film the difference is decisive: `boxed` at the top clears the
artwork's own lettering completely, while `classic` at the bottom is the worst
of the six, because with no plate the drawn-in text reads *through* the gaps
between the caption's lines.

#### The six themes, and "no subtitles" as one of the choices

`classic` (white, black edge, soft shadow, no box), `boxed` (rounded
translucent plate), `band` (full-width bar, flush to the edge), `card` (the one
light theme — near-black on warm off-white), `punch` (heavy yellow, thick edge,
for muted social video), `minimal` (small, light, shadow only). Six because
they span the actual decision, which is *how much of the photograph the caption
is allowed to cover*.

Every length in a theme is a **fraction of the frame**, never a pixel, so one
theme is one design at 720p, at 4K and in all three of D-070's aspects. A test
asserts every theme is legible by construction: no theme may have plain fill
with no outline, no shadow and no backdrop, because such a theme is unreadable
over some sky in some scene.

**Off is the default, and off is a row in the list.** Burning text into the
picture is irreversible — it is in the pixels, and an operator who did not want
it re-renders the whole film — while the reverse mistake costs one flag. So
`subtitles.enabled` defaults to `false`. And on the window's Subtitles screen
*"No subtitles"* is the first row of the same list the themes are in, not a
switch beside it: off is a real choice, and on this screen it is the usual one.

Surfaces, all of them overrides for one run — `project.yaml` is an input and
nothing writes to it (D-013):

```yaml
subtitles:
  enabled: true
  theme: boxed      # or classic, band, card, punch, minimal
  position: bottom  # or top
```

```
still subtitles                              # the themes, and what each is for
still render DIR --subtitles boxed           # on, this run, with this look
still render DIR --subtitle-position top     # off the artwork's own lettering
still render DIR --no-subtitles              # off, this run
```

The window's chooser previews a theme by calling the **renderer**
(`spoonstill_app::subtitles::preview`) and painting its RGBA into a canvas — not
by imitating it in CSS. A preview drawn a second way can be wrong about the one
thing it is for, which is legibility. The response is eight bytes of
little-endian width and height followed by straight RGBA, so the picture and its
shape cannot disagree.

#### The cache key, and what it costs

The subtitle spec joins the segment cache key (D-043): theme, placement, and
every cue's text and timing to the millisecond. Measured on the 100-scene
project: changing the theme misses every captioned scene and **hits every scene
that has no words**, because for those the bytes really are the same. A scene
with no cues emits no overlay chain at all, so its graph is byte-identical to
the one it had before this feature existed.

Cues are timed against the **narration** duration, not the padded segment
duration, so the caption leaves the screen when the speaking stops and D-022's
padding is silent in both senses. Time is shared out by character count, which
is the honest approximation available without word boundaries.

Cost, measured here 2026-08-29, cold cache, `--jobs 4`:

| project | without | with (`boxed`) | memory |
|---|---|---|---|
| 100 scenes, 960x540 | 21.6 s | 25.6 s | +6 MB |
| 40 scenes, 1920x1080 | 24.0 s | 25.2 s | +1 MB |

**About 5% at 1080p**, because x264 dominates and the caption is drawn once per
cue rather than once per frame. The first measurement was 3x worse until two
things were fixed: the font was being re-parsed for every cue, and `balance()`
re-measured the same glyphs a few hundred times per caption. Both are cached
now — `OnceLock` per weight, and a `(char, px)` advance map per call.

#### Found on the way, and fixed: a race in the atomic move

Rendering a hundred scenes that shared one recording failed with *"replacing
the existing file at …/cache/audio/file-….wav: No such file or directory"*.
`move_into_place` did `if to.exists() { remove_file(to)? }`, and two workers
that resolve to one cache entry both see it and both remove it; the loser fails
the whole render about a file it had just been told was there. `NotFound` on
that remove is now success, because what the call wants is for the destination
to be gone.

This is not a subtitle bug and it predates this change. It is recorded here
because of how it was found and because of who it hits: **five hundred scenes
narrated from one long recording is the ordinary case at the design point**,
not a contrived one, and no test in the suite ran enough identical scenes at
once to open the window.

### D-107 — A segment's cache key holds the narration it contains · Accepted

Found 2026-08-29 by audit and reproduced end to end before it was believed.

`segment_key` in `spoonstill_app::film` held the image content, the frame
count, the move, the geometry, the encoder settings and the subtitles — and
**not the narration**. The frame count is derived from the narration's
*duration*, so duration was standing in for identity. It is not identity: two
recordings of the same length are two different films.

The reproduction, which is now gate 4b of `scripts/m2-gates.sh`:

```
render 001.jpg + a 1.000s 440 Hz tone   -> film A
replace 001.wav with a 1.000s 880 Hz tone
render again                            -> "1 segment reused", film A byte for byte
```

The audio cache did its job perfectly — the new recording was hashed,
normalized and stored under a new key. The segment cache then ignored all of
it, because the only thing it took from the audio was a number that had not
changed. **The operator re-recorded a line, the tool reported success, and the
film contained the previous take.** Nothing warned, because from the renderer's
point of view nothing was wrong.

The fix is one field. `ResolvedAudio` now carries the `key` it was already
stored under — the content hash of the narration and its normalization profile,
computed in `audio::resolve` and previously thrown away — and `segment_key`
hashes it with everything else.

Three things worth keeping:

- **The identity was already computed.** The audio cache had the right answer
  the whole time; the segment cache simply never asked for it. A content-
  addressed cache that keys on a *derivative* of the content (a duration, a
  size, a modification time) is not content-addressed, and the failure is
  silent by construction — a hit is a hit.
- **The cache still works, and that is half the gate.** Putting the original
  recording back reuses the segment and reproduces the first film byte for
  byte. A fix that made every render a miss would pass "different audio,
  different film" and be a worse bug at n=500 than the one it replaced.
- **Gate 4 could not have caught this and gate 4b was written to fail first.**
  Gate 4 renders an unchanged project twice and asserts everything is reused,
  so it only ever exercised the hit. The new gate was run against the
  unfixed key and observed to fail before the fix was restored — per this
  file's own rule that a fixture encoding a hazard must be shown to encode it.

D-043 said "never key on a path". The other half, now written down: **key on
everything the artifact contains.** A segment contains its narration.

### D-108 — Work for one cache key is done once, not once per worker · Accepted

Found 2026-08-29 by audit, and measured before it was believed. Sixteen scenes
sharing one line, `--audio-jobs 8`, cold cache:

```
unique narrations needed:   1
times edge-tts was called:  8
```

`resolve` looked at the cache with `path.exists()`, found nothing, and did the
whole job. Eight workers ran that at once, all eight saw an empty cache, and
all eight spoke the same line to the same provider. Today that is eight times
the network. Once a metered provider lands (D-014) **it is eight times the
bill**, and the bill is the entire reason D-043 says never to key on a path.

This is not an exotic case. **Many scenes resolving to one cache entry is the
ordinary case at the design point** — one recording used throughout, a repeated
line, a folder of stills with no narration at all. It is also the same shape as
the `move_into_place` race found while building D-106.

The fix is a keyed single-flight: one `Mutex` per cache key, and the work is
done under it after a **second** look at the cache. Three details worth
keeping:

- **The fast path takes no lock.** The overwhelmingly common case is a hit, and
  a hit answers before the lock is ever reached. Measured at the design point:
  200 scenes with 200 distinct keys cold-render in 433 s against a 459 s
  baseline, at an identical 795 MB peak. Distinct keys never contend, so
  single-flight costs nothing where there is nothing to share.
- **Only the locked look evicts.** An artifact that will not probe may be one
  another worker is *mid-write*; deleting it there would be the check causing
  the corruption it exists to detect. Unlocked: report a miss. Locked: nobody
  else is writing, so a bad entry is genuinely bad and is removed.
- **In-process is the correct scope.** An `AudioCache` lives under one
  project's `.spoonstill/`, and two renders of one project are already refused
  by `render.lock`. The only writers that can collide are threads of this
  process, so an OS lock would be answering a question nothing asks.

The gate is offline and needs no voice service: stills with no script and no
recording all resolve to the same silence, which provokes the identical
collision with no network. It discriminates exactly — **8 narrations from
cache without the lock, 15 with it**, out of 16 — and was run against the
unlocked build and seen to fail first.

A note recorded because it surprised us while testing this: re-rendering a
200-scene project after deleting the audio cache produces a **byte-identical
video stream and a different audio stream**. D-077's determinism holds over
everything we compute; `edge-tts` does not return identical bytes for identical
text. The film is reproducible because the narration is *cached*, not because
the provider is deterministic — which is one more reason the cache is a
correctness feature and not an optimisation.

### D-109 — The segment cache keeps three generations, not all of them · Accepted

Found 2026-08-29 while testing D-107 against the author's own project folder,
and it is the kind of defect only real data shows. Nothing ever removed a
superseded segment, so a project accumulated **one dead generation per render,
forever**. Measured in `~/Downloads/RANDOM vidoe `:

```
source media          1.6 MB
.spoonstill/          159 MB
  segments            134 MB   52 files, 10 scenes — at most 10 can be live
  cache/audio          25 MB
```

Roughly 108 MB of that is files nothing will ever read again. The ratio is the
point: **the derived data is a hundred times the input**, and at the design
point of 500 scenes the same five generations are several gigabytes inside a
folder the operator thinks contains their photographs.

#### Why not simply keep what the film used

Because that punishes the loop this tool exists for. Choosing a subtitle theme
means rendering A, then B, then A again; so does choosing between two voices.
Keeping only the live set makes every flip a full re-encode — at 200 scenes,
seven and a half minutes to see a theme that was rendered five minutes ago.

So the rule keeps the live set **plus two spare generations**
(`SPARE_GENERATIONS`), evicted oldest-first by mtime. Both halves are asserted
together in gate 4d, because each is trivial to satisfy alone and worthless
without the other: bounded alone is "delete everything", free-to-flip alone is
"keep everything". Measured on the author's folder: 52 files and 127 MB became
30 files and 73 MB, and it stays at 30 through any number of themes.

The bound is a **count that scales with the project** rather than a byte
budget, because there is no honest number of megabytes to write down — three
times the film is a sentence an operator can check, and it is the same sentence
at four scenes and at five hundred.

#### The rules the sweep obeys

- **Only after the join succeeds.** A failed or cancelled render leaves the
  whole cache untouched, so the next attempt is as fast as this one would have
  been. Verified: a project with a corrupt still sweeps nothing.
- **Only files we wrote.** `is_our_segment` matches exactly the name
  `render_segments` emits — `seg-`, four digits, sixteen hex, `.mp4` — because
  a cache is not a licence to delete a stranger's file. A `holiday.mp4` an
  operator dropped in that folder survives, and there is a test listing nine
  near-misses that must not match.
- **Never fails a render.** The film is already made and already asserted. A
  cache that cannot be tidied is not a reason to withhold it, so every error in
  the sweep is logged and stepped over.
- **`--keep-cache` is the way out**, for an operator who would rather spend the
  disk than ever re-encode. The window inherits the sweeping default through
  `..defaults`, which is right: it is the surface where a project is iterated on
  hardest.

**Audio is deliberately not swept.** A segment is pure CPU to rebuild; a
narration is a network call and, under D-014, money. The asymmetry that makes
D-108 worth doing is the same one that makes sweeping audio a bad trade — and
at 25 MB against 134 MB it is not where the disk goes anyway.

This does not contradict D-100's "nothing is ever deleted". That rule is about
the **operator's own files**, which move to `removed/` and are never destroyed.
A segment is derived data this program wrote, reproducible from the still, the
narration and the settings. The distinction is worth keeping sharp: we delete
only what we can rebuild.

### D-110 — A segment is reused only at the length the plan asked for · Accepted

Found 2026-08-29 by audit. The reuse check in `render_one` probed the cached
file and asserted its `SegmentProfile` — which pins container, codec, profile,
level, pixel format, colour, geometry, frame rate and time base, and **says
nothing about length**. So a file carrying a segment's name and a segment's
shape was reused at any duration, and the frame count reported for it was
`plan.frames`: the planned number, which nothing had checked against the file.

Reproduced by putting a 60-frame segment under a 240-frame segment's cache
name — every profile field identical, four times the wrong length.

#### The consequence was not a wrong film, and was worse to live with

The audit that found this predicted a wrong-length film. It is not what
happens: D-041's assertion on the joined film compares the total against the
sum of the planned frame counts, so the short film is caught and the render
refuses. **No wrong film reaches an operator, and that half of the design
worked exactly as written.**

What happens instead is that the project becomes **permanently unrenderable**:

```
[  1/2] 002   240 frames  8.000s  (reused)     <- a number nothing verified
still: /tmp/.film.mp4.partial-17757-1.mp4 does not match the segment profile
  nb_frames: expected "300", found "120"
```

Every subsequent render repeats it. The progress line claims the bad scene
succeeded at its planned length, the failure blames a temporary file that no
longer exists, no scene is named, and the offending cache entry is left in
place because the profile assertion — the only thing that removes a bad
entry — passed. The one way out is deleting a folder the operator does not
know exists.

So this is filed as a defect of **reporting a number we did not check**, and
the fix restores the property that makes the cache recoverable: an entry that
cannot be shown to be right is removed and re-made.

#### The check is free, and deliberately reads the header

`is_the_planned_length` compares the video stream's declared `nb_frames` with
the plan. That value is **already in the probe the reuse check performs** — it
was being discarded. Measured at the design point: a fully cached 200-scene
re-render takes 12.35 s against a 12.2 s baseline.

It must stay a header read. D-096 is explicit that the reuse check has to be as
fast on six hours as on four seconds, and counting frames means decoding: at
200 scenes a decoding probe per scene would cost far more than the re-encode it
saves. The header is the right evidence for the same reason the film's own
assertion trusts it — this file was written by our muxer, and it only ever
*got* this name by passing the counted assertion in `scene.rs` (D-042).

Three cases, all tested: a declared count must match exactly, since the join
asserts an exact total and one frame out is out; with no declared count the
duration must be within one frame; with neither, the file is not reusable,
because re-rendering costs one scene and trusting it costs a wrong film.

Gate 4e asserts the **recovery**, not the refusal — the bad entry is dropped,
that one scene re-renders, the good one is still reused, and the film decodes
to exactly 300 frames. Run against the unguarded build first, where it fails.

### D-111 — The folder scan sorts, folds case, and refuses to guess · Accepted

Found 2026-08-29 by audit; both halves reproduced before either was changed.
`from_convention` keyed its groups on `(natural_key(stem), stem)` — the **raw**
spelling — and filled each slot with `get_or_insert_with` over an unsorted
`read_dir`. Three defects came out of those two lines.

#### A stem is one scene whatever its case

```
Shot.jpg + shot.wav
  ->  1 scene — 0 narrated, 0 supplied, 1 silent
      warn: "shot.wav" pairs with no image
```

The operator recorded a voiceover and got a **silent film** and a warning.
`natural_key` already lowercases, so the two stems sorted adjacently and then
became separate entries anyway, purely because the raw stem was also in the key.

The decisive evidence that folding is the intended rule is that the other half
of this convention already did it: `ingest::stem_of` lowercases, and its doc
comment says "folded for comparison". So `still add` paired these two files and
the folder scan did not — **one convention, implemented twice, disagreeing.**

#### Two files claiming one job is reported, not resolved quietly

```
001.jpg + 001.png + 001.wav + 001.mp3
  ->  1 scene, built from 001.png and 001.wav
      no problems
```

`get_or_insert_with` keeps the first and discards the rest in silence. "No
problems" is printed over a scene assembled from a still the operator never
chose. `ProblemKind::DuplicateId` had documented this exact case since M2 —
*"In convention mode this is `001.png` and `001.jpg`"* — but convention mode
collapses the group before validation can ever see two of anything, so the
problem it was written for could not be produced.

New `ProblemKind::AmbiguousScene { slot, candidates }`, at `Error`, named
against the scene:

```
error scene 001: 2 files claim to be this scene's image (001.jpg, 001.png)
                 — remove or rename all but one
```

An error rather than a warning, for the reason `ConflictingAudioSources`
already gives: guessing which one the operator meant is how a project renders
500 scenes of the wrong thing. Naming the candidates is the point — the
operator has to know *which file to delete*, not that something is wrong.

#### The choice must not depend on the filesystem

`read_dir` order is unspecified by std and differs between APFS, ext4 and NTFS,
and this loop decided which of two files a scene got. Honesty about what was
measured: on APFS here the winner was stable across creation orders — `001.png`
beat `001.jpg` either way — so **run-to-run nondeterminism was not observed on
this machine**, and the audit's claim of it is not confirmed. What is true is
that the order is arbitrary and unowned: nobody decided png beats jpg, and
D-071 says this code targets Windows too. The scan now sorts the folder before
pairing, so the choice is `001.jpg` — alphabetical, stable, and nameable in a
decision.

#### Cost

The scan reads the folder into a `Vec` and sorts it before pairing, rather than
streaming `read_dir`. At the design point that is 1 500 short strings for 500
scenes; the probes that follow dominate it entirely.

Three tests, each run against the unfixed code and seen to fail first: a folded
stem pairs, two candidates are reported by name with the scene attached and at
`Error`, and the file chosen does not depend on the order it was written.

### D-112 — The film's destination is contained by the same code as its inputs · Accepted

Found 2026-08-29 by audit and reproduced. `destination` tested the `output:`
setting **lexically** — reject an absolute path, reject any `..` component —
and then joined it onto the project root. A symlink is neither of those things:

```
project/escape -> /tmp/outside
project.yaml:  output: escape/film.mp4

  -> render succeeds, film written to /tmp/outside/film.mp4
```

This is a defect against the project's own stated rule, not a new requirement.
`CLAUDE.md` already says the `output:` setting "is *manifest data* and is held
to D-054 like every other path in the file — a project that renders itself into
`../../etc` is the thing containment exists to prevent." The rule was written;
only a weaker check was implemented.

`path_safety` has done this correctly since M2 — canonical, component-wise,
symlink-following, with `deepest_real_ancestor` closing the existence-oracle
leak. The reason `destination` did not use it is real rather than careless:
`resolve_within` answers for an **input**, so a path that does not exist is
`PathError::Missing`, and a destination normally does not exist.

So the containment decision is factored into `resolve_contained`, which returns
the resolved path *and* whether it exists. `resolve_within` keeps its meaning
(absent is an error); the new `resolve_destination_within` returns the path
either way. **Reading a file and writing one now cannot drift into two ideas of
what "inside the project" means** — the rule is one function, and the only
thing the callers disagree about is whether absence is a failure.

Four behaviours held fixed, each tested:

- **`--out` is still honoured wherever it points.** It is an argument the
  operator typed for this run; the setting is data in a file that may have come
  from somewhere else. That asymmetry is the whole of D-054.
- **A symlink that stays inside resolves to where it really points.** The
  output is reported as `project/real/film.mp4`, not `project/here/film.mp4`,
  because the operator's spelling is a request and not an address.
- **A nested destination that does not exist yet still works**
  (`renders/2026/film.mp4`), which is the case `resolve_within` could not have
  served.
- **D-089 survives.** A project folder whose name ends in a space renders, and
  the path is not trimmed anywhere in the new route.

One behaviour deliberately changed: `destination` now requires the project root
to **exist**, because containment is decided on canonical paths and a root that
is not there contains nothing. A test had been passing an invented
`/projects/demo`; it now uses a real folder. That test was not wrong about the
old code — it was checking a string join, which is what the old code did.

### D-113 — The render lock is the operating system's, not the file's · Accepted

Found 2026-08-29 by audit, reproduced end to end 2026-08-30. The lock was a
file whose *existence* was the lock: `create_new` to take it, `remove_file` on
`Drop` to release it, and `--force` to overwrite it. Every one of those three
is wrong, and they compound.

Demonstrated with two real renders of one 24-scene project:

```
A: still render …               -> takes the lock, pid 43746
B: still render … --force       -> takes it anyway, while A is running
   both alive at once, writing the same segment paths
B finishes, its Drop removes the shared file
   -> the project is now UNLOCKED with A still rendering
C: still render …               -> not refused. three renders, one project.
```

`--force` could not tell the case it existed for — "a machine that lost power
leaves a lock behind" — from the case the lock exists to prevent. And release
was unlinking a *shared path*, so whichever run finished first unlocked the
other.

#### The fix is to stop inventing a lock

`std::fs::File::try_lock` is stable as of Rust 1.89 and this workspace pins
1.94, so the kernel's own advisory lock is available with no dependency and no
`unsafe`. `render.lock` is opened and locked; **the file's existence carries no
authority and it is never deleted.**

Both defects disappear rather than being patched:

- **A lock cannot go stale.** The operating system releases it when the holding
  process dies — crash, `kill`, or power loss. Verified: kill the holder, and
  the next render succeeds *with the lock file still on disk and no flag*.
- **One run cannot unlock another.** Releasing is closing a handle, not
  unlinking a path two runs share.

#### `--force` is kept and no longer overrides

There is nothing left for it to rescue: the crashed-run case is now automatic,
and a refusal from the kernel means a live process is holding the lock. So
`--force` is accepted, does not override, and says why:

```
another render is working on this project (pid 45241)
--force cannot take a lock a running render holds. Wait for it, or stop it.
```

Kept rather than removed so scripts do not break, and answered with a sentence
rather than silence so an operator who reaches for it learns what changed. The
flag's help text says the same. **This is strictly better in both directions:
safer against the case that corrupts a film, and more convenient in the case
the flag was invented for.**

#### Two tests had encoded the bug

Worth recording, because it is the second time in this audit that the thing
asserting correctness was asserting the defect:

- `a_second_render_of_one_project_is_refused_until_the_first_finishes` ended
  with `Lock::take(&root, true).expect("forced")` — **`--force` taking a lock a
  live run held, asserted as correct.**
- Gate 5 wrote `pid 999999` into the file and required a refusal. Under the new
  design that file is not a lock, and requiring a refusal was requiring the tool
  to stay stuck after a run the machine lost.

Gate 5 now holds a **real** lock — a render running in the background, waited
for rather than slept past — and asserts both that a second render is refused
and that `--force` is refused too. Run against a build where `--force` still
overrode, it fails.

### D-114 — Output geometry has a ceiling, and it is derived from the level table · Accepted

Found 2026-08-30 by audit. `OutputSpec::new` had no upper bound and unchecked
arithmetic, and it is reachable from three places that all take operator input:
`project.yaml`'s `short_edge`, `still render-scene --short-edge`, and the
window's `subtitle_preview` IPC command, whose `short_edge: u32` arrives
straight from the webview.

Measured, in both profiles:

```
debug    short_edge 4294967292 -> panic: attempt to multiply with overflow
release  short_edge 4294967292 -> 3340530112x4294967292, "no problems"
release  short_edge 1431655776 -> prescale canvas 3340530176x32
release  short_edge 90000      -> 160000x90000, a 57 GB RGBA frame
```

Three different failures from one missing bound. The middle one is the nastiest:
both output dimensions look plausible while the **prescale height has wrapped to
32**, so the filter graph gets a canvas that is not remotely the shape the rest
of the pipeline assumes. And `still validate` reported *no problems* for all
three, which is the part that matters — the whole promise of `validate` is that
it says everything that is wrong before anything is rendered.

#### The ceiling is 36 864 macroblocks, and it was not chosen

`spoonstill_media::profile::LEVELS` tops out at H.264 level 5.2, `MaxFS`
36 864 — and past the table `h264_level` returns 52 **regardless**. So a larger
frame is not merely big: it gets *labelled* level 5.2 while being something no
5.2 decoder can play, and D-041's assertion then cheerfully confirms the label
we wrote ourselves. The segment profile has to be a true description of the
file, so the ceiling is the largest frame that description stays true for.

4K UHD is 32 400 macroblocks and renders. 8K is 129 600 and is refused. That is
a real restriction and it is the honest one: we cannot currently label 8K.

`MAX_MACROBLOCKS` lives in `spoonstill-core`, which depends on nothing and so
cannot import the level table it comes from. A test in `spoonstill-media`
asserts the two still agree, because the drift would be silent in exactly the
way described above. The table moved to module scope so that test reads the
same array the function does rather than a copy of it.

With the cap in place the prescale multiply cannot overflow — an accepted edge
is far below `u32::MAX / 3` — so `prescale_width`/`prescale_height` keep their
plain multiplication and document the invariant, with a test that walks down to
the largest frame each aspect admits and checks it there. **The invariant
belongs to the constructor, which is why the constructor exists.**

#### A message that was already wrong, made visible

`ProblemKind::UnusableSetting` renders `` `field`: "value" is not {expected} ``,
and `settings.rs` was handing it the whole `GeometryError` `Display` — itself a
complete sentence. With the old short reasons that read badly; with a longer one
it read as nonsense:

```
`short_edge`: "4294967292 at 16:9, 30 fps" is not short edge 4294967292 is not a size this renders …
```

Fixed rather than left, since this change is what exposed it. Every `reason` is
now a **noun phrase**, so it reads correctly both alone ("short edge 1081 is not
even — H.264 4:2:0 cannot represent odd dimensions") and after a caller's own
subject; `GeometryError::reason()` hands over the phrase without the subject.
That also removed a `Box::leak` per malformed setting.

### D-115 — The window supervises its own render, and a killed run leaves no litter · Accepted

Found 2026-08-30 by audit. Three separate claims, and testing them changed what
the fix should be — one of the three does not happen.

#### What does not happen: FFmpeg is not orphaned

The audit predicted that killing the parent leaves FFmpeg running. Measured on
a four-worker render, SIGKILL on the parent:

```
ffmpeg processes before kill: 4
2s after:                     0
```

`command::spawn` gives every child piped stdin, stdout **and** stderr, so when
the parent dies the pipes close and FFmpeg exits on its own. That is incidental
rather than designed, but it is real, and it is the difference between "the
window leaks CPU forever" and "the window leaves some files behind". Recorded
because the next person to read the audit will otherwise go looking for a
process leak that is not there.

#### What does happen: temporaries with nobody to rename them

The same kill left **four** `.seg-….partial-66085-N.mp4` files. `atomic` writes
beside-then-renames (D-042), so a run that dies mid-encode leaves one temporary
per scene in flight, and **nothing ever removed them** — not even D-109's
sweep, which deliberately matches only names a segment earned.

They are safe to remove at the sweep, and the reason is D-113: the render lock
is exclusive per project and held for the whole run, so no other render can own
a temporary in that folder, and this run's own have been renamed away by the
time the film is joined. Anything still called `.partial-` is litter from a run
that is gone. A stranger's dotfile is not touched — `.notes.txt` survives, and
there is a test listing five near-misses.

#### The slot the window could never release

`render_project` claimed the window's one render slot, then released it with a
statement placed **after** `handle.await…?`. A panic in the render thread makes
that join return `Err`, the `?` returns past the release, and the slot stays
claimed: every later render is refused with "a render is already running in this
window" until the application is restarted. It is now an `ActiveRender` guard,
which an early return cannot skip and which runs while a panic unwinds.

#### Closing the window now asks the render to stop

There was no `on_window_event` and no `RunEvent` handling at all: the CLI has
had a signal ladder since D-045 and the window had nothing, so closing it
mid-encode simply ended the process. `CloseRequested` now calls `cancel.request()`
on the active slot — the same thing the Cancel button does.

The close is deliberately **not** prevented. A window that refuses to shut is a
worse failure than a scene that has to be re-encoded, every artifact is written
beside-then-renamed so there is nothing half-written to find, and the lock
releases itself now (D-113). What the operator gets is an orderly stop instead
of an abrupt one, and — with the sweep above — no litter from it either.

**Honesty about coverage.** The guard and the cancel-through-the-slot mechanism
are unit tested, and both tests were run against the unfixed code and fail
there. The handler *firing on a real close during a real render* is not: that
needs GUI automation this project does not have. What was checked by hand is
that the window still opens and closes cleanly with the handler registered.
`ui_contract.rs` cannot help here — it asserts ids and commands, and this is
neither.

### D-116 — No cue is a flicker, and the count was never the guarantee · Accepted

Found 2026-08-30 by audit, which named one defect. Reproducing it found a
second, and the second is the real one.

**What the audit found.** The tail join was
`pieces.split_off(allowed.saturating_sub(1).max(1))`, and with `allowed == 1`
that `.max(1)` kept one piece *and* appended the joined rest — two cues where
one was allowed:

```
"one two three four five six seven. eight" in 1.0s  ->  0.872s + 0.128s
```

Also at 1.5s, which the audit did not try: `floor(1.5 / 0.9)` is still 1, so it
produced a **0.192s** cue — six frames at 30fps.

**What it missed, and it is the general case.** Capping the *count* was never a
guarantee about *duration*. Time is shared out by character count, with no
floor, so a perfectly legal number of cues can still contain a sliver:

```
"aaaa…(40) bbbb…(30). c" in 3.0s  ->  3 cues (allowed 3), the last 0.042s
```

One frame. The count was legal; the arithmetic that turned it into times was
not bounded at all.

So the fix is not the off-by-one. After the pieces are chosen, the worst
offender is merged into the neighbour it reads with — the one following it,
except at the end — and that repeats until every cue clears
[`MIN_CUE_SECONDS`] or only one remains. Each pass strictly reduces the count,
so it terminates. **A scene shorter than one readable cue still gets one cue of
the whole scene**, because that is the floor rather than a violation of it.

Honestly: the `.max(1)` removal is kept but is **not** what fixes this. Tested
by restoring each defect separately — with the off-by-one back and the merge
loop present, every test still passes, because merging subsumes it. It stays
because the tail join should honour `allowed` on its own terms rather than
leaving a contract violation for a later stage to clean up.

#### Why the existing test did not catch either

`a_short_scene_never_produces_a_flicker` asserts exactly the right property —
`cue.duration() >= MIN_CUE_SECONDS` — on a 2.0s scene whose text splits into
pieces of similar length. `allowed` is 2 there, so the off-by-one cannot fire,
and the pieces are balanced, so the missing floor cannot either. **A test that
checks the right thing on an input that cannot exhibit the bug is not
coverage**, and this is the third time in this audit that a test has quietly
been that.

Its replacement sweeps: six texts against 120 durations from 0.25s to 30s
across three budgets, asserting the minimum on every cue, that a sub-minimum
scene collapses to exactly one, and that the cues still tile the narration
exactly. That is the shape of test that finds the *next* one.

### D-117 — A shadow fits inside the canvas it is drawn on · Accepted

Found 2026-08-30 by audit, and the arithmetic it reported is exactly right:
`Mask::shadow` runs **three** box blurs, three passes of radius `r` spread
`3r`, and the band reserved `shadow_blur * 2`. The outermost ring of every soft
shadow was therefore cut off against the edge of its own canvas.

**Measured, because "can be clipped" and "is clipped" are different claims.**
Reading the maximum alpha on the outermost rows and columns of every theme:

```
1080p   classic  edges 0            band  edges 205  (its plate, blur = 0)
        card     bottom 2           boxed edges 172  (its plate, blur = 0)
        minimal  bottom 1           punch edges 0
```

So it is **real and it is small**: alpha 1–2 of 255 on one row. Nobody has ever
seen it. It is fixed anyway, for three reasons: the change is one constant, the
existing comment already claimed to reserve "the blur in every direction", and
the size of the error scales with `shadow_blur` — `card` is today's largest at
0.22 and a softer theme added later would clip visibly.

Two details worth keeping, because both explain the shape of the evidence:

- **Only the bottom edge ever clipped.** The shadow is offset down and to the
  right, so upward it needs `3r - offset` and downward `3r + offset`, while the
  margins reserve `outline + 2r` and `outline + offset + 2r`. Downward is
  therefore the binding direction, and it is short by exactly `r - outline` —
  which is why `classic` and `punch`, whose outlines are wide relative to their
  blur, showed nothing at all.
- **It clipped identically at 720p, 1080p and 4K.** That is not a coincidence,
  it is D-106's scale-invariance working: every length is a fraction of the
  frame, so the bug is a fraction too. The same property that makes one theme
  one design at every size makes one bug the same bug at every size.

The blur count is now `SHADOW_BLUR_PASSES`, used by both the blur loop and the
margin, because the disagreement between a `3` in one place and a `2` in the
other **is** this defect. Cost: `card`'s band grows 142 rows to 162 at 1080p,
about 150 KB per cue against the 780 MB per worker of §10.

The test asserts a property of the **output** rather than of the arithmetic —
*a theme that casts a shadow leaves the edge of its canvas transparent* —
because a test that recomputed the margin would agree with whatever the code
did. Themes with no shadow are excluded on purpose: `band` is full-frame-width
by design and `boxed` runs flush top and bottom, so their edges are *meant* to
carry ink. It runs at 720p and 1080p; 4K was measured during the investigation
and left out because a 3840-wide triple box blur is most of a minute in a debug
build.

### D-118 — A key that holds an operator's words is length-prefixed · Accepted

Found 2026-08-30 by audit. `segment_key` flattened the subtitle spec with
`key_fields().join("\u{1f}")` and handed the result to `fnv1a_fields`, which
separates its own fields with the byte `0x1f`.

**`fnv1a_fields` documented this as a precondition and was right to:**

> `0x1f` (ASCII unit separator) cannot occur in a path, a project id, or a hex
> digest, so it cannot be forged by the field contents.

That was **true when it was written**. Every caller then was a path, an id or a
digest. D-106 later began feeding it *subtitle text* — a `.txt` an operator
wrote — and nothing anywhere checks the precondition, so the violation is
silent and its symptom is a **cache hit**.

Constructed, not argued:

```
two cues:  ["alpha beta", "gamma"]
one cue:   ["alpha beta\u{1f}0.000>2.000:gamma"]

joined:    both "theme=classic\u{1f}place=bottom\u{1f}0.000>2.000:alpha beta\u{1f}0.000>2.000:gamma"
```

Two genuinely different films, one segment key, so one of them renders the
other's subtitles. And `0x1f` **is not whitespace** — measured:
`'\u{1f}'.is_whitespace()` is `false` — so `normalize` passes it through into
cue text untouched. A `.txt` exported from a tool that uses unit separators
carries it in.

The fix is `hash::fnv1a_prefixed`: every field preceded by its length, so a
boundary is *stated* rather than *looked for*, and no field content can imitate
one. `segment_key` uses it and passes each subtitle field separately rather
than pre-joining. Cost is eight bytes hashed per field.

#### What was deliberately not changed

`MotionSpec::seeded` also calls `fnv1a_fields`, and its `project_id` field is
the **folder's name** — operator text, so the documented precondition does not
hold there either. It is left alone, for a reason rather than by omission: a
collision there needs the field *after* the forged separator to line up, and
that field is a hex content digest, which cannot contain `0x1f`. So it is not
constructible. Against that, changing the seed would change **which Ken Burns
move every existing scene gets** — a visible change to films the author has
already made, to close a hole nobody can open. If motion's inputs ever stop
being a digest, this is the decision to revisit.

`fnv1a_fields` keeps its separator and gains a much louder doc comment: the
precondition is stated as a requirement, D-118 is named as what breaking it
looked like, and it points at the prefixed variant for any field that can hold
arbitrary bytes. Its test now asserts the collision *as a fact about the
function* — `fnv1a_fields(["a\x1fb"]) == fnv1a_fields(["a","b"])` — so the
limitation is pinned rather than remembered.

### D-119 — `move_into_place` is one rename, because Rust's already replaces · Accepted

Found 2026-08-30 by audit. `move_into_place` removed the destination and then
renamed over it, on a premise stated in its own doc comment:

> `fs::rename` replaces the destination silently on Unix but fails on Windows
> when it already exists, so the destination is removed first.

**The second half is not true of Rust.** It is true of the raw `MoveFile` API
and of several other languages, which is presumably where the belief came from.
Checked against the pinned toolchain's own source rather than from memory:
`std::fs::rename` is documented as *"Renames a file or directory to a new name,
replacing the original file if `to` already exists"*, and
`library/std/src/sys/fs/windows.rs` calls
`MoveFileExW(old, new, MOVEFILE_REPLACE_EXISTING)`. The only caveat in the
platform note concerns directories, and both sides here are files.

So the removal bought nothing on either platform, and cost two things:

- **A window with no artifact in it.** Between the unlink and the rename the
  destination did not exist. A crash there destroys the file that was already
  good — and this function moves the finished **film**, not only cache entries,
  so re-rendering over yesterday's film could lose yesterday's film.
- **A race that then had to be handled.** Two workers finish one cache entry
  whenever two scenes share a narration, which is the ordinary case at the
  design point. Both saw `exists()`, both unlinked, and the loser failed the
  whole render with "No such file or directory" about a file it had just been
  told was there. That was found and patched during D-106 — a patch for a
  problem the removal itself created.

Both disappear by deleting four lines. `rename` is last-writer-wins, so
concurrent finishers simply both succeed.

The test is the property rather than the implementation: one thread replaces a
file three hundred times while another asks nothing but *is it there*. Against
the old code it fails on the first replacement; there is no reasoning about
crash windows in it at all. The D-106 race test is **kept** with its rationale
rewritten — the race cannot arise now, but the two cases it exercises (moving
onto nothing, moving onto something) must both work either way.

Verified at the design point afterwards, because every artifact in the program
goes through this function: 200 scenes cold in 454 s at 795 MB peak, in line
with the 433-459 s of the runs before it.

### D-120 — A scene file is never seen holding part of a photograph · Accepted

Found 2026-08-30 by audit. `copy_in` opened the **real** destination with
`create_new` — the no-clobber check — and then filled it with `fs::copy`. So
the operator's own `001.jpg` existed, at its final name, holding incomplete
media for the whole of the copy.

**It is invisible on the machine this was written on.** `fs::copy` on APFS is a
copy-on-write clone and finishes in microseconds; a watcher polling from a shell
never caught it, and a `kill -9` after 250 ms was far too late. So the
reproduction was done on the filesystem the operator's media actually lives on —
a FAT32 volume, an external drive — where the same 400 MB copy takes 0.77 s:

```
still add … &  ; kill -9 after 0.3s
  ->  001.jpg   232783872 bytes   (the source is 419430400)
  still validate: source geometry 0x0 — the image is unreadable or truncated
```

A broken scene at a real scene name, a scene number consumed, and something for
the operator to find and delete. `validate` does catch it, which is the system
working; it is still a project the operator has to repair by hand after an
interruption they may not even have noticed.

The copy now goes to `atomic::partial_path` beside the destination, and the
name is claimed **after** it, then renamed over (D-119's rename replaces our own
empty claim atomically). Same kill, same volume, afterwards: no scene file at
all, one hidden `.partial`, and the retry takes `001` as if nothing had
happened.

#### What this does not achieve, stated plainly

The destination still exists as an **empty** file for the two syscalls between
the claim and the rename. Removing even that needs an atomic create-exclusive
rename, and the portable primitive for it is `hard_link` — which **FAT32 does
not support**: measured, `Operation not supported (os error 45)`, on the very
volume the defect matters on. So the choice is between refusing to overwrite and
appearing atomically, and refusing to overwrite is rule 2 of the module. An
empty file for two syscalls with no I/O between them is a different thing from
232 MB of a photograph for the length of a copy, and that is the honest claim.

#### Three tests that pass against the broken code

Worth recording, because it nearly went unnoticed. The obvious tests here —
complete content, no leftover temporary, an existing name still refused — all
pass **either way**, because they inspect the end state and the defect is a
window. That is exactly the trap D-116 named one decision earlier, walked into
while fixing something else.

The test that does distinguish watches: one thread copies 64 MB while another
asks only for the destination's *size*, and fails if it is ever between zero and
whole. Against the old order it catches 1 MB of 64 MB. It also had to be
written twice — the first version asserted the destination never *exists* during
the copy, which fails against the fix too, for the empty-claim reason above. The
assertion has to be the property the fix actually buys.

### D-121 — An interrupted renumber is finished or undone, never left · Accepted

Found 2026-08-30 by audit and reproduced end to end. `renumber` renames a
scene's files to `.arranging-…` and back in two passes, and every step is
`rename(..)?` — so an interruption returns early and leaves the files parked.
The parked names begin with a dot, and D-050's folder scan ignores dotfiles.

A 2000-scene project, `still remove` killed 120 ms in:

```
433 files parked, 434 scenes gone
still validate:  1566 scenes — no problems
```

**That last line is the defect.** Nothing was deleted, so D-100 held; but 434 of
the operator's photographs were invisible, `validate` said the project was
fine, and every arrange command then refused it — *"`.arranging-0002-jpg.jpg` is
not a numbered scene"* — so the one surface that could have repaired it was the
one that turned it away. There was no recovery command, and no message anywhere
said what had happened.

#### The journal is the filename

A parked file is now `.arranging-<from>-to-<wanted>.<ext>`, carrying both where
it came from and where it was going. That makes recovery decidable from the disk
alone, with no separate journal to keep in step:

- **Destination free → put it there.** After pass one every numbered name has
  been vacated, so this is always the branch for an interruption during pass
  two. It finishes the job.
- **Destination taken → put it back where it came from.** Its old name must be
  free, because pass one moved it away and pass two had not begun to fill
  anything in. That is the branch for pass one, and it is a rollback.
- **Neither free → leave it parked.** There is nothing safe to do, and leaving
  a file parked beats overwriting a photograph. `validate` still reports it.

`recover` runs at the top of `scenes`, so it happens **before the project is
read**, not merely before it is written — a folder is never *shown* to anyone
in the half-renamed state.

Names in the older format are recovered too. A folder damaged by the build that
shipped this bug has to be repairable by the build that fixes it; those names
carry only where the file came from, so the only safe reading is "put it back".
Verified on the actual damaged project from the reproduction: 433 files
restored, 1566 scenes back to 1999, `validate` clean, and all 2000 photographs
accounted for — 1999 in the project and one in `removed/`.

#### And `validate` no longer says nothing

`ProblemKind::InterruptedRename` is reported by the folder scan, counted
**before** the dotfile skip that made these invisible in the first place.
"1566 scenes — no problems" over a project that had 2000 is the most misleading
thing this program can say, and it is the same class as D-091: a screen showing
a true thing in a way that misleads is a defect of the same kind as a wrong
number.

The recovery tests build each state by hand rather than racing a kill — landing
the window took eight attempts at varying delays, and a test that reproduces one
run in eight is a test people learn to re-run.

### D-122 — The activity log is locked, and nothing in it can be run · Accepted

Found 2026-08-30 by audit. Two defects in `runs.csv`, the machine-wide activity
log — the file D-093 exists so that "what went wrong" can be answered without
knowing which folder it went wrong in.

#### It is written by several processes and had no lock

`append_line` rolled if large, asked `path.exists()` to decide whether to write
a header, then wrote the header, a newline, and the row as **three separate
writes**. Four concurrent `still render` processes into a fresh log:

```
when,level,scope,project,what happened,details,folder
when,level,scope,project,what happened,details,folder"2026-08-30T05:45:02.056Z","info",…
when,level,scope,project,what happened,details,folderwhen,level,scope,project,what happened,details,folder
```

Three headers, two of them welded onto other rows. Not a file any spreadsheet
can read, which is the one thing this file is for.

This is the case a per-project lock cannot cover: the writers are different
projects in different processes. Three changes, each removing one of the ways
that happened — the file's own lock held across deciding *and* writing;
"fresh" read from the **locked file's length** rather than `path.exists()`,
which was a check against one fact and an act on another; and one `write_all`
instead of three. Eight rounds of four concurrent renders afterwards: clean.

Still silent on failure, deliberately. A render must never fail because a
spreadsheet could not be written, so a lock that cannot be taken falls through
to the write rather than dropping the row.

#### RFC 4180 is about parsing, not about running

`field` quoted correctly and stopped there. Excel, LibreOffice and Numbers all
strip the quotes and *then* read a leading `=`, `+`, `-`, `@`, tab or carriage
return as the start of a formula — and this file exists to be opened in a
spreadsheet, from Settings > Activity log and `still diagnostics where`.

Reachable through the operator's own folder name, which is the `project`
column. Verified end to end: projects in folders called `=1+1`, `@SUM(A1:A9)`,
`-2+3` and `+1` put exactly that in the column. D-052 already says a name
someone chose is hostile input; a name someone *else* chose — a project folder
that arrived from elsewhere — more so.

A leading `'` now defuses those, and **numbers are left alone**: `-16.0` is a
loudness reading that belongs in the column as a number, and a value that parses
as `f64` cannot be a function call or a DDE payload. That is the test, not a
list of exceptions.

#### What the tests prove, and what they do not

The defusing test fails without the fix. The concurrency test fails against the
original `append_line` — but it does **not** isolate the lock: with only the
lock removed it still passes, because inside one process the length check and
the single write are enough. Recorded in the test itself. The lock is for the
case a unit test cannot reach, and the evidence for it is the eight-round
process-level reproduction, not the test.

### D-123 — An installer says why it failed, and a temporary cleans itself up · Accepted

Found 2026-08-30 by audit. Two small things, both in the "a missing tool is
something to press" territory D-105 opened.

#### Every failure was reported as an absence

`tools::install` matched `spawn().and_then(wait_until)` with
`Err(_) => "`{program}` is not on this machine"`. That expression can fail four
ways — `BinaryMissing`, `Spawn`, `Timeout`, `Cancelled` — and only the first one
means what the message says.

The realistic one is the timeout. `INSTALL_TIMEOUT` is 900 seconds and
`brew install ffmpeg` on a slow connection genuinely outlives it, so the
operator watched their package manager run for a quarter of an hour and was
then told it **was not on the machine** — a wrong diagnosis, not a vague one,
and one that sends them to install what they already have. D-105 exists because
a screen that reports a problem it could have ended is a defect; a screen that
reports the *wrong* problem is worse.

`describe_failure` now answers each case in its own terms, and the timeout's
answer says "it is installed" in as many words.

**Honesty about the reproduction.** The two reachable-in-a-test failures turn
out to be handled correctly already: a non-executable candidate is skipped by
`locate` (which is right), and an unexecutable one comes back through
`posix_spawn`'s shell fallback as `exited 126: cannot execute binary file`,
which the `Ok(finished)` arm reports accurately. The gap is `Timeout` and
`Cancelled`, and a 900-second timeout is not something a test waits for — so the
evidence is the code path plus a unit test over the four error values, not an
end-to-end run.

#### A cleanup loop is only reached on the paths somebody remembered

`Edge::speak` removed its part files in a loop placed **after**
`join_mp3(&parts, &audio)?`. A failed join returns past it, leaving every part
in the **audio cache** — which, unlike the segment directory, has nothing that
sweeps it (D-115). A failed `move_into_place` leaked the joined file the same
way. The in-loop failure path did clean up, which is what makes this the kind of
omission that survives review: the cleanup exists, it is just not on every exit.

It is a `Scratch` guard now, holding every temporary the call makes, removing
them all on `Drop`. The manual loops are gone rather than duplicated. The
finished audio is registered too: on success it has already been renamed away so
removing it is a no-op, and on a failed rename it is exactly what wants removing.

### D-124 — What is embedded in the binary is named in the binary · Accepted

Raised 2026-08-30 by audit. Three weights of Inter are compiled into every
build with `include_bytes!` — they exist because `brew install ffmpeg` ships
without `libass` and `libfreetype`, so spoonstill rasterizes subtitle text
itself (D-106). Their licence sat beside the `.ttf` files in the repository and
went **nowhere else**: the release archives were `tar -czf … still`, one file,
and `tauri.conf.json` declared no resources.

The SIL Open Font License is unusually direct about this, in condition 2:

> Original or Modified Versions of the Font Software may be bundled,
> redistributed and/or sold with any software, **provided that each copy
> contains the above copyright notice and this license.** These can be included
> either as stand-alone text files, human-readable headers or in the appropriate
> machine-readable metadata fields within text or binary files as long as those
> fields can be easily viewed by the user.

A published archive contained the font and not the licence. That is a plain
mismatch with a condition written in the licence's own words — not a legal
opinion, which this file is not the place for.

Three things now carry it, in increasing order of how hard they are to lose:

- `THIRD-PARTY-NOTICES.md` at the workspace root, in **both** release archives
  (staged beside the binary so an operator extracting into Downloads gets two
  files rather than a folder) and in the window's bundle resources.
- **`still licences`**, which prints the file `include_str!`'d into the binary.
  So every copy contains the notice *however it was obtained* — extracted,
  copied off another machine, or built here — which is the second form the OFL
  itself offers. The installers were left alone deliberately: they install only
  the binary, and the binary is now sufficient.
- A test that asserts the notices file contains the licence **read from the
  fonts' own `LICENSE-Inter.txt`**, rather than a copy of the words. Replace the
  fonts and it fails until the notice is updated. `include_bytes!` makes an
  obligation invisible at the call site; nothing else in this build would have
  noticed.

#### What is deliberately not claimed

The Rust dependencies are **not** reproduced. Most are MIT or Apache-2.0, both
of which ask for their notice on a binary distribution, and doing it honestly
means generating the roll-up from `Cargo.lock` at release time — `cargo-about`
is the usual tool — so that it cannot drift from what was actually linked.
Hand-writing it would produce a file that is wrong by the next `cargo update`.
The gap is written into `THIRD-PARTY-NOTICES.md` itself so the next person finds
it rather than assuming the file is complete.

`README.md`'s licence section said "Not yet chosen" and stopped, which read as
though nothing about licensing was settled. It now separates the two questions:
spoonstill's own code is still undecided (D-062), and the third-party material
inside it is answered.

### D-125 — A release is pinned, scoped, and checked by name · Accepted

Raised 2026-08-30 by audit, covering two things that both decide whether a
published release can be trusted.

#### Every action is a commit now

`actions/checkout@v4`, `dtolnay/rust-toolchain@stable`,
`Swatinem/rust-cache@v2` and — worst — `cargo-bins/cargo-binstall@**main**`.
A moving tag means the thing that builds and signs a release can change without
a commit here, and `@main` means it can change between two runs of the same
workflow. Every one is now a full commit SHA with the old ref beside it as a
comment. The SHAs were **resolved from GitHub**, not written from memory:

```
actions/checkout         11d5960a326750d5838078e36cf38b85af677262  # v4
Swatinem/rust-cache      6323deb102c322ba6fcbdcafc7e3dddab59af2b6  # v2
dtolnay/rust-toolchain   4360b52568e2003a75bf9bc1d59f33a8e3fc893c  # stable
cargo-bins/cargo-binstall 75b4bfae1b2c753a6806bbce6e6cb89b602de33c # v1.22.0
```

The Tauri CLI was `--version '^2'`, which resolves at release time, so two tags
cut a week apart could bundle with two different CLIs and nothing would record
which. It is `=2.11.4` now, matching the `tauri` 2.11.5 that `Cargo.lock` pins.
`cargo tauri build` gained `--locked` for the reason the CLI job already had it:
a release is built from the versions that were tested.

`permissions: contents: write` was declared once at the top and therefore
applied to every step in every job, including the ones that only compile. It is
`read` at the top now, and the four jobs that touch the release ask for `write`
themselves.

#### Twelve of the wrong files is still twelve

The publish gate was `[ "$count" -ge 12 ]`. That is satisfied by twelve assets
with the wrong names — a leg that uploaded a stale file from a re-run, or
exactly the rename D-098 warns about, where the release looks complete and every
installer 404s.

It asserts the **exact set** now, and then downloads every asset and verifies
every `.sha256` before undrafting. Finding a bad checksum here costs a re-run;
finding it in `install.sh` costs an operator a release that refuses to install.

The logic was run against four cases before being committed to a file nobody
can execute locally: the complete set publishes; a missing leg refuses; **twelve
files with one renamed refuses**, naming both the missing and the unexpected;
and an unexpected extra warns without withholding a release that is otherwise
complete.

#### The names are a contract, and nothing type-checked it

They live in three places — the workflow that builds them, the gate that
requires them, and the two installers that download them — and D-098 is the
record of what one-sided changes cost. `release_assets.rs` now asserts all
three agree, in both directions: every built asset is required by the gate with
its checksum, every name the gate requires is one that is built, and both
installers ask for names the release publishes. Verified by renaming an asset
in the workflow alone, where two of the three tests fail. It runs as its own CI
step, because a failure means the next tag publishes something that cannot be
installed.

### D-126 — A file is measured before it is read · Accepted

Raised 2026-08-30 by audit. Several reads pulled a whole file into memory with
no idea how big it was. The one that matters is the narration script, and it is
worth the number:

```
a 191 MB 001.txt
  still validate  ->  607 MB resident, and "no problems"
```

Three copies — the `Vec<u8>`, the `String`, and the trimmed clone — and the
verdict was that this is a **valid narrated scene**. D-095's refusal of a line
no scene could hold is real, but it lives in the provider and does not run until
render, so `validate` — whose entire promise is to say everything that is wrong
before anything is rendered — said nothing.

`MAX_SCRIPT_BYTES` is **derived, not chosen**, from limits that already exist:

```text
MAX_SCENE_SECONDS 3600 x SPEECH_CHARS_PER_SECOND 17.3 x 4 bytes/char = 249 120
```

rounded up to 256 KiB. A `.txt` past that is not a narration that happens to be
too long, it is a file that landed in the folder by mistake. It lives in
`spoonstill-core`, and the measurement it comes from lives in `spoonstill-tts`,
which core cannot see — so a test asserts the cap stays **above** every
speakable line and not absurdly above it. Same shape as D-114, and the same
reason: a drift here fails quietly in the worse direction, refusing a script the
provider would have spoken.

Afterwards the same folder validates in **14 MB** and names the problem.

`project.yaml` gained a plain 1 MiB ceiling, which is honestly labelled as a
round number rather than a derivation — a settings file has no natural length,
and the fixtures' are under 200 bytes.

#### And one function existed twice

`scene.rs` hashed a still with `std::fs::read` — the whole image — while
`film.rs` had already written a streaming loop for the same job. Two
implementations that must agree on a **cache key**: if they ever diverged,
`still render` and `still render-scene` would key the same scene differently and
neither would reuse the other's work. There is one now, in `spoonstill-media`,
and it streams. Verified afterwards that a real project still reuses all ten of
its segments and renders byte-identically.

#### Not done

The window's recent-projects and settings JSON are still read whole. They are
files this program writes itself, in its own config directory, so the hostile
input D-052 is about does not reach them the way it reaches a project folder.
Recorded rather than silently skipped.

### D-127 — The window's authority is the project it has open · Accepted

Raised 2026-08-30 by audit, which was careful to say this is **not exploitable
today** — the CSP is static, the frontend is local files, and there is no remote
content. It is defence in depth, and both halves are places where a rule this
codebase already wrote down was applied to some commands and not others.

#### The grant did not match its own comment

```rust
// Thumbnails are real files, so the webview has to be allowed to read
// them — and only them.
let _ = app.asset_protocol_scope().allow_directory(&root, false);
```

`allow_directory` is every recording, every script, `removed/` and
`.spoonstill/` besides — not "only them". It is now one `allow_file` per still
actually shown, which is what the sentence always claimed.

**The obvious fix is a trap, and it is worth writing down.** The audit suggested
revoking the previous project's scope. Tauri's scope keeps an allow list and a
*deny* list; `is_allowed` checks deny first and nothing removes from allow. So
`forbid_directory` on a project the operator navigated away from would block it
for the rest of the session, and reopening it would show a grid of broken
thumbnails. Checked in `tauri-2.11.5/src/scope/fs.rs` rather than assumed. What
is bounded instead is the *size* of each grant.

#### The rule was written for two commands and needed to cover six

`Session` carries this note, from D-086:

> the frontend must never hand a path to a command that opens something.
> `open_film` and `reveal_project` take no arguments at all.

Meanwhile `set_narration`, `add_media`, `remove_scene`, `move_scene` and
`preview_voice` took `root: String` straight from the webview — so the rule
covered the two commands that *open a file* and not the four that **rearrange
the operator's photographs**.

They go through `project_root` now, which returns the session's own root. The
page's value is **compared rather than ignored**, so a page that has drifted
onto a stale project is told so instead of quietly rearranging the one that
happens to be open. Canonical comparison, with a plain equality fallback, so
`./x` and `/tmp/x` are the same project.

#### Verification, and its limit

The `State` parameter is injected by Tauri, not sent from JavaScript, so
`app.js` needed no change — the pattern was already proven by `render_project`,
which mixes `State<Active>` with page arguments. The window was built and
launched on a real 10-scene project and opened clean with an empty log.

**Clicking Remove inside the window is still not covered**, and that needs the
GUI automation this project does not have. What replaces it is a contract test:
every one of the five commands must still be invoked *with* a `root`, because
Rust now compares it — and a page that stopped sending one would have every edit
refused, silently, which is the failure mode `ui_contract.rs` exists for. A
first attempt to check this with `grep` reported three of the five as broken;
they are invoked through an `arrange` helper, and the test reads the call site
properly.

### D-128 — Windows is a different shell and a different PATH · Accepted

Raised 2026-08-30 by audit. Two places where code written on macOS is wrong on
the other platform D-071 says is in scope — and which nothing has ever been run
on.

#### A paste-ready command that does not paste

`shell_quote` produced POSIX single quoting, `'...'` with `'\''` for an
embedded quote. The audit is right that the *invocation* is safe — it is an
argument vector, and `no_shell_strings.rs` enforces that structurally — this is
only the form shown to a human. But that form exists for one purpose: D-016's
"when a render fails at scene 147 the operator's first move is to run that exact
command in a terminal". A line that does not paste fails at exactly that moment.

PowerShell's single quotes are literal, which is right for a Windows path full
of backslashes, but an embedded quote is escaped by **doubling** it. So
`Dad's photos` — not an exotic filename — came out as `'Dad'\''s photos'`,
which PowerShell will not parse.

`posix_quote` and `windows_quote` now exist separately, selected by `cfg`, and
**both are compiled and tested on every platform**. That is the point rather
than a detail: a rule that only compiles on Windows is a rule nobody here
checks, and the test asserts among other things that the two never agree on
`Dad's photos`.

`cmd.exe` is deliberately not served. It has no single quoting at all, so no one
form can satisfy both shells; PowerShell is what the shipped installer is
written in and what a modern Windows terminal opens.

**The function had to be renamed.** `powershell_quote` tripped
`no_shell_strings.rs`, whose denylist contains that word. That guard is a blunt
source scan **on purpose** — its own header says so, because what it prevents is
somebody adding a second way to spawn a process years from now. The right
response to a false positive from a guard like that is to move, not to widen the
guard. It is `windows_quote`, and the doc comment names the shell freely because
`code_only` strips `//`-prefixed lines.

#### `-like "*$InstallDir*"` is not a PATH lookup

`install.ps1` decided whether its folder was already on PATH by substring. Two
failures, and the first one is silent:

```
PATH contains …\spoonstill\bin-old
InstallDir is  …\spoonstill\bin
  -> "already on PATH"  -> the real folder is never added
  -> `still` is not found after a successful install
```

And `-like` reads `[` and `]` in its pattern as a character class, so a user
folder containing either would never match itself. PATH is a list, and it is
split on `;`, trimmed, and compared entry by entry now.

`Copy-Item … -Force` straight onto the live `still.exe` also truncates it first,
so a copy that fails part-way leaves the operator with neither the build they
had nor the one they asked for. Both installers stage beside and then replace —
the same rule as D-119 and D-120.

#### What was run, and what was not

There is **no PowerShell on this machine**, so `install.ps1` is reviewed and not
executed. Said plainly rather than implied. What was run: the Unix installer's
staged replacement, where the old build keeps working while the new one is
staged and then swaps in one step; and the PATH comparison as a shell stand-in
for the same rule, which reproduces the `bin-old` false positive exactly. What
guards the rest is a test asserting the two wrong shapes have not come back —
the substring test and the in-place copy — because a file nothing can execute
here is a file that otherwise rots unwatched.

### D-129 — Eighteen warnings nobody read become one gate that fails · Accepted

Raised 2026-08-30 by audit. Its numbers reproduce exactly: 516 crates, **zero
vulnerabilities**, 18 warnings — 17 unmaintained and 1 unsound. The finding was
not that any of them is dangerous; it was that nothing in this repository
recorded having looked, and nothing would notice a nineteenth.

#### The triage, which is the actual work

The audit guessed "much of the GTK/glib group is Linux-only through Tauri". It
is, and here it is per target, from
`cargo tree --target <triple> -e normal -i <crate>`:

| | macOS | Windows | Linux |
|---|---|---|---|
| gtk, glib, atk, gdk\* (11 crates, incl. the one **unsound**) | absent | absent | present |
| proc-macro-error | absent | absent | present (build only) |
| unic-\* (5 crates) | present | present | present |
| ttf-parser | present | present | present |

Spoonstill ships macOS and Windows and no Linux build — five `target:` entries
in `release.yml`, none of them Linux. So **twelve of the eighteen, including the
only unsound advisory, are not in anything an operator can download.** That is
worth stating precisely rather than as a shrug: it is not that they do not
matter, it is that they do not reach a shipped binary *today*, and the day a
Linux target is added the file that says so is wrong until it is re-read.

Six do ship. Five `unic-*` reach the **window only**, through
`urlpattern -> tauri-utils`. The sixth is the one that describes a risk actually
carried: **`ttf-parser`, through `fontdue`, in the CLI and the window, on every
platform** — it draws every burned-in subtitle (D-106).

And there is nothing to upgrade to. crates.io's newest `ttf-parser` on
2026-08-30 is **0.25.1**, exactly what `Cargo.lock` holds: the crate is
unmaintained *at its latest release*. So the remediations are replacing
`fontdue` or vendoring, and neither is worth doing for an unmaintained flag with
no advisory behind it. What would change that — a vulnerability against it, or
`fontdue` itself going quiet — is what the review date exists to look for.

#### The list is only worth having if something reads it

`.cargo/audit.toml` carries the eighteen, each annotated with which shipped
target it reaches, and a review date. CI runs `cargo audit --deny warnings` in
its own job, because it is the one check here that can fail without a line of
our code changing — an advisory published this morning breaks it, which is the
point.

That inverts what the audit found: eighteen warnings nobody reads become
**zero** warnings and a build that stops on the nineteenth. Verified by removing
`ttf-parser`'s entry, where `--deny warnings` exits 1 and names it.

A test asserts the file keeps its shape — a review date, a bound, and a comment
beside every silenced id. An unexplained `RUSTSEC-` line in that file is exactly
what the file exists to prevent, and it also asserts CI still runs the thing,
because an ignore list nothing consults is decoration.

### D-130 — The caption rasterizer is measured, and then it is fast · Accepted

Raised 2026-08-30 by audit, which asked for benchmarks rather than asserting a
problem. The benchmark found one, and it is worse at 4K than anyone had guessed
— which matters now, because D-114 has just made 4K the largest size this
renders.

Milliseconds per cue, release build, one line of the author's own narration:

```
             1080p   4K
  classic     28.5   336.5
  boxed        1.3     4.6
  band         1.6     5.8
  card        33.9   238.1
  punch       41.3   604.3
  minimal     14.4   107.2
```

**Six hundred milliseconds for one cue.** The two themes that cost nothing —
`boxed` and `band` — are the two with no outline and no shadow, which names the
culprits exactly. And `punch` going 41 ms to 604 ms is 14.6x for a 4x area:
cost was growing with the **fourth power of the resolution**, because both hot
loops scale with the frame *and* with a radius that is itself a fraction of the
frame.

#### Both were the same shape of mistake

`Mask::dilate` built the disc as a list of ~pi*r^2 offsets and walked all of
them for every pixel: `O(area * r^2)`. But a disc is the union of its rows, and
each row is an interval — so dilating by a disc is the maximum over row offsets
of a **one-dimensional** dilation, and a 1-D dilation is `O(1)` per pixel with a
sliding-window maximum. `O(area * r)`.

`Mask::box_blur` was already separable but re-added all `2r+1` samples per
pixel. Consecutive windows differ by one sample at each end, so the sum carries
across: `O(area)` per pass.

Together:

```
             1080p          4K
  classic     28.5 ->  7.6   336.5 ->  47.3    7.1x
  card        33.9 ->  3.7   238.1 ->  19.1   12.5x
  punch       41.3 ->  9.7   604.3 ->  61.7    9.8x
  minimal     14.4 ->  1.8   107.2 ->   7.9   13.6x
```

At the design point — 500 scenes, two cues each, 4K, `punch` — that is ten
minutes of drawing text becoming one.

#### The output is identical, and that is the whole risk

An optimisation to a rasterizer is worthless if it changes a pixel, and nothing
about a shadow being one shade different would be noticed until two builds were
compared. So it is asserted three ways rather than reasoned about:

- The **original implementations are kept in the tests** as the definition of
  right, and the new ones are asserted equal to them across a dot, the four
  edges, a diagonal of greys, an empty band, and radii from 0 to 20 — including
  radii wider than the mask, where a window runs entirely off the data.
- The border rule is preserved deliberately: samples off the end count as zero
  and the divisor stays `2r+1`, so shadows fade into the border exactly as
  before. That is arguably not the *best* rule; changing it is a different
  decision from this one.
- End to end, a three-scene film rendered with `punch` — which uses both — is
  **byte-identical** before and after, with only those two functions swapped.

#### What the benchmark also settled

`CLAUDE.md` recorded the subtitle cost at 1080p as "about 5%". Re-measured on
the author's ten-scene project: `punch` 17.17 s against 16.79 s with subtitles
off, and `card` 16.36 s against 17.32 s — the heaviest theme came out *faster*
than none. **At 1080p the cost is now inside run-to-run noise**, and the honest
statement is that it cannot be measured end to end rather than that it is some
particular percentage. 4K is where the per-cue numbers still matter, and they
are recorded above.

`caption_bench.rs` is `#[ignore]`d, like the live TTS suite: it measures and
prints a table rather than asserting a threshold, because a timing assertion on
a shared CI runner is a test that fails for reasons that are not defects.

### D-131 — Coverage went where the defects were · Accepted

The last item of the 2026-08-30 audit, and the one that is a measurement rather
than a defect. Re-run after everything above, with `cargo llvm-cov`:

```
                                    audit    now
  TOTAL (lines)                    75.09%  77.35%
  crates/spoonstill-app/film.rs    42.91%  61.05%
  apps/desktop/src/main.rs         10.03%  18.10%
  crates/spoonstill-media/concat.rs 35.66%  35.66%
  crates/spoonstill-media/audio.rs  51.00%  51.00%
```

The shape of that is the point, and it is not a flattering-numbers exercise:
**the two files that moved are the two this work touched, and the two that did
not move are the two it never went near.** `film.rs` gained eighteen points
because D-107 through D-110, D-112 and D-115 all live there; the window gained
eight for the same reason. `concat.rs` and `media/audio.rs` are exactly where
they were, and saying so is more useful than an average.

`spoonstill-cli/main.rs` went **down**, 18.46% to 17.46%, because D-124 and
D-109 added surface — `still licences` and `--keep-cache` — whose printing paths
nothing exercises. A number that only moves upward is a number being managed.

#### The eight tests the audit asked for exist

It listed the cases that should have caught these defects in CI. All of them do
now, and each was run against the unfixed code first:

| asked for | where it lives |
|---|---|
| changed same-duration narration | gate 4b (D-107) |
| output symlink escape | `film.rs` (D-112) |
| duplicate convention files | `rows.rs` (D-111) |
| case-only stem pairing | `rows.rs` (D-111) |
| same cache key requested concurrently | gate 4c (D-108) |
| forced lock ownership | gate 5 (D-113) |
| close during render | `apps/desktop` (D-115) |
| filesystem failure during renumber | `arrange.rs` (D-121) |
| filesystem failure during copy | `ingest.rs` (D-120) |

#### What is deliberately still open

- **No coverage threshold in CI.** A number with a gate on it becomes a number
  people satisfy; the audit's own advice was to spend effort on orchestration,
  filesystem failure and shutdown rather than on more small pure-function tests,
  and a percentage gate pushes the other way.
- **The shell gates stay macOS-only.** Windows CI runs `cargo test --workspace`,
  which includes the integration suites that render real media through real
  FFmpeg — `segment_integrity`, `subtitles`, `motion_matrix`. What it does not
  run is `m1-gates.sh` and `m2-gates.sh`, which want `shasum`, `seq -w` and
  friends. That is the existing note in `ci.yml` — the Rust tests build their own
  fixtures "precisely so that this job does not need bash" — and it is still the
  right trade.
- **No GUI automation**, so D-115's close-during-render handler and D-127's
  bound commands are unit-tested at the seam and not clicked. Both decisions say
  so in their own text.

### D-132 — Windows is checked by compiling for Windows, not by reading the code · Accepted

Decided 2026-08-30, immediately after D-107..D-131, while checking that the
audit's own work was safe to publish. Two defects, both on the platform D-071
puts in scope and nothing here has ever run, and **both found the same way**:
by adding the target to the pinned toolchain and building against it.

```
rustup target add x86_64-pc-windows-msvc
RUSTFLAGS="-D warnings" cargo check --target x86_64-pc-windows-msvc --all-targets \
  -p spoonstill-core -p spoonstill-media -p spoonstill-state \
  -p spoonstill-tts -p spoonstill-app -p spoonstill-cli
```

That is thirty seconds of machine time and it is the check this project did not
have. `apps/desktop` is excluded because `tauri-winres` needs `llvm-rc` to
compile a Windows resource file and this machine has no such linker — a host
limitation, not a code one, and the reason the exclusion is by crate rather
than by silencing anything.

#### `posix_quote` was dead code on Windows, and CI denies warnings

D-128 split `shell_quote` into two dialects and put
`#[cfg_attr(not(windows), allow(dead_code))]` on `windows_quote` so the POSIX
machine everyone develops on would not warn about the half it never calls. The
symmetric exemption was never written. On Windows `shell_quote` never reaches
`posix_quote`, so the lib build has it as dead code — and `ci.yml` sets
`RUSTFLAGS: -D warnings` for every job.

**This had never failed because it had never been pushed**: `posix_quote` is
introduced by 82cae5e, which was still local. The next push would have failed
the Windows leg on a lint, in a commit whose own decision text says both
dialects are *"compiled and tested on every platform"* — true of the tests,
which name both functions directly, and false of the library.

The fix is one attribute and its reason. What makes it worth a decision is the
generalisation: **an exemption written for one platform makes a claim true on
one platform.** D-128 wanted both dialects compiled everywhere; that is now
what happens, and the command above is what says so.

#### `titleBarStyle: "Overlay"` is macOS-only, and 82px of the title bar knew it

`.titlebar` carried `padding: 0 12px 0 82px`, and the comment above it is
honest about why: *"the traffic lights are the system's, overlaid — hence the
82px."* Traffic lights are overlaid because `tauri.conf.json` asks for
`titleBarStyle: "Overlay"`.

Checked in the pinned runtime rather than assumed — `plan/tauri`, and
`tauri-runtime-wry/src/lib.rs:1123`:

```rust
  #[cfg(target_os = "macos")]
  fn title_bar_style(mut self, style: TitleBarStyle) -> Self {
```

So on Windows the setting is ignored **without a word**, the window keeps its
native title bar, and the 82px becomes an empty indent under a bar we did not
ask for, with the app mark floating in the middle of the strip. Exactly D-088's
shape: a macOS assumption that a webview accepts silently, on the one platform
nobody here can look at.

`app.js` now writes the platform onto the root element and the stylesheet gives
the padding back where there are no traffic lights to reserve it for. Three
things about that, each of which was a choice:

- **It is JavaScript, not an inline `<script>`.** The CSP is
  `script-src 'self'` (D-083), which forbids one. The cost is that the
  attribute is absent for the moment before the module runs.
- **The rule is written positively — `[data-os="windows"]`, not
  `:not([data-os="macos"])`.** The negated form matches during that moment, so
  every macOS launch would flash the mark under the traffic lights. **Absent
  means macOS**, which is what the file already assumed, so that machine is
  pixel-identical to before this decision.
- **Windows keeps its native title bar** rather than gaining a drawn one.
  Custom minimize/maximize/close buttons are a bigger change than the defect
  justifies, and a strip under a native title bar is an ordinary toolbar.

`ui_contract.rs` asserts all three — that `app.js` still sets the attribute,
that the stylesheet still has the Windows rule, and that the rule has not been
rewritten into its negated form. It fails against each of the three states it
forbids. It also needed `css_code_only`, a `/* … */` stripper, because the
first version of the test failed on the comment explaining which selector not
to use.

#### And a third, which only the Windows runner could find

The cross-compile above is a `cargo check`. It does not *run* anything, so it
caught both defects above and was blind to this one:
`display_quoting_survives_a_hostile_filename` asserted the POSIX escape
`'\''` against the output of `shell_quote`, which is the platform-dependent
one. On Windows that returns the doubled form and the test failed — on the
first Windows CI leg that ever executed it, because the dead-code lint above
had been failing the job before any test ran.

The dialects themselves were never the problem: D-128's own
`an_embedded_apostrophe_is_escaped_the_way_each_shell_reads_it` checks
`posix_quote` **and** `windows_quote` directly, on every platform, and it
passed throughout. What was missing is that a test going through the
*platform-dependent* function has to expect the platform's answer. It now
picks the escape with `cfg!(windows)` and asserts the whole quoted run rather
than the absence of a fragment — the old form would also have been satisfied by
a bug that simply dropped the dangerous text.

So the honest ordering is: **compiling for Windows catches lints and type
errors; only the Windows runner catches a wrong expectation.** Both are cheap,
and neither substitutes for the other.

#### The limit, stated

The title bar is still not *seen*. The compile is real, the contract test is
real, and the Windows job now runs the whole suite green — but what proves a
title bar is a Windows machine, and there is still not one, consistent with
D-071 and with D-131's note that GUI automation does not exist here. What has
changed is that compiling for Windows is written down as a pre-release step,
and that the Windows CI leg is known to have actually executed the test suite
rather than dying on a lint before it started.

### D-133 — The release page is five downloads and one list · Accepted

Decided 2026-08-30, from the author looking at the v0.1.4 release page and
saying that a lot of it *"might make a less technical person confuse"*.

Fourteen rows. Six of them were files nobody downloads:

```
spoonstill-macOS.dmg                    6.23 MB
spoonstill-macOS.dmg.sha256               87 B
spoonstill-Windows-Installer.exe        2.32 MB
spoonstill-Windows-Installer.exe.sha256   99 B
spoonstill-Windows-Installer.msi        3.47 MB
spoonstill-Windows-Installer.msi.sha256   99 B
…
```

A `.sha256` twin after every asset **doubles the list without answering the one
question the reader has**, which is *which file do I click*. And the `.msi` is
a second Windows installer for the same application — a question put to the
person downloading that they have no way to answer, and that D-098 says should
never have been theirs to answer.

Two changes, and one thing deliberately not changed.

#### One `SHA256SUMS.txt`, made where the verifying already happened

The build jobs still compute a `.sha256` per asset, and the publish gate still
verifies every one of them against the file it names before undrafting. **That
check is the one that catches a corrupted upload and it is untouched** — it
matters that the sum is computed by the job that built the binary rather than
by whatever last touched it. What changed is what survives: after verifying,
the gate concatenates the sums into `SHA256SUMS.txt`, uploads it, and deletes
the twins.

Order is load-bearing. The upload happens **before** the deletion, so a failure
between the two leaves a release with too many checksums rather than none.

Both installers read the one file and pick out their own line. A name that is
**not** in the list is a failure, not a skip — a check that passes by finding
nothing to check is not a check. `install.sh` greps the line and pipes it to
`shasum -c -`; `install.ps1` splits each line and compares the name exactly,
so `still-Windows.zip` cannot be satisfied by a longer name that ends in it.
Verified locally against a fake release: a good asset passes, a tampered one is
refused, and an unlisted one is refused.

#### The `.msi` is gone

Removed from `tauri.conf.json`'s bundle targets, from the workflow's collect
step, and from the release notes. NSIS's `.exe` installs the same application
and is the one a person double-clicks. An `.msi` is for deploying across an
estate, which is not this product — and building one nobody collects would be
a silent cost.

Result: **six rows instead of fourteen** — five downloads, one list, and the
two source archives GitHub attaches on its own and does not let anyone remove.

#### The publish job leaves the checkout, so it names the repository

Cutting v0.1.5 built all six binaries and then failed to publish them:

```
failed to run git: fatal: not a git repository (or any of the parent directories): .git
```

`gh` works out which repository it means by asking `git`, and the step
verifies checksums inside `mktemp -d` on purpose — a downloaded asset must not
be confusable with a built one. From that directory `git` has nothing to
answer with, and the first `gh release download` dies.

**The cause is older than this decision.** D-125 added the
download-and-verify block, and it had never executed: v0.1.4 was built from a
commit before it existed, and the gate it replaced counted assets without
fetching any. So the first tag that reached the new code was the first tag to
find the bug — with six green build legs above it, which is exactly the shape
that makes it look like the publish step is at fault rather than the working
directory.

`GH_REPO: ${{ github.repository }}` on the step is the fix: naming the
repository outright is what makes it independent of where it runs. A test
asserts both halves — that the step sets `GH_REPO`, **and** that it still uses
`mktemp -d`, because if the verification ever moves back into the checkout then
`GH_REPO` is no longer what makes this work and the test would otherwise sit
there claiming a reason that had stopped being true.

#### And the fold itself welded two assets together

v0.1.5 published, and its `SHA256SUMS.txt` had **four lines for five assets**:

```
fe227e6b…  still-Windows.zip2e5aade4…  still-macOS-AppleSilicon.tar.gz
```

The Windows packaging step wrote its checksum with `-NoNewline`, so the file
had no line terminator and `cat` ran the next asset's row onto the end of it.
Effect, with both binaries perfectly good and only the manifest wrong:
`still-Windows.zip` was no longer findable, so **`install.ps1` refused to
install at all**, and `still-macOS-AppleSilicon.tar.gz` matched a three-field
line and **failed its checksum** — the two most likely machines.

This is D-122's defect exactly — *"three header lines, two welded onto other
rows"* — in a different file, four decisions later. Writing a row without
terminating it is the same mistake whether the file is a log or a manifest.

It survived the publish job's own verification because that loop checks each
`.sha256` **individually**, and every one of them is valid alone. Only the
concatenation was broken, and nothing looked at the concatenation.

`-NoNewline` was not simply wrong, which is why it was there: dropping it makes
`Out-File` end the line with **CRLF**, and `shasum -c` on macOS would then hunt
for a filename ending in `\r`. The newline is written explicitly as `` `n ``.

Three defences, because the producer is a PowerShell step on a platform nobody
here runs: it writes an explicit newline; the fold uses `awk 1`, which
re-terminates every record, so a file arriving unterminated becomes its own
line rather than joining another; and the result is **counted and shaped** —
one line per asset, two fields each — before it is uploaded.

**Reproduced before fixing, and the reproduction needed the runner's locale.**
On macOS the glob folds case and `still-Windows.zip.sha256` sorts last, so
nothing follows it and the weld does not happen; under `LC_ALL=C`, which is the
Ubuntu runner, it sorts before `still-macOS-*` and the bug appears — four
lines, the same welded row. The fold produces five clean lines and the new
guard rejects the old output.

#### What did not change

The verification. It would have been easy to compute the sums in the publish
job from what it downloaded, which would have made this a smaller diff and
quietly deleted the property D-125 added: a checksum computed after the upload
proves the file matches itself. The sums are still made where the binary is.

`release_assets.rs` grew two tests, because all of this is spread across a
workflow, two shell scripts and a JSON file, none of which type-check against
each other. One asserts the gate still makes the file **and** still deletes the
twins — the second is the one that fails silently, since a release with the
extra files works perfectly and is merely the thing this decision removed. The
other asserts nothing anywhere mentions a `.msi`, in either direction: a dead
reference or a returning bundle target both fail it.

### D-134 — The README opens with a real render, and it is generated · Accepted

Decided 2026-08-30, from an outside read of the repository as a portfolio
piece: the architecture and the testing are stronger than the *presentation*,
and the first thing missing is that **nothing on the page shows the program
working**. A renderer whose README has no render is asking to be taken on
trust.

So `assets/demo/render.gif` sits under the badges: four scenes, fifteen
seconds, motion, cuts on the spoken line, and captions burned in.

Three things about it, each of which was the decision:

- **It is a real render.** `make demo` runs `still render --subtitles boxed`
  through the same filter chain, the same caption rasterizer and the same
  concat every operator gets. It is not a mock-up, and it cannot drift from
  what the program does without the command that rebuilds it failing.
- **The stills are generated, not photographed.** `scripts/gen-demo.py`
  describes them — a graded ground, a ring off centre, a few rules — the same
  way `gen-brand.py` describes the logo (D-079). The author's own photographs
  are their work rather than this repository's, and stock photography would be
  showing off someone else's picture. These show what the program *does to* a
  picture, which is the thing being demonstrated.
- **640px, 10fps, 96 colours, 3.4 MB.** Measured against 720px (6.2 MB, too
  heavy to sit at the top of a page) and 560px (2.0 MB, where the burned
  caption starts to break down). The caption is the one detail that has to
  survive the palette, because it is the feature the frame is proving.

The demo is deliberately **not** in `make gates` and not in CI. It needs the
network for the voice service, and a gate that fails because a provider is
having an afternoon is a gate people learn to ignore (the reasoning D-094
already applies to `tts-live`).

#### The two neighbouring suggestions that were not taken

The same outside read asked for a licence and a desktop end-to-end test. Both
are refused **here** because both are already decided elsewhere, and this file
outranks a review:

- **A licence is D-062 and it is the author's to choose.** It is entangled with
  the FFmpeg question — an LGPL build has to exist before the shipped binary's
  terms can be stated — and picking one to make a page look finished is exactly
  the kind of decision this file exists to stop being made in passing.
  `THIRD-PARTY-NOTICES.md` and `still licences` already cover what ships inside
  the binary (D-124); what is undecided is spoonstill's own terms, and the
  README says so.
- **A desktop end-to-end test is D-131's stated omission**, not an oversight.
  There is no GUI automation in this project, D-115 and D-127 are tested at the
  seam and say so in their own text, and `ui_contract.rs` covers the two joints
  that fail silently. Adding a single clicked flow would be the least valuable
  test in the suite and the most fragile.

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

### D-072 — Captions in M-scope? · Open, and its default is superseded by D-106

Default was: **SRT in V1.1**, not V1 — the text is already present so it is
nearly free. **That default no longer holds.** D-106 burns subtitles into the
picture, which is what was asked for and what a sidecar `.srt` cannot do at the
places a film gets posted. A sidecar file is still worth having one day and is
still nearly free; it is now an addition to D-106 rather than the plan.

What remains genuinely Open here is **word-level karaoke** via Edge TTS word
boundaries — a real differentiator for this genre, V1.1+, and dependent on
D-023's internal build. D-106's cue timings are proportional to character count
precisely because word boundaries are the thing we do not have, and a supplied
recording does not have them at all.

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

### D-135 — A lock whose failure is tolerated is a lock that was never taken · Accepted

Decided 2026-08-30, from the Windows CI leg failing on `master` at the commit
`v0.1.5` was cut from. The failing test was D-122's own:

```
test runs::tests::concurrent_writers_produce_one_header_and_whole_rows ... FAILED
assertion `left == right` failed: more than one header — two writers both
thought the file was new
```

**It is a race, not a regression.** `runs.rs` has not changed since D-122
landed, and the same commit range passed the Windows leg twice (runs
`33300757792`, `33301401098`) and failed it once (`33301841728`). Code that
passes twice and fails once is not code that broke; it is code that was always
wrong and usually got away with it.

**The mechanism, from the pinned toolchain's own source rather than memory.**
`append_line` opened the log with `.create(true).append(true)`. In
`library/std/src/sys/fs/windows.rs`, `get_access_mode` maps that to
`(false, _, true, None) => FILE_GENERIC_WRITE & !FILE_WRITE_DATA` — a handle
carrying neither `GENERIC_READ` nor `FILE_WRITE_DATA`. `File::lock` is
`LockFileEx`, which requires one of them, and `std::fs::File::lock`'s own
documentation says so in as many words:

> On Windows, locking a file will fail if the file is opened only for append.
> To lock a file, open it with one of `.read(true)`, `.read(true).append(true)`,
> or `.write(true)`.

So `file.lock()` returned `Err` on **every Windows write this program has ever
made**. D-122's cross-process guarantee — the one case a per-project lock cannot
cover — existed on macOS only, and shipped in v0.1.1 through v0.1.5.

**What actually hid it is the line above the lock**, not the lock:

```rust
// A poisoned or unsupported lock is not a reason to lose the row, so
// failure here falls through to the write rather than returning.
let locked = file.lock().is_ok();
```

That tolerance is right — dropping an operator's log row is worse than a rare
interleave — and it is also why a *permanent, platform-wide* failure produced no
signal. The rule this decision is named for: **a lock whose failure is
survivable must still be a lock whose failure is loud.** Tolerating a failure at
runtime is not the same as never asserting it can succeed.

The fix is one access right — `.read(true)` — extracted into
`open_for_locked_append` so that the new test exercises the handle the program
actually opens rather than a copy of it that can drift.

**D-122's own test comment was wrong, and that is the second finding.** It said:

> It does *not* isolate the lock: with the lock alone removed it still passes,
> because within one process the length check and the single `write_all` are
> enough.

Windows disproved it. With no effective lock, two threads both observe `len == 0`
between the open and the write and both write a header — the reasoning mistook a
check and a write *in sequence* for an atomic one. The test did detect the
missing lock; it just detected it only when the race lost, which is why it took
three CI runs to say so. The comment is corrected in place, because a false
claim about what a test covers is how the next person decides not to look.

**Every other file lock in the tree was checked, and D-113's is correct.** The
render lock in `film.rs` opens `.read(true).write(true)`, which carries
`GENERIC_READ`, so `LockFileEx` succeeds there. The serious lock — the one that
stops two renders corrupting one project — was never affected. It was checked
rather than assumed.

**Stated limit, and it is the same one D-132 records.** Compiling for Windows is
clean both before and after this change: the defect is a runtime access right,
not a type error. `the_log_handle_can_actually_be_locked` is deterministic
rather than racy, so the Windows runner will fail it every time against the old
open options instead of one time in three — but **this machine cannot prove
that**, because `flock` on macOS is happy with an append-only descriptor and the
test passes here either way. The proof is the Windows CI leg.

**Closed 2026-08-30.** Run `33323926750`: the Windows leg is green, and the log
shows `runs::tests::the_log_handle_can_actually_be_locked ... ok` on
`windows-2022`. That is the deterministic half — the handle the program really
opens can now be locked on the platform where it never could. The racy half
passed too, but one green run of a race proves nothing on its own and is not
what this rests on.

### D-136 — The installers are executed, not just shipped · Accepted

Decided 2026-08-30, from an outside read pointing out that `install.ps1` has
never run anywhere. That is exactly true, and worse than it sounds:

- It has never run on this machine, which has **no PowerShell** — D-128 says so
  in as many words and calls the file "reviewed, not run".
- It has never run in CI. Neither has `install.sh`.
- Both are the first thing a stranger executes, and both are executed by being
  **piped straight into a shell**.

So the one piece of this project that reaches a person first is the one piece
that had never been executed at all. Every other line here is covered by 469
tests; these two files were covered by reading.

**Two jobs in `ci.yml`, one per platform.** Each runs the script *from the
checkout* rather than from `raw.githubusercontent`, so a pull request tests its
own change instead of what is already on `master`.

macOS asserts four things, each of them a decision that had no runtime proof:

- the CLI is executable where the script said it put it;
- `still --version` carries the **released** tag — which is D-102 checked from
  the outside, on a downloaded artifact, rather than from the workspace
  manifest D-102 already pins;
- the window was installed;
- `com.apple.quarantine` is **gone** (D-099). That attribute surviving is the
  difference between the app opening and the operator meeting a dialog whose
  brightest button is *Move to Trash*.

Windows asserts the CLI runs and that its folder is on the user PATH **by
whole-entry comparison** — which is D-128's substring bug (`…\spoonstill\bin-old`
counting as `…\spoonstill\bin`, so the real folder was never added and `still`
was not found after an install that reported success) being executed for the
first time since it was fixed.

**These jobs depend on the outside world, deliberately.** They install from the
*latest published release*, so they fail if that release is missing or
malformed. That is the same bargain as the `audit` job (D-129): a check that can
go red without our code changing is a check that is watching something real.
D-098 and D-125 are both about a release that looks complete while every
installer 404s — the publish gate asserts the asset **names**, and nothing until
now asserted that downloading and verifying them actually works.

**Expectations were verified locally before pushing, not guessed** — D-132's
lesson is that compiling catches type errors while only a runner catches a wrong
expectation, and a CI assertion is nothing but an expectation. Checked here
first: `still --version` prints `still 0.1.5`, so matching on the tag with its
`v` stripped is right; `tauri.conf.json`'s `productName` is `spoonstill`, so the
bundle is `spoonstill.app`; and `gh release view --json tagName` returns
`v0.1.5`.

**One hazard is known and left to the runner to answer.** `install.ps1` sets
`$ErrorActionPreference = 'Stop'`, and PowerShell 7.4 defaults
`$PSNativeCommandUseErrorActionPreference` to true, which makes a *native*
command's non-zero exit throw. If `winget` is present on the runner and
`winget install Gyan.FFmpeg` fails, the script may die at a step that is meant
to be advisory — the FFmpeg install is explicitly best-effort, because D-062
and D-012 say FFmpeg is the operator's. If that happens it is a real defect for
real Windows users, not a CI artifact, and the fix belongs in `install.ps1`.
This is written down before the first run rather than after it, so that the
outcome is a test of the prediction.

**The first push of this job failed before any of it ran, and that is the more
useful half of the decision.** The workflow referenced `${{ runner.temp }}` in a
**job-level** `env:`. The `runner` context is not available there — only
`github`, `inputs`, `matrix`, `needs`, `secrets`, `strategy` and `vars` are — so
GitHub rejected the file whole. What that looks like is worth writing down,
because it is nothing like a test failure: **no jobs, no logs, no annotations**,
one red tick, and `gh run view --log-failed` answering *"log not found"*.

From which the rule: **CI cannot check its own syntax.** A workflow GitHub
refuses to parse runs zero jobs, so any validation step placed *inside* the
workflow is exactly the thing that does not execute. This check has to happen
before the push or it does not happen — which makes it the mirror image of
D-129, where the point was to move a check *into* CI.

So `make workflows` is a local target, wired into `make lint`. It fails loudly
when `actionlint` is absent rather than skipping quietly, because a gate that
passes by not looking is D-125's asset count all over again. It was verified the
way everything else here is: the defect was put back, the gate failed and named
the line and the context, and it passed once the defect was removed.

The sharpest detail is that **`actionlint` was already installed on this
machine** when the bad workflow was pushed. The tool was there; nothing ran it.
`.github/` was simply the corner of the tree that no gate covered — the same
observation D-125 made about asset names living in three files nothing
type-checked, one directory over.

**What this does not cover.** The `.msi` is gone (D-133), the DMG's contents are
checked by name and not launched, and no GUI is driven — D-131's stated omission
stands. `make workflows` runs `actionlint -shellcheck=`: the expression and
context analysis is the part that decides whether a workflow can run at all,
while `release.yml`'s four shellcheck notes (`ls | grep`, unquoted globs) are
style in scripts handling asset names we control, and failing the gate on them
would teach people to skip it. It also does not test the *published* one-liner,
which fetches from a mutable `master` URL; that is a separate question and it is
still open.

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

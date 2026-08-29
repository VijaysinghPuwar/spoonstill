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

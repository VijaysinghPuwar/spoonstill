# plan.md — build plan for `spoonstill`

**Last updated:** 2026-08-26.
**Authority:** `decisions.md` outranks this file. This file says *when* and
*how you know you are done*; `decisions.md` says *what* and *why*.

---

## 0. Where the project actually is

Be precise about this, because three documents have described work as if it
were underway.

| | |
|---|---|
| Application code written | M0, M1, M2, and M4's shell ahead of M3 — run `make gates` |
| Rust toolchain on this machine | **installed** — rustc/cargo 1.94.0, clippy, rustfmt (2026-08-26) |
| Version control | **initialised** 2026-08-26; planning docs committed before code |
| `plan/` contents | 10 read-only reference checkouts (~1.7 GB) + 3 retired planning docs |
| FFmpeg | 8.0.1 present, Homebrew GPL build — dev only, not shippable (D-062) |
| Node | v26.6.0 |
| Design questions settled | product, architecture, audio model, motion filter |
| Design questions open | D-072 only (captions, V1.1) — D-070/D-071 are Accepted |

The motion pipeline — historically the riskiest unknown — is now measured and
decided (`ffmpeg-findings.md`). The remaining risk is concentrated in state,
resume, and behaviour at n=500.

## 1. Sequencing principle

**Build the risky, measurable thing first; build the visible thing last.**

Each milestone is a *vertical* slice that runs end to end from the CLI. No
milestone is "the database layer" or "the UI layer". Every one produces
something you can execute and check with `ffprobe`.

Two rules that decide arguments about ordering:

1. **If the CLI cannot do it, it does not exist.** The CLI is the permanent
   complete control surface, not a stepping stone to the GUI (D-010).
2. **A milestone is complete when its exit gates pass, not when the code is
   written.** Every gate below is a command with an observable result.

```
M0  toolchain + skeleton      ── 1 day    ── unblocks everything
M1  one scene, end to end     ── 1 week   ── retires the motion risk
M2  project model + 3 sources ── 1–2 wks  ── retires the input risk
M3  queue, state, resume, 500 ── 2–3 wks  ── retires the scale risk   ← hardest
M4  Tauri shell + review grid ── 2 wks    ── first thing an operator sees
M5  packaging + commercial    ── 1–2 wks  ── signing, notarization, updater
```

Estimates are for one focused engineer and assume D-070/D-071 get answered
before M2 closes. They are calibration, not commitments.

---

## M0 — Toolchain and workspace skeleton · ✅ COMPLETE 2026-08-26

All 8 exit gates pass; rerun them any time with `make gates`. Commit `6353f25`.

What exists: six crates, binary `still`, `rust-toolchain.toml` pinning 1.94.0,
`Makefile`, `.github/workflows/ci.yml`, `scripts/gen-fixtures.sh`,
`scripts/m0-gates.sh`, and `crates/spoonstill-cli/tests/architecture.rs`.
11 tests green, clippy clean at `-D warnings`.

What does **not** exist: any rendering code at all. No filter graph, no FFmpeg
process boundary, no `SEGMENT_PROFILE`, no state, no queue, no CLI subcommands.
The four infrastructure crates are documented stubs with one test each.

Two corrections this milestone made to the documents:

- **Rust was already installed** — this file said otherwise. rustc 1.94.0 was
  present; only `~/.cargo/bin` was absent, because Homebrew's rustup keeps its
  shims in `/opt/homebrew/opt/rustup/bin`. That directory was not on `PATH`, so
  every check reported "not installed". Now on `PATH` via `~/.zshrc`, and
  re-exported by the `Makefile` so `make test` works from any shell.
- **`odd.jpg` was generating even** (1998×1000), which would have made the D-033
  SAR test vacuous. See `ffmpeg-findings.md` §8b. `gen-fixtures.sh` now asserts
  the fixture is genuinely 1999×1001 and hard-fails otherwise.

`just` is not installed; this section permits "justfile (or Makefile)", so the
entry points are `make test`, `make lint`, `make fixtures`, `make gates`.

**Goal.** A `cargo test` that runs, in a repo with history, with the
architectural boundary of D-010 enforced by the compiler rather than by
discipline.

**Why first.** Nothing else can be verified without it, and the dependency
direction is far cheaper to establish now than to retrofit.

### Deliverables

- Rust stable via `rustup`. Pin it: `rust-toolchain.toml`.
- `git init`, and a first commit that contains the planning docs **before** any
  code, so the reasoning has a history separate from the implementation.
- `.gitignore` covering `target/`, `.spoonstill/`, `plan/` (see the note below),
  `*.mp4`, `*.mkv`, and scratch fixtures.
- The Cargo workspace of D-010. Every crate compiles empty with one real test.
- `crates/spoonstill-core/Cargo.toml` has **no** dependency on Tauri, React, any TTS
  SDK, or any process/UI crate. Enforce it — a `cargo-deny` rule or a test that
  parses the manifest. A comment is not enforcement.
- `justfile` (or `Makefile`) with `just test`, `just lint`, `just fixtures`.
- `.github/workflows/ci.yml`: fmt, clippy `-D warnings`, test. macOS runner
  only for now (D-071); leave the Windows job present and commented with the
  reason.

> **`plan/` and version control.** 1.7 GB of shallow checkouts, ~70 % of it
> Remotion (D-061). Do not commit them. Record the pinned commits — they are
> already in `refergit.md` §2 — and gitignore the directory. Anyone can restore
> an exact checkout from the hash.

### Exit gates

```bash
cargo --version && rustc --version      # both resolve
cargo test --workspace                  # green, every crate has ≥1 real test
cargo clippy --workspace -- -D warnings # clean
git log --oneline                       # planning history exists before code
cargo tree -p spoonstill-core | grep -iE 'tauri|reqwest|elevenlabs'   # no output
```

---

## M1 — One scene, end to end · COMPLETE (2026-08-26)

> **Status: complete. `make gates-m1` is 8/8.**
>
> `still render-scene --image X --audio Y --out seg.mp4` produces a segment that
> passes the full profile assertion. What landed:
>
> | Deliverable | Where |
> |---|---|
> | `build_filter`, pure | `spoonstill-core/src/motion.rs` |
> | geometry, prescale, the three aspects | `spoonstill-core/src/geometry.rs` |
> | frame/sample arithmetic (D-021, D-022) | `spoonstill-core/src/timing.rs` |
> | stable content hash (D-035, D-043) | `spoonstill-core/src/hash.rs` |
> | process boundary, retained child | `spoonstill-media/src/command.rs` |
> | timed, typed `ffprobe` | `spoonstill-media/src/probe.rs` |
> | `SEGMENT_PROFILE` + assertion | `spoonstill-media/src/profile.rs` |
> | render → validate → atomic move (D-042) | `spoonstill-media/src/scene.rs` |
> | the motion matrix | `spoonstill-media/tests/motion_matrix.rs` |
> | `still render-scene`, `still diagnostics` | `spoonstill-cli/src/main.rs` |
>
> **Decisions made or changed while building it** — each recorded in
> `decisions.md` in the same commit as the code:
>
> - **D-012 resolved: reimplement, do not depend on `ffmpeg-sidecar`.** Its
>   auto-download is default-on and already rejected; it has no timeout-bounded
>   `ffprobe`, which M1 named as a risk and needed from the first call; and its
>   typed stderr events are something plan.md already forbids relying on.
> - **D-037 added, and it changes this milestone's filter chain.** A JPEG is
>   full-range, and that flag survives `format=yuv420p` into the encoder, which
>   then reports `yuvj420p`. The chain gains `:out_range=tv` on the prescale and
>   a `setparams` before `setsar=1`. Measured, with the reproduction in
>   `ffmpeg-findings.md` §7b. D-033 still holds literally — `setparams` sets
>   metadata and does not touch SAR.
> - **D-016 added:** diagnostics are written as they happen and exported as one
>   file, so a failure on another machine can be diagnosed here. Author request.
> - **D-070 accepted** (16:9, 9:16, 1:1 all V1) — the recorded default, now
>   implemented and covered in the matrix.
> - **D-071 accepted** (cross-platform from M1) — author decision. The code is
>   written for both platforms and the Windows CI job is on; **nothing has been
>   run on Windows yet**, and every number in `ffmpeg-findings.md` is still
>   macOS arm64.
>
> Two things this milestone added beyond its brief, both because they were
> cheaper now than later: the segment profile pins the four colour fields, not
> just SAR and pixel format; and the black-edge probe has a control test proving
> it can fail (`ffmpeg-findings.md` §7c), because a vacuous probe would have
> made every black-edge assertion in the matrix meaningless.


**Goal.** `still render-scene --image X.jpg --audio Y.mp3 --out seg.mp4`
produces a segment that passes the full profile assertion, on a machine that
has never seen this project.

**Why now.** This is the whole product in miniature. Everything after it is
multiplication, orchestration, and presentation.

### Deliverables

**`spoonstill-media` — the process boundary (D-011, D-012)**

- `FfmpegCommand`-shaped builder over `OsStr`. Argument vectors only; a test
  asserts no code path ever builds a command from a formatted string.
- Retained child handle with separate `quit()` / `kill()` / `wait()`.
- Progress via `-progress pipe:` where the command is ours; **raw stderr
  retained regardless**, because a parser-classified log level is not a
  substitute for exit status plus output validation.
- `ffprobe` JSON with a timeout and typed normalization. Model it on
  `plan/lossless-cut/src/common/ffprobe.ts` — the most thorough local schema.
  It is GPL-2.0-only: read it, do not paste it (D-062).
- Fail fast with the specific path when a configured binary is missing.
- Log a safely escaped display form of every command executed. Operators need
  to paste it into a terminal; `plan/lossless-cut/src/renderer/src/LastCommands.tsx`
  is the pattern.
- Hide the extra console window on Windows.

**`spoonstill-core::motion` — the filter graph (D-030…D-035)**

One function. Pure. No I/O:

```rust
fn build_filter(
    source: SourceGeometry,   // width, height, SAR
    output: OutputSpec,       // width, height, fps
    motion: MotionSpec,       // kind, amount, anchor, seed
    frames: u32,
) -> String
```

It emits, in this order and no other:

```
scale=<3*OUT_W>:<3*OUT_H>:force_original_aspect_ratio=increase,
crop=<3*OUT_W>:<3*OUT_H>,
zoompan=z='<f(on)>':x='<expr>':y='<expr>':d=<N>:s=<OUT_W>x<OUT_H>:fps=<FPS>,
setsar=1,
format=yuv420p
```

Non-negotiable, each with a test that fails if it regresses:

- prescale is `3 * output_height`, derived (D-032) — not a constant, not 8000
- cover-fit happens **before** `zoompan` (D-034)
- `setsar=1` is **last** before `format` (D-033)
- the frame count is structural: single still, no `-loop`, `d=N`,
  `-frames:v N` (D-030)
- motion choice is seeded from `(project_id, scene_index, content_hash)` (D-035)

**`SEGMENT_PROFILE` — one constant, one assertion (D-040)**

Codec, profile, level, pixel format, dimensions, SAR, frame rate, time base,
audio codec, sample format, sample rate, channels, layout, container, stream
order. Plus `assert_matches_profile(&ProbeResult) -> Result<(), Vec<Mismatch>>`
that names every field that differs.

Write this in M1 even though nothing concatenates yet. Retrofitting a profile
after segments exist means re-rendering all of them.

### Exit gates

```bash
# 1. the single-scene render works and is frame-exact
still render-scene --image fixtures/land.jpg --audio fixtures/n.mp3 --out /tmp/s.mp4
ffprobe -v error -select_streams v:0 -count_frames \
  -show_entries stream=nb_read_frames,sample_aspect_ratio,width,height,r_frame_rate \
  -of default=nw=1 /tmp/s.mp4
# frames == ceil((audio_dur + pads) * fps), SAR == 1:1, dims == output spec

# 2. the motion matrix — this is the M1 test that matters
cargo test -p spoonstill-media --test motion_matrix -- --nocapture
```

`motion_matrix` ports `ffmpeg-findings.md` §9 into CI: 3 durations × 2 frame
rates × every V1 aspect ratio × landscape/portrait/square sources ×
ASCII/Unicode/spaced paths. For each combination it asserts exact frame count,
exact duration, SAR 1:1, and no black edge on the first, middle, and last frame.

```bash
# 3. odd dimensions do not corrupt SAR (the D-033 regression)
cargo test -p spoonstill-media odd_dimensions_sar

# 4. hostile paths survive
still render-scene --image "fixtures/ünïcode spaced 名前.jpg" --audio ... --out ...

# 5. cancellation is clean
still render-scene ... & sleep 1; kill -INT %1
ls /tmp/s.mp4        # absent, or present and marked partial — never a valid-looking stub
```

### Risks

- **`ffmpeg-sidecar` is blocking.** Decide here: depend on it behind a blocking
  worker boundary, or reimplement ~600 lines. Either is fine; deciding late is
  not (D-012).
- **`ffprobe` on hostile media hangs.** Every probe gets a timeout from the
  first call, not after the first hang.

---

## M2 — Project model and the three audio sources · COMPLETE

**Goal.** `still render ./project/` renders a three-scene project — one TTS
scene, one supplied-audio scene, one silent-duration scene — from a folder an
operator could have produced by hand.

**Where it stands:** done, 2026-08-26. `still render fixtures/projects/mixed/`
renders all three sources into one film, and that is gate 7.

### Progress

M2 is being built in four slices, each one landing with its own tests and its
own decisions. Where a slice is done, the exit gate it satisfies is named.

| Slice | What | State |
|---|---|---|
| **1. The pure domain** | `spoonstill_core::path_safety` + `spoonstill_core::project`: containment, the scene model, and every validation rule that needs no disk. D-054, D-055. | ✅ 2026-08-26 — satisfies `cargo test -p spoonstill-core path_safety` |
| **2. Import and `still validate`** | `project.yaml` and the CSV manifest, convention-mode stem pairing, path resolution and media probing merged into one problem list, `still validate` printing it. D-056. | ✅ 2026-08-26 — satisfies `still validate fixtures/projects/mixed/` |
| **3. The three audio sources, and `still render`** | `AudioSource::resolve()` → `(normalized_path, Duration)`: ingest normalization to 48 kHz stereo, `ffprobe` on the normalized artifact, generated silence. Then `still render DIR` over a whole project — **parallel**, with two bounded pools. D-075, D-076, D-077, D-078. | ✅ 2026-08-26 — `make gates-m2` is 20/20 |
| **4. Speech behind a trait** | `spoonstill-tts`: the `Provider` trait, typed settings and errors, and the `edge` implementation — `edge-tts` through the one process boundary, cached under `hash(text, provider, voice, settings, profile)`. `still voices`, `--voice`. D-081, D-082. ElevenLabs is deferred, not cancelled. | ✅ 2026-08-26 — gate 7 renders `mixed/` |
| **+ Getting media in** | Not in the original four. `spoonstill_app::ingest`, `still new`, `still add`: the operator drops what they have and the program names and pairs it. D-080. | ✅ 2026-08-26 |

Slice 1 notes, for whoever picks this up:

- `spoonstill-core` still has **zero dependencies**. Path canonicalization —
  the one fact the domain cannot derive — enters through the `RealPath` trait,
  so the containment tests run with a fake filesystem and pass identically on
  Windows (D-071).
- Validation is deliberately incomplete on its own: it never asks whether a
  file exists or whether it is really a JPEG. `ProblemKind::Path` and
  `ProblemKind::NotUsableMedia` exist for slice 2 to fill in, so the operator
  gets **one** report rather than one per stage.
- `Anchor::parse` was added to `spoonstill_core::motion` for the manifest's
  `zoom_anchor` column, with a round-trip test over `Anchor::ALL`.

Slice 2 notes:

- `still validate DIR` is real and is the M2 gate. It prints the mode, the
  scene list with its audio-source badges, and every problem at once —
  project-level first, then scene by scene in **render order**, so a project is
  fixed row by row rather than stage by stage.
- **Resolution runs over every row, not only the rows that validated.** A row
  with a missing `duration` *and* a mistyped image path reports both in one
  run. That was a real bug caught by running the command rather than the tests.
- Two fixture projects are generated by `make fixtures`, one per D-050 mode:
  `fixtures/projects/mixed/` (convention) and `fixtures/projects/manifest/`
  (CSV, with per-scene `zoom_direction`/`zoom_anchor`). Both validate clean.
- The `ffprobe` media check sits behind the `MediaCheck` trait, so every test
  around it runs with no FFmpeg — and M3's queue can swap in a cached
  implementation when it is probing 500 files rather than 6.
- Still **not** done after slice 2: no audio is resolved or normalized yet, and
  `still render` over a project does not exist. That is slice 3.

Slice 3 notes:

- **`still render DIR` is real, and it renders several scenes at once.**
  Requested by the author: *"make it like that i can make multiple render at
  the same time"*. Two bounded pools (D-044): the audio pool resolves every
  narration, then the render pool encodes every segment, then the join. The
  default is `available_parallelism() / 2` capped at 4 — **measured**, not
  chosen: `ffmpeg-findings.md` §10a, D-076. `--jobs` and `--audio-jobs`
  override it and are uncapped.
- **Concurrency changes the timing and nothing else** (D-077). `--jobs 1` and
  `--jobs 4` produce byte-identical films, and gate 3 asserts it. Motion is
  seeded before the pool starts, each worker writes its own content-addressed
  segment path, and results are collected by input index.
- **A re-run re-encodes nothing.** Normalized audio is content-keyed in
  `.spoonstill/cache/audio/` (D-075) and segments are content-named in
  `.spoonstill/segments/`, each one only getting its final name after passing
  the profile assertion (D-042). The 6-scene fixture renders in 2.99 s cold at
  `--jobs 1`, 1.31 s cold at `--jobs 6`, and 0.40 s warm. A cache hit is still
  *probed* every run — trusting it would be D-021 with the "measured" removed.
- **One render per project at a time**, via `.spoonstill/render.lock`, because
  two runs would interleave segments into one film. Two runs against
  *different* projects share nothing and are not locked.
- **The film is asserted on its video stream, not the container** (D-078). An
  MP4's container duration is its longest track's, and AAC priming puts the
  audio one 1024-sample frame beyond the video. Measured: `ffmpeg-findings.md`
  §10c.
- TTS is refused with a typed, named error rather than being silently replaced
  by silence — gate 7. A line somebody wrote must never become a silent scene.

### Deliverables

**Manifest and import (D-050)**

- `project.yaml` parse, validate, and a `still validate` that reports every
  problem at once with scene IDs. Not the first error — all of them.
- Convention mode: stem-keyed pairing, plus the CSV manifest, manifest wins.
- Resolution produces a `ResolvedProject` where every scene has an image, an
  `AudioSource`, and a motion spec. Unresolvable rows are collected as typed
  warnings, never a panic and never a silent skip.
- Path safety centralized in one module: canonicalize, assert containment
  within the project root, reject traversal. Follow the *shape* of
  `plan/MoneyPrinterTurbo/app/services/task.py:359` `resolve_custom_audio_file()`
  — in particular its rule that an out-of-bounds path returns a generic error
  whether or not it exists, so the caller cannot probe the filesystem.

**Audio sources (D-020, D-021, D-022)**

- The `AudioSource` enum, and a `resolve()` returning
  `(normalized_path, Duration)`.
- Ingest normalization to 48 kHz stereo. **The operator's original is never
  touched**; the normalized copy lives in the cache.
- Duration from `ffprobe` on the normalized artifact. Reject zero, NaN, and
  absurd values with the scene ID attached.
- `Silent { duration }` generates a real silent track — not a special case
  threaded through the renderer.
- TTS behind a trait with typed settings and errors. ElevenLabs first
  (BYOK, `keyring-rs`, D-014). One `provider` module per implementation; a
  giant `match` on provider name is the `MoneyPrinterTurbo/app/services/voice.py`
  mistake.

**Validation rules**

- Exactly one of `text` | `audio` | `duration`. Zero is an error; two is an
  error (D-020).
- Every referenced file exists, is readable, and probes as the media type it
  claims to be. Extensions are a hint, not evidence.

Slice 4 notes:

- **Edge TTS is the `edge-tts` command line tool, not a Rust client** (D-081).
  The endpoint is reverse-engineered and moves; the Python implementation
  tracks it. It is spawned through `spoonstill_media::command` — the one place
  a process is spawned — so it inherits argument vectors, a timeout, retained
  stderr, and a paste-ready command line, and `spoonstill-tts` moves to its own
  layer above `spoonstill-media` to make that legal. The architecture test
  enforces the new direction.
- **The script goes in a file, never in an argument.** The command line is
  logged (D-016) and lands in the bundle the operator sends us. Their words are
  their content.
- **A cache miss is the operator's money** once ElevenLabs lands, so the
  provider's raw output is kept beside the normalized artifact: changing the
  normalization profile re-normalizes without re-speaking. The key's fields are
  length-prefixed, so no boundary can be moved without changing it.
- **A voice chosen at the command line or in the window is an override for one
  run** (D-082). Nothing writes `project.yaml`.
- **Still owed from this slice: ElevenLabs.** `providers()` is one line plus a
  module, and the recorded-fixture discipline in the risks below still stands.

### Exit gates

`make gates-m2` runs all of these. **20/20 pass as of 2026-09-04**, slice 4
included, and D-143's size and shape gate, D-144's capacity gate, D-145's
undersized-source gate, D-146's overlap gate, D-147's audio-cache bound and D-148's
activity-log gate with them.

```bash
still validate fixtures/projects/mixed/     # 3 scenes, 3 sources, 0 warnings
still render   fixtures/projects/renderable/ --out /tmp/out.mp4 --jobs 4

ffprobe -v error -select_streams v:0 -show_entries stream=duration -of csv=p=0 /tmp/out.mp4
# == sum of the resolved scene durations, within one frame
# The *container* duration is one AAC frame longer; assert the video stream
# (D-078, ffmpeg-findings.md §10c).

cargo test -p spoonstill-app validation      # both-sources and no-sources rejected
cargo test -p spoonstill-core path_safety    # ../ traversal rejected; no existence leak
```

Two amendments this milestone made to its own gates, both recorded rather than
quietly applied:

- **The bulk of the render gates run against `fixtures/projects/renderable/`,
  not `mixed/`.** `renderable` is the same shape without a spoken line, so most
  of M2 is provable on a machine with no network and no voice service. `mixed`
  is gate 7's alone, and gate 7 now renders it. Where `edge-tts` is absent that
  gate asserts the other half of D-020 instead: the render must fail and name
  the missing tool, because a line somebody wrote must never quietly become
  silence.
- **The duration gate is asserted on the video stream** (D-078).

Four gates that slice 3 added, because parallel rendering makes four new
promises:

```bash
# 3. concurrency changes the timing and nothing else (D-077)
still render P --out a.mp4 --jobs 1 && still render P --out b.mp4 --jobs 4
shasum -a 256 a.mp4 b.mp4          # identical

# 4. a second run reuses every narration and every segment (D-043, D-075)
# 5. a second render of one project is refused, and says --force (D-077)
# 6. odd dimensions and a Unicode filename survive the join (D-033, D-052)
# 7. a script, a recording and a silent still become one film (D-020)

# 7b. 2K, 4K and a vertical Short come out at the size asked for (D-143)
still render P --out s.mp4 --aspect shorts --resolution 4k   # 2160x3840, not 3840x2160
still render P --out s.mp4 --short-edge 4320                 # refused: past D-114's ceiling

# 7c. four workers fit at 1080p and are warned about at 4K (D-144)
# The budget is stated, not measured. The core count cannot be stated, so the
# assertion is on an *explicit* --jobs: a small runner derives one worker for
# every size and cannot express a difference between automatic counts.
SPOONSTILL_MEMORY_BUDGET_MB=5600 still render P --resolution 1080p --jobs 4
                                   # 3.3 GB — runs four, says nothing new
SPOONSTILL_MEMORY_BUDGET_MB=5600 still render P --resolution 4k --jobs 4
                                   # 11 GB — obeyed, warned, told "try --jobs 2"
```

Secrets check — grep the run output, the manifest, the state DB, the cache
keys, and the logged command lines for the API key. Zero hits, and a test that
keeps it that way. **Still owed**: `edge` is BYOK-free, so there is no key in
the product yet. It becomes real with ElevenLabs. The discipline is already in
place, though — `edge` passes the operator's script through a *file* rather
than an argument precisely because the command line is logged (D-081).

### Risks

- **ElevenLabs is a live paid API.** Build against a recorded-response fixture
  from the start; the real key belongs in one integration test that is skipped
  by default.
- **D-070 (9:16 in V1) must be answered before this closes.** It changes the
  test matrix, not the code — but it changes it a lot. *Answered: all three
  aspects ship, and D-143 made every one of them reachable from `still render`
  and from the window as a run override.*

---

## M3 — Queue, state, resume, and n=500

**Goal.** A 500-scene project renders, survives being killed at scene 147,
resumes without redoing 146, and re-renders exactly one scene after a one-word
edit.

**This is the hardest milestone and the one most likely to be underestimated.**
M1 and M2 are correctness in the small. This is correctness under concurrency,
partial failure, and restart — three things that do not compose by accident.

### Deliverables

**`spoonstill-state` — SQLite (D-013, D-042)**

- Schema: scenes, status, cache keys, resolved durations, segment paths,
  attempts, failure detail, timestamps. Migrations from the first commit.
- One transaction per state transition. Explicit legal transitions —
  `Pending → Resolving → Resolved → Rendering → Rendered → Validated`, plus
  `Failed{reason}` and `Cancelled` — as a type, not a set of booleans. Study
  `plan/Automated-Video-Generator/src/agentic/management/job.ts`.
- **`project.yaml` is never written to.** A test asserts its mtime and hash are
  unchanged after a full render.
- `.spoonstill/` deletable at any point: the next run rebuilds from manifest + cache
  and re-renders only what the cache cannot supply.

**Concat, and the transition mode (D-040)**

- Hard cuts are the default: concat demuxer plus stream copy, after every
  segment has passed the profile assertion (D-041). This is what makes n=500
  affordable.
- **Transitions are a setting: `cut`, `fade`, `dissolve` (D-057).** Project
  level in `project.yaml`, with the same two names as optional per-scene
  manifest columns, matching how `zoom_direction` already works.
- **Measure the `xfade` cost curve before changing the default.**
  `ffmpeg-findings.md` §8 lists it as asserted and unmeasured; D-057 keeps the
  default at `cut` until there is a number, because anything else re-encodes
  both sides of every join and n=500 is the design point.
- **The overlap comes out of the pads, never the narration (D-057).** A
  transition of length `d` overlaps the outgoing tail pad and the incoming head
  pad; where they are too short they are extended, not trimmed — the same rule
  as D-022, applied at the join. The film's duration is the sum of the scenes
  minus `d` per join, computed and asserted rather than left to `xfade` to
  imply.

**Cache (D-043)**

- Content-addressed. Keys hash content bytes plus every output-affecting
  setting. Never a path or URL alone.
- A key-derivation test that changes each input in turn and asserts the key
  moves — the failure mode is a key that ignores a field, and it is invisible
  until someone gets a stale segment.
- Bounded, atomic, corruption-tolerant. A corrupt cache entry is a miss, never
  a crash, and never corrupts a source file.

**Queues (D-044)**

- Two pools with independent caps. TTS is network- and rate-limit-bound; render
  is CPU- and RAM-bound. Sharing one pool starves whichever is slower.
- TTS: per-provider concurrency cap, retry with backoff, explicit rate-limit
  handling. ElevenLabs allows low single digits on cheap tiers.
- Render capacity derived from measured available RAM. Start at ~1.5 GB per
  worker (`ffmpeg-findings.md` §1 control), then **measure it** — that figure is
  extrapolated from one static encode, not from N concurrent segment renders.

**Cancellation (D-045)**

Stop admitting → graceful `q` to each child → force-kill after a deadline →
delete or mark partials → persist resumable state. Supervise only this job's
children. Never enumerate and kill by name or memory use.

**Concat (D-040, D-041)**

- Assert `SEGMENT_PROFILE` against `ffprobe` output for **every** segment
  before writing the concat list. §5 of `ffmpeg-findings.md` is why: FFmpeg
  returns exit 0 on a mismatched join.
- Defensive concat-list generation. Do not rely on simple quoting;
  `plan/ffmpeg-ai`'s `concat_plain()` and editly's
  `seg.replace(/'/g, "'\\''")` are both too optimistic for arbitrary operator
  filenames. Prefer stable internal segment names in the list and keep the
  human-facing name out of it entirely.

### Exit gates

Generate a 500-scene fixture project, then:

```bash
# 1. clean run — record wall time and peak RSS
/usr/bin/time -l still render fixtures/projects/n500/ --out /tmp/n500.mp4

# 2. kill mid-flight, resume
still render fixtures/projects/n500/ --out /tmp/n500.mp4 &
sleep 60 && kill -9 %1
still render fixtures/projects/n500/ --out /tmp/n500.mp4    # resumes; log shows ~0 re-renders

# 3. determinism — byte-identical across a full re-run
shasum /tmp/n500-a.mp4 /tmp/n500-b.mp4                      # identical

# 4. single-scene invalidation
sed -i '' 's/old word/new word/' fixtures/projects/n500/scene_047.txt
still render fixtures/projects/n500/ --out /tmp/n500.mp4 --verbose | grep -c 'cache miss'
# exactly 1 scene re-rendered, plus the final concat

# 5. mismatched segment is refused
cargo test -p spoonstill-app concat_rejects_profile_mismatch

# 6. cancellation leaves nothing valid-looking
cargo test -p spoonstill-app cancel_leaves_no_partial_segment
```

Record the wall time and peak RSS from gate 1 in this file. A number nobody
wrote down is a number nobody can regress against.

**Measured 2026-09-04 (D-154).** 500 scenes, 1920x1080, `--jobs 4`, supplied
narration, on the ten-core machine every other number here comes from:

| | |
|---|---|
| cold render, three runs | **161 s, 151 s, 154 s** |
| peak resident, all children | **2873 MB** |
| film | 30 000 frames, 1000.021 s against 1000.000 expected |
| all three cold films | byte-identical |
| killed at 60 s (167 of 500 done), then resumed | **167 reused**, 105 s, film identical to a clean run |
| one narration edited | **499 of 500 reused** |

**Gates 1, 2, 3 and 4 pass on the tree as it stands, with no state database** —
resume falls out of D-043's content-addressed cache rather than out of anything
that remembers. Gates 5 and 6 exist as tests under other names
(`profile.rs`'s per-field assertions; `cancellation_leaves_no_valid_looking_stub`).
What M3 still owes is the database **as an index for reporting**, transitions
(D-057), and an integration test for gate 5. Read D-154 before starting it.

### Risks

- **Determinism is easy to lose and hard to notice.** Any unseeded choice, any
  hash-map iteration order that reaches a filter string, any timestamp in a
  cache key. Gate 3 is the only thing that catches it — run it every milestone
  after this one, not once.
- **The resume gate passes trivially if the cache is too eager.** Pair it with
  gate 4: a resume that redoes nothing *and* an edit that redoes exactly one
  scene together prove the cache key is right.
- **The pool sizing figure is a guess until M3 measures it.** Say so until then.

---

## M4 — Tauri shell and review grid

**Goal.** An operator selects a folder, reads the review grid, hits Render,
watches progress, and cancels — with the shell containing no business logic.

**Entry condition: M3's exit gates all pass.** A UI over an unreliable queue
makes the queue harder to fix, not easier to use.

### Deliverables

- Tauri 2 (the local checkout is `2.11.5`). Commands for control-plane
  requests; ordered `Channel`s or `emit_to` for progress and log streams.
- Queue and supervisor handles in managed Rust state. **Never in React state.**
- Capabilities granted to named windows and exact commands only. No general
  shell permission, no broad `fs` scope — the frontend passes selected paths
  into validated Rust commands and never constructs an FFmpeg argument.
- Review grid (D-051): thumbnails, resolved durations, audio-source badges,
  loud warnings on unresolved rows. Read-only by default.
- Single-scene preview (D-053): same filter graph, reduced scale, structurally
  unable to mutate render state.
- Explicit exit handling: `ExitRequested` cancels active children and flushes
  state. Study `plan/tauri/crates/tauri/src/app.rs`.
- `plugins-workspace`: `store` for UI prefs only (never project or render
  state), `dialog` for folder selection, `window-state`.

### Exit gates

```bash
cargo test --workspace          # still green with the shell present
cargo tree -p spoonstill-core | grep -i tauri     # no output — the boundary held
```

- Every CLI capability is reachable from the UI, and every UI action maps to a
  CLI-expressible operation.
- Killing the UI mid-render leaves resumable state — same guarantee as M3.
- A capability audit: list every granted permission and justify each in one
  line. `plan/example-tauri-v2-python-server-sidecar` is the anti-pattern here —
  wildcard CORS, `csp: null`, broad `http://**/`. Do not copy its config.

---

## M5 — Packaging and commercial readiness

**Goal.** A signed, notarized, auto-updating build with a defensible licence
position.

### Deliverables

- **FFmpeg licensing resolved (D-062).** The dev build is GPL+version3 and
  cannot ship with a proprietary product. Build or source LGPL FFmpeg; record
  the exact source, build flags, enabled codecs, and notices. **Do this before
  M5, not during it** — it can invalidate an encoder choice.
- Bundled, pinned, checksum-verified FFmpeg/ffprobe per platform. No runtime
  download (D-012).
- macOS: hardened runtime, code signing, notarization. Windows: signing
  certificate. Both cost money and lead time; budget them at M0, not here.
- Auto-update via the Tauri updater with signing keys configured and insecure
  transport rejected in release builds.
- Third-party licence manifest generated at build time, covering every crate
  and npm package, plus the FFmpeg notice.
- Windows first-class (D-071), including a Windows CI runner and the full
  motion matrix re-measured there — every number in `ffmpeg-findings.md` is
  macOS arm64.

### Exit gates

- A downloaded build launches on a clean machine with no security warning, on
  both platforms.
- An update is offered, downloaded, verified, and installed.
- The licence manifest is complete, and the FFmpeg build's licence is compatible
  with the intended distribution.

---

## Cross-cutting: how work gets verified

### Fixtures, committed once and reused

| Fixture | Exercises |
|---|---|
| `land.jpg` 4000×3000 | landscape → every aspect |
| `port.jpg` 3000×4000 | portrait → every aspect |
| `square.jpg` 2000×2000 | square → every aspect |
| `odd.jpg` 1999×1001 | the D-033 SAR trap |
| `cmyk.jpg` | colour-space conversion |
| `exif_rotated.jpg` | orientation handling |
| `truncated.jpg` | graceful failure with a named cause |
| `ünïcode spaced 名前.jpg` | argument vectors, never shell strings |
| `vbr_lying_header.mp3` | D-021 — header duration ≠ real duration |
| `zero_byte.mp3` | rejected with the scene ID attached |
| `n500/` (generated) | M3 scale gates |

Generate what can be generated (`just fixtures`); commit only what cannot.

### The gate that applies to every milestone

Borrowed from `refergit.md` §8 and tightened:

- [ ] Filenames include spaces and Unicode; commands are argument vectors
- [ ] Audio is normalized and probed before motion is computed
- [ ] Every segment is independently rendered, probed, atomically checkpointed
- [ ] Re-running unchanged input is a cache hit with identical motion choices
- [ ] Changing one scene invalidates that scene and the final concat, nothing else
- [ ] Cancellation leaves no valid-looking partial; the next run resumes
- [ ] Concat refuses a deliberately mismatched segment
- [ ] Motion covers the duration × fps × aspect × source-shape matrix, no black edges
- [ ] Preview uses the same recipe and cannot mutate render state
- [ ] Raw FFmpeg stderr is retained with a scene ID and a human summary
- [ ] Secrets appear in no log, manifest, cache key, argument, or project file
- [ ] Every copied or reference-derived construct has a recorded licence decision

### Working with the reference repos

`plan/` is read-only study material. For each borrowed idea: find it via
`refergit.md` §3, open the actual source, label it **adopt / modify / reject**
in the PR, reimplement it in Rust, and add a test that proves the target
behaviour. Reference implementations are evidence, not oracles — §2 and §3 of
`ffmpeg-findings.md` are two cases where the local repos were confidently wrong.

---

## What not to build, restated

Because these creep in as "small additions":

- A timeline, a scrubber, or drag-to-arrange (D-001, D-003)
- Video clips as scene sources — stills only in V1
- Image generation, script writing, stock search, cloud upload
- Multi-track mixing beyond one music bed
- A localhost HTTP server between the UI and the core — the Rust core talks to
  FFmpeg directly; the Python-sidecar example is a lifecycle reference only
- Remotion, at runtime, in any form (D-061)
- Runtime FFmpeg download (D-012)
- Any editable field in the review grid that is not justified against D-001

## Immediate next actions

Updated **2026-08-30**, after an external audit was worked through end to end.

**The audit is closed.** Twenty-six decisions, D-107 through D-132 — the list
is in `CLAUDE.md` under "State as of 2026-08-30", and each decision carries its
own reproduction. `make gates` is 8/8, 8/8, 20/20; `cargo test --workspace` is
558; `cargo audit --deny warnings` is new and green. Nothing it found changes
M3's shape, and M3 has still not been started.

**D-132 was not in the audit — it came from checking the audit.** Building the
workspace for `x86_64-pc-windows-msvc` found two Windows defects in the audit's
own commit: `posix_quote` is dead code there, which `RUSTFLAGS: -D warnings`
would have failed the Windows CI leg on at the next push, and the title bar's
82px traffic-light indent is macOS-only room drawn under a native Windows title
bar. **Add the cross-check to the pre-release routine** — the exact command is
in `CLAUDE.md`, it takes about thirty seconds, and it is the difference between
a green macOS run and a release whose Windows leg dies on a lint.

**Shipped as v0.1.5.** v0.1.4 predates every one of these decisions, so anyone
who downloaded from the releases page had none of them. Version bumped in
`Cargo.toml` and `tauri.conf.json` together, which is what D-102's first
workflow step checks against the tag. The release page itself is shorter as of
D-133: five downloads and one `SHA256SUMS.txt`, no `.sha256` twins and no
`.msi`.

Four of its findings did **not** survive being tested, and the decisions say so
rather than quietly fixing something that was not broken: FFmpeg is not orphaned
when the window dies (D-115), a wrong-length cached segment never produced a
wrong film (D-110), the folder scan's winner was stable rather than random on
APFS (D-111), and the two installer failures a test can actually reach were
already handled correctly (D-123).

Two cosmetics were noticed and left, both trivial and neither worth a decision:
the `--subtitles` error message has a run of about twenty spaces in it, and the
window does not show the swept-cache figure the CLI prints.

---

The list below is from 2026-08-26, after M0, and items 1–5 are historical.

1. **Answer D-070 (9:16 + 1:1 in V1).** This is the only Open decision that
   shapes work already in flight: it sets the breadth of M1's `motion_matrix`,
   which is M1's headline exit gate. It changes the *test matrix*, not the
   renderer — D-034's cover-crop is already aspect-agnostic. Recorded default:
   yes, all three in V1. Needed before M1 closes, not before it starts.
2. **Start M1.** Unblocked. Order within it, riskiest first:
   `spoonstill-core::motion::build_filter` (pure, no I/O, fully testable against
   `ffmpeg-findings.md` §1–§6) → `SEGMENT_PROFILE` + its assertion →
   `spoonstill-media` process boundary → `still render-scene` wiring them up.
   Decide D-012 (depend on `ffmpeg-sidecar` vs reimplement ~600 lines) when the
   process boundary starts, and record it.
3. **Answer D-071 (Windows day one).** Does not block M1. It decides whether the
   commented-out Windows job in `.github/workflows/ci.yml` turns on, which also
   means re-measuring every number in `ffmpeg-findings.md` on Windows.
4. **Decide on `plan/remotion` (D-061).** 1.2 GB for a paragraph of concepts
   already written down. Now gitignored either way, so this is disk only.
5. **Start the FFmpeg licensing question (D-062).** Lead time, and it can
   invalidate an encoder decision — it should not wait for M5.

Not blocking anything, but worth knowing: the workspace directory is still
`vidio/` while the project is `spoonstill` (D-073). Renaming it is cosmetic and
is the author's call.

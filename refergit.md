# GitHub reference guide for `spoonstill`

This file is the working index for Claude Code (and any other coding agent) while building `spoonstill`. It summarizes every GitHub repository currently checked out under `plan/`, identifies the exact source files worth consulting, and records what should be adopted, modified, or rejected.

The repositories are read-only study material. Do not build the application inside them, edit them, copy an architecture wholesale, or silently add one of them as a runtime dependency.

## 1. Read this before writing code

Use the planning documents in this order:

1. `CLAUDE.md` — orientation and the rules that hold everywhere.
2. `decisions.md` — **the single source of truth.** Numbered decisions (D-001…) with status and reasoning. It outranks every other document, this one included.
3. `plan.md` — milestones M0–M5 with runnable exit gates.
4. `ffmpeg-findings.md` — benchmarks measured on this machine. Evidence, not policy: it outranks every claim in the retired docs and in the reference repos, but not `decisions.md`.
5. This file — to decide which repository and source path to inspect for a particular implementation problem.
6. `plan/BRIEF_RECONCILIATION.md`, `plan/PROJECT_BRIEF.md`, `plan/REFERENCES.md` — **all retired.** Kept for their reasoning. Do not build from them.

The author-owned `kenburns-batch` master brief is described as authoritative by the retired reconciliation file, but it has never been present in this workspace. Do not invent missing requirements from it (D-074).

The reconciled architecture is CLI-first: a Rust library/core, a permanent headless CLI, and a later thin Tauri shell. The UI must never own the render queue or contain business logic that the CLI cannot use.

### Claude's reference procedure

For each feature or bug:

1. Find the topic in the selector table below.
2. Open the listed local source files; do not rely only on this summary or a repository README.
3. Label the borrowed idea as **adopt**, **modify**, or **reject** in the implementation notes or PR description.
4. Reimplement the pattern in the target Rust architecture. Copy code only after a license review and explicit approval.
5. Add a focused test that proves the target behavior. Reference implementations are evidence, not correctness oracles.
6. For FFmpeg filters, print/log the final argument vector and verify the generated media with `ffprobe`. Never trust a copied filter string without an empirical test.
7. Keep `plan/*` read-only. New application code belongs outside the reference checkouts.

Useful local searches:

```bash
rg -n "zoompan|scale=.*crop|xfade" plan/ffmpeg-ai plan/Automated-Video-Generator
rg -n "FfmpegProgress|quit\(|kill\(|runFfmpegWithProgress|AbortController" plan/ffmpeg-sidecar plan/lossless-cut
rg -n "generate_handler|Channel|Emitter|emit_to|RunEvent::Exit" plan/tauri
rg -n "externalBin|sidecar|allow-execute|ScopeAllowedArg" plan/example-tauri-v2-python-server-sidecar plan/plugins-workspace
rg -n "custom_audio|audio_duration|silent|elevenlabs" plan/MoneyPrinterTurbo/app
```

## 2. Scan scope and pinned snapshots

The scan was performed on 2026-08-24 against the local shallow checkouts. `editly` was added in a follow-up pass on the same day and is summarized in section 5.10. File counts are tracked files, not generated files. Commit hashes make every path in this document reproducible even if a checkout is updated later.

| Repository | Branch / commit | Tracked files | Source files / lines | Local size | License | Primary use |
|---|---:|---:|---:|---:|---|---|
| `plan/tauri/` | `dev` / `56d19c39e457` | 1,124 | 539 / 120,311 | 38 MB | MIT or Apache-2.0 | Tauri core, IPC, events, app lifecycle, capabilities, packaging |
| `plan/ffmpeg-sidecar/` | `main` / `f56a5e127b93` | 48 | 36 / 5,936 | 544 KB | MIT | Rust FFmpeg process wrapper, progress, stderr events, cancellation |
| `plan/lossless-cut/` | `master` / `e1f434863757` | 324 | 200 / 30,125 | 12 MB | GPL-2.0-only | Production FFmpeg workflow and desktop media UX; patterns only |
| `plan/plugins-workspace/` | `v2` / `db9c5998feff` | 1,448 | 447 / 47,719 | 19 MB | MIT or Apache-2.0 | Tauri store, updater, dialog, fs, shell, window state, Stronghold |
| `plan/remotion/` | `main` / `05075f384a0a` | 13,305 | 9,028 / 1,264,105 | 1.2 GB | Custom Remotion license | Timing and composition concepts only; no V1 dependency/code copy |
| `plan/example-tauri-v2-python-server-sidecar/` | `main` / `40ff11b0746a` | 82 | 10 / 672 | 3.9 MB | Apache-2.0 | Small Tauri 2 external-binary lifecycle example |
| `plan/MoneyPrinterTurbo/` | `main` / `57f83e06a271` | 202 | 99 / 42,757 | 346 MB | MIT | Audio-first duration, supplied audio, TTS providers, subtitle/audio flow |
| `plan/ffmpeg-ai/` | `main` / `6f419564e3ee` | 55 | 33 / 7,933 | 13 MB | MIT | Compact image/audio-to-video pipeline and motion formulas |
| `plan/Automated-Video-Generator/` | `main` / `9cb0c33cd4b8` | 1,105 | 782 / 118,391 | 72 MB | MIT | Scene state, job/cache patterns, segmented rendering, FFmpeg failure lessons |
| `plan/editly/` | `master` / `dc46674052ea` | 100 | 39 / 4,917 | 736 KB | MIT | Declarative JSON config shape, audio mixdown, easings. **Frame-server architecture, not an FFmpeg-filter reference** |

Totals: **17,793 tracked files**, including **11,213 source files** and **1,642,866 source lines**.

### Full-code scan method and boundaries

The scan did not stop at README files. For every checkout it:

1. Enumerated every tracked path with Git, so ignored build output did not distort the inventory.
2. Included every tracked source file in a full-content pass across Rust, TypeScript/JavaScript, Python, Go, Ruby, PHP, Kotlin, Swift, Objective-C/C headers, Svelte, CSS, HTML, and shell sources.
3. Mapped package/workspace manifests, executable entry points, public modules, subprocess boundaries, state/config code, concurrency/cancellation, path/security logic, and tests.
4. Inspected the implementation files that directly affect the reconciled product architecture, then recorded exact starting paths below.
5. Inventoried lockfiles, fonts, icons, screenshots, sample audio/video, compiled native artifacts, and generated data, but did not treat binary/media bytes as application logic.

“Scanned” therefore means every tracked code file was included in the repository-wide inventory and content search. It does not mean every one of the 1.64 million lines is equally relevant or safe to copy. The detailed atlas in section 6 records the disposition of the entire source tree, while the selector table identifies the implementation paths Claude should actually open for a task.

Origins:

- `tauri/`: `https://github.com/tauri-apps/tauri.git`
- `ffmpeg-sidecar/`: `https://github.com/nathanbabcock/ffmpeg-sidecar.git`
- `lossless-cut/`: `https://github.com/mifi/lossless-cut.git`
- `plugins-workspace/`: `https://github.com/tauri-apps/plugins-workspace.git`
- `remotion/`: `https://github.com/remotion-dev/remotion.git`
- `example-tauri-v2-python-server-sidecar/`: `https://github.com/dieharders/example-tauri-v2-python-server-sidecar.git`
- `MoneyPrinterTurbo/`: `https://github.com/harry0703/MoneyPrinterTurbo.git`
- `ffmpeg-ai/`: `https://github.com/numbpill3d/ffmpeg-ai.git`
- `Automated-Video-Generator/`: `https://github.com/itsPremkumar/Automated-Video-Generator.git`
- `editly/`: `https://github.com/mifi/editly.git`

`plan/BRIEF_RECONCILIATION.md` also names `edge-tts`, `elevenlabs-python`, `editly`, and `keyring-rs`. Of those, **`editly` is now checked out and reviewed** — see section 5.10 and D-060. `edge-tts`, `elevenlabs-python`, and `keyring-rs` are still absent; use their official published API contracts rather than guessing, and do not claim to have read them.

The reconciliation called `editly` the highest-value missing reference for FFmpeg filter graphs. That premise was wrong: editly does not use FFmpeg filters for motion at all (section 5.10). The production motion recipe was instead settled empirically — see `ffmpeg-findings.md` and D-030…D-034.

## 3. Fast selector: which repository to consult

| Problem | Primary reference | Exact starting points | Decision |
|---|---|---|---|
| FFmpeg child process API | `ffmpeg-sidecar` | `src/command.rs`, `src/child.rs`, `src/event.rs`, `src/iter.rs` | Adopt the typed process boundary; extend for async queue needs |
| FFmpeg stderr/progress/errors | `ffmpeg-sidecar`, `lossless-cut` | `src/log_parser.rs`; `src/main/ffmpeg.ts`, `src/main/progress.ts` | Modify: prefer machine progress where possible and retain raw stderr |
| Cancellation | `ffmpeg-sidecar`, `lossless-cut` | `FfmpegChild::quit/kill/wait`; `abortFfmpegs()` and `AbortController` registry | Adopt graceful-then-force semantics with cleanup of partial output |
| `ffprobe` metadata | `lossless-cut` | `src/main/ffmpeg.ts`, `src/common/ffprobe.ts`, `src/renderer/src/ffmpeg.ts` | Adopt JSON probing, timeouts, typed normalization |
| Sidecar packaging | Tauri example, `lossless-cut` | `src-tauri/tauri.conf.json`, `src-tauri/src/main.rs`; `package.json` FFmpeg resources | Modify for FFmpeg/ffprobe binaries; no localhost server |
| Rust ↔ UI commands and progress | `tauri` | `examples/api/src-tauri/src/lib.rs`, `packages/api/src/core.ts`, `crates/tauri/src/lib.rs` | Adopt commands for requests, channels/events for streaming updates |
| Capability/security model | `tauri`, plugins | `crates/tauri/src/ipc/authority.rs`, `ipc/capability_builder.rs`, shell `src/scope.rs` | Adopt least privilege; keep FFmpeg spawning in Rust core |
| Global preferences | plugins | `plugins/store/README.md`, `plugins/store/src/store.rs` | Adopt only for UI preferences, never project/render state |
| Secrets | plugins plus reconciliation | `plugins/stronghold/*` | Reject for shared CLI credentials; reconciliation selects `keyring-rs` |
| Auto-update | plugins | `plugins/updater/README.md`, `src/config.rs`, `src/updater.rs` | Adopt signed HTTPS updates in the desktop shell |
| File/folder chooser | plugins | `plugins/dialog/*`, `plugins/fs/*` | Adopt native dialogs; keep filesystem access scoped and validated in core |
| Audio-first duration | `MoneyPrinterTurbo`, `ffmpeg-ai` | `app/services/task.py`, `app/services/voice.py`; `video/composer.py` | Adopt duration from the resolved audio artifact, never from text estimates |
| Supplied narration file | `MoneyPrinterTurbo` | `resolve_custom_audio_file()`, `generate_audio()`, `generate_subtitle()` | Adopt path validation and source-independent downstream contract |
| TTS provider boundary | `MoneyPrinterTurbo` | `app/services/voice.py` | Adopt provider dispatch concept; replace giant conditional with Rust trait/enum |
| Silent scene | `MoneyPrinterTurbo` | `is_no_voice()`, `generate_silent_audio()` | Modify to explicit `duration` scene source, not a magic voice name |
| Ken Burns formulas | **`ffmpeg-findings.md` first**, then `ffmpeg-ai`, Automated Video Generator | `ffmpeg-findings.md` §1–§4 and D-030…D-035; then `video/composer.py`; `operations/visual-fx.ts`, `orchestrator/render.ts` | **Settled.** Use the measured recipe. The repo formulas are historical context, and several are wrong |
| Per-scene segments and concat | Automated Video Generator, `ffmpeg-ai`, `lossless-cut` | `orchestrator/render.ts`; `concat_plain()`; concat implementation/search results | Adopt uniform segments + concat demuxer; assert compatibility before joining |
| Job state/restart recovery | Automated Video Generator | `infrastructure/persistence/job-store.ts`, `management/job.ts` | Adopt explicit states and atomic writes; use SQLite per reconciliation |
| Content-addressed cache | Automated Video Generator, `ffmpeg-ai` | `operations/asset-cache.ts`, `_image_cache_signature()` | Modify: hash complete inputs and output-affecting settings, not URLs alone |
| Bounded concurrency | all pipeline repos | AV Generator `wave-scheduler.ts`; `ffmpeg-ai/pipeline.py`; LosslessCut `p-map` calls | Adopt bounded pools; compute render capacity from RAM and filter cost |
| Timing/easing concepts | `remotion` | core `interpolate.ts`, `spring/`, `Sequence.tsx`; transitions package | Translate concepts into deterministic Rust math; no dependency or code copy |
| Review-grid/media UX | `lossless-cut` | renderer `App.tsx`, errors, hooks, `LastCommands.tsx` | Adopt inspectability and precise errors; reject timeline/editor behavior |

## 4. Cross-repository conclusions that override individual examples

### 4.1 Target process boundary

The Rust core owns FFmpeg and ffprobe. It constructs an OS-safe argument vector, spawns a known binary path, streams progress and raw stderr, supports cancellation, checks the exit status, validates the output with ffprobe, and only then commits the scene checkpoint.

The React UI sends typed commands to the Rust shell and receives typed progress events. The frontend must not receive a general shell permission and must not construct FFmpeg command lines.

### 4.2 Audio source contract

Model scene audio as one closed, explicit input abstraction with at least:

```text
Tts { text, provider, voice, settings }
File { original_path }
Silent { duration }
```

Every variant resolves to a normalized local audio artifact plus authoritative duration. Everything after resolution consumes the same result and does not branch on origin. Preserve supplied originals and normalize into the cache/work area.

### 4.3 Human manifest versus machine state

Do not mutate the user's manifest with render status. The reconciliation requires two artifacts:

- `project.yaml`: human-owned, editable and diffable input.
- `.kenburns/state.db`: machine-owned, disposable render state including per-scene status, cache keys, durations, segment paths, attempts, and failure details.

Use one database transaction per state transition/checkpoint. Rebuild state from the manifest and cache if the database is lost.

### 4.4 Segment contract

Render one file per scene and concatenate only after all required segments are valid. Pin one canonical segment profile in code and verify it before concat:

- codec/profile/level and pixel format
- width, height, SAR, frame rate, and time base
- audio codec, sample format, sample rate, channel count/layout, and time base
- container/stream ordering and timestamp policy

Hard cuts use concat demuxer plus stream copy. Crossfades are a separate slower mode because `xfade` forces a composed filter graph/re-encode and scales poorly with hundreds of scenes.

### 4.5 Ken Burns safety rules — **settled empirically; see `ffmpeg-findings.md`**

> This section asked for a benchmark. The benchmark was run on 2026-08-24 and the
> recipe is now decided in D-030 through D-035. Three of the warnings below turned
> out to be wrong. Read `ffmpeg-findings.md` before touching a filter graph, and
> treat the rest of this section as the reasoning that motivated the measurement.

Still true:

- `zoompan`'s `d` is output frames per input frame, and the form matters. Measured: a single still with no `-loop` and `d=N` bounded by `-frames:v N` is correct and cannot hang; `-loop 1` with `d=N` and **no** `-t` runs forever — 8,400 frames and 312 MB in 25 seconds. That unbounded form is the "5-hour hang" Automated Video Generator documents, not `zoompan` itself.
- Motion selection must be deterministically seeded from stable project/scene data. `random.choice()` without a seed is not acceptable for resumable or cacheable rendering.
- No black edge may enter the output — though the fix is structural, not clamping. Cover-fit into the prescale canvas **before** `zoompan`, and the failure mode disappears (D-034).
- No local repository contains a command that may be copied unmodified. That includes `editly` (section 5.10), which does not use FFmpeg filters for motion at all.

Corrected by measurement:

- ~~`ffmpeg-ai` uses `-loop 1` with `d=<total_frames>`; treat this as an experiment~~ — it also passes `-t`, which bounds it correctly. Its real defect is elsewhere: `composer.py:67` prescales to **2/3 of output resolution** and then upscales back at `:145`.
- ~~Fixed `scale=8000:-1` is prohibited because of memory~~ — prohibited, but for the wrong reason. Prescale is a CPU cost, not a memory cost: 1× → 6× moves peak RSS by 8 % and wall time by 288 %. The correct value is `3 * output_height`, which is where stepping stops (300/300 unique frames) and above which nothing improves.
- ~~Compare `zoompan` against time-based `scale` + `crop`; the latter may win~~ — compared. `zoompan` won by 7.7× on wall time and 2× on peak memory. Automated Video Generator switched to `scale` + `crop` to escape the unbounded-`-loop` hang, not because it was faster.

Still required, and now encoded as the M1 exit gate in `plan.md`: three durations, two FPS values, all V1 aspect ratios, landscape/portrait/square sources, and both short and Unicode/spaced input paths.

### 4.6 Concurrency and recovery

Never use unbounded `Promise.all`, `asyncio.gather`, `join_all`, or one-thread-per-scene behavior over a 500-scene project.

- TTS has a provider-specific queue, concurrency cap, retry/backoff, and rate-limit response handling.
- Render work has a separate bounded pool sized from available memory as well as CPU.
- A cancellation request stops admitting new work, asks active FFmpeg children to quit gracefully, force-kills after a deadline, removes/marks partial files, and persists a resumable state.
- Checkpoint only after the segment passes ffprobe validation and is atomically moved into its final cache/segment path.

### 4.7 Licensing boundary

Patterns are not permission to copy code.

- LosslessCut is GPL-2.0-only: study behavior and UX, but do not copy code into a proprietary-capable product without a deliberate licensing decision.
- Remotion has a custom commercial license and is explicitly rejected as a runtime dependency. Do not copy implementation code.
- FFmpeg distribution licensing depends on the selected build configuration. Record the exact binary source, build flags, codecs, notices, and GPL/LGPL decision before packaging.
- MIT/Apache reference code still requires attribution/license compliance when copied.

## 5. Repository summaries

### 5.1 `ffmpeg-sidecar`: primary Rust subprocess reference

What it is: a small Rust crate around a standalone FFmpeg CLI. It is the cleanest local example of keeping FFmpeg behind a typed process boundary.

Source map:

- `src/command.rs`: `FfmpegCommand` builder, `OsStr` path arguments, raw `arg/args` escape hatch, sidecar path selection, no-console-window behavior on Windows.
- `src/child.rs`: `FfmpegChild`, stdout/stderr/stdin ownership, graceful `q`, forced `kill`, and `wait` deadlock prevention.
- `src/event.rs`: typed version, stream, duration, log, progress, raw-frame, chunk, error, and completion events.
- `src/iter.rs`: concurrent stdout/stderr reader threads and filtered iterators.
- `src/log_parser.rs`: human stderr parsing into semantic events.
- `src/metadata.rs`: incremental metadata accumulator.
- `src/ffprobe.rs` and `src/paths.rs`: adjacent-binary discovery with PATH fallback.
- `src/download.rs`: per-platform binary download/unpack implementation.
- `examples/progress.rs`, `examples/ffprobe.rs`, `examples/download_ffmpeg_with_progress.rs`: small usage examples.

Adopt:

- Use `std::process::Command`/argument arrays, never shell-string concatenation.
- Keep a child handle and expose graceful quit, forced kill, and wait separately.
- Preserve raw stderr while also emitting typed progress/error events.
- Hide extra console windows on Windows.
- Place known FFmpeg/ffprobe binaries next to, or in a deterministic resource path relative to, the executable.

Modify or reject:

- The crate is blocking/synchronous. The target queue needs an async-safe supervisor or a dedicated blocking worker boundary.
- Human FFmpeg stderr formats can change. Prefer `-progress pipe:<fd>`/machine key-value progress for owned commands, with raw stderr retained for diagnostics.
- `src/ffprobe.rs` only checks/version-probes; it is not the full JSON metadata layer the project needs.
- Runtime auto-download is not automatically suitable for a commercial desktop app. Prefer bundled, pinned, checksum/signature-verified binaries or an explicit installer flow.
- Do not let a parser-classified log level replace exit-status and output validation.

### 5.2 `lossless-cut`: production media behavior and UX reference

What it is: a mature Electron/React FFmpeg desktop application. It is valuable because it handles ugly media and desktop edge cases, not because its editor architecture matches `spoonstill`.

Source map:

- `src/main/ffmpeg.ts`: packaged binary resolution, FFmpeg/ffprobe execution, timeouts, progress, global child registry, cancellation, waveform/scene/silence/thumbnail operations.
- `src/main/progress.ts`: video and audio-only progress-line parsing with bounds handling.
- `src/common/ffprobe.ts`: extensive typed ffprobe JSON schema.
- `src/renderer/src/ffmpeg.ts`: metadata normalization, probing, bounded parallel reads, and media-specific operations.
- `src/renderer/src/hooks/useFfmpegOperations.ts`: command construction for export/remux/concat flows.
- `src/renderer/src/App.tsx`: large-project media loading, precise errors, warnings, progress, and operation lifecycle.
- `src/renderer/src/LastCommands.tsx`: operator-visible last FFmpeg command log.
- `src/main/configStore.ts`: application preferences and migration/backup behavior.
- `package.json`: per-platform FFmpeg artifacts, app resources, macOS hardened runtime/notarization, Windows packaging, and license generation.

Adopt:

- Fail fast when the configured binary does not exist and give a specific path error.
- Use ffprobe JSON with timeouts and typed normalization rather than trusting extensions.
- Maintain a registry of active FFmpeg children so app-level cancellation and shutdown can reach all of them.
- Log a safely escaped display form of each command while executing an argument array.
- Carry actual stderr into a detailed error surface; show warnings separately from fatal errors.
- Bound metadata/proxy/segment operations rather than starting all files at once.
- Let operators inspect the actual FFmpeg command and the file/stream metadata that caused a failure.
- Package, sign, notarize, and test each platform artifact explicitly.

Modify or reject:

- Electron, timeline editing, keyframe cutting, and multi-track editing are out of scope.
- GPL-2.0-only means implementation code is not a casual copy source.
- LosslessCut's packaged FFmpeg artifacts include GPL builds; `spoonstill` must make its own documented FFmpeg licensing choice.
- Its global abort registry is useful, but `spoonstill` also needs per-job and per-scene cancellation ownership.

### 5.3 `tauri`: authoritative shell/IPC/lifecycle reference

What it is: Tauri 2 itself. Use it to understand framework contracts rather than infer behavior from small third-party examples.

Source map:

- `examples/api/src-tauri/src/lib.rs`: app setup, managed state, `Channel`, event listen/emit, typed commands, window creation, and `RunEvent` handling.
- `packages/api/src/core.ts`: frontend `invoke()` and ordered `Channel` delivery semantics.
- `crates/tauri/src/lib.rs`: `Manager`, capability loading, `Listener`, `Emitter`, and targeted `emit_to` APIs.
- `crates/tauri/src/app.rs`: application run loop, `ExitRequested`, cleanup, restart, and `run_return` behavior.
- `crates/tauri/src/ipc/authority.rs`: runtime authorization decisions and capability diagnostics.
- `crates/tauri/src/ipc/capability_builder.rs`: local/remote context, window/webview labels, permissions, scopes, and platforms.
- `ARCHITECTURE.md`: roles of core, runtime, WRY, TAO, JS API, CLI, and bundler.
- `crates/tauri-bundler/` and `crates/tauri-cli/`: packaging implementation when a build detail is unclear.

Adopt:

- Keep the Tauri layer thin: deserialize a typed request, call the core, and translate results/events.
- Use commands for control-plane requests and ordered channels or targeted events for render progress/log streams.
- Store queue/supervisor handles in managed Rust state, not in React state.
- Scope events to the requesting window/job when practical rather than broadcasting all progress globally.
- Handle close/exit explicitly so active children receive cancellation and state is flushed.
- Grant capabilities to named windows and exact commands only.

Modify or reject:

- Do not expose a general shell or broad filesystem API to the webview just because Tauri can.
- Do not make the desktop shell required for the core/CLI to work.
- Examples often demonstrate many APIs at once; reduce the target surface to the minimum needed for project selection, preview, progress, cancellation, and settings.

### 5.4 `plugins-workspace`: official Tauri desktop integrations

What it is: the official Tauri v2 plugin monorepo. The relevant subset is much smaller than the repository.

Relevant plugin map:

- `plugins/store/`: JSON-compatible persistent key/value preferences, Rust/JS interoperability, debounced autosave.
- `plugins/updater/`: desktop-only signed update checks/download/install; HTTPS is enforced by default in release builds.
- `plugins/dialog/`: native open/save/message dialogs.
- `plugins/fs/`: scoped filesystem APIs and watchers.
- `plugins/shell/`: sidecar and process spawning, stdout/stderr/termination events, fixed/regex-validated arguments, environment/cwd controls.
- `plugins/window-state/`: window size/position persistence.
- `plugins/stronghold/`: encrypted secret database with a password-to-key callback.
- `plugins/log/`, `plugins/process/`, `plugins/single-instance/`, and `plugins/opener/`: useful later for desktop hardening.

Adopt:

- Store global UI preferences separately from copyable project data.
- Use the native dialog plugin to choose project/manifest folders.
- Configure updater endpoints and signing keys before release; reject insecure transport flags in production.
- Persist window state as a shell concern.
- If the frontend ever spawns an allowed helper, use exact command scopes and fixed/validated arguments from `plugins/shell/src/scope.rs`.

Modify or reject:

- Per `plan/BRIEF_RECONCILIATION.md`, Stronghold loses to `keyring-rs` for API keys because the CLI must access the same credentials without depending on Tauri. Stronghold remains a study reference, not the selected credential store.
- Store is not a transactional render-state database and must not replace `.kenburns/state.db`.
- The preferred architecture spawns FFmpeg inside Rust core, so the UI should not need shell spawn permissions at all.
- Avoid broad `fs` scopes; pass selected paths into validated Rust commands.

### 5.5 `example-tauri-v2-python-server-sidecar`: packaging/lifecycle shape only

What it is: a deliberately small Tauri 2 + Next.js app that bundles a PyInstaller FastAPI binary. Its sidecar lifecycle shape is relevant even though the target sidecar is FFmpeg, not Python.

Source map:

- `src-tauri/tauri.conf.json`: `bundle.externalBin` declaration and platform bundle settings.
- `src-tauri/capabilities/migrated.json`: sidecar execute permission shape.
- `src-tauri/src/main.rs`: `CommandChild` in managed state, duplicate-spawn guard, stdout/stderr monitoring, frontend events, stdin shutdown, and exit handling.
- `package.json`: target-triple sidecar naming and per-platform build scripts.
- `src/backends/main.py`: child self-shutdown protocol and localhost server startup.

Adopt:

- External binary names are resolved to target-specific bundle artifacts.
- Retain the child handle in managed application state.
- Forward sidecar stdout/stderr as structured events.
- Make startup idempotent and shut children down on application exit.

Modify or reject:

- `spoonstill` talks directly to FFmpeg/ffprobe; it does not need a localhost HTTP server, CORS, a fixed port, or a Python bootloader.
- Do not copy the example's wildcard CORS, `csp: null`, broad `http://**/` and `https://**/` permissions, broad shell defaults, or swallowed startup errors.
- Add readiness/failure reporting, timeout handling, exit-code handling, and force-kill fallback; the example is too small to be a production supervisor.

### 5.6 `ffmpeg-ai`: compact applied pipeline reference

What it is: a Python CLI that turns generated text/audio/images into short or landscape video. Its compactness makes data flow easy to see, but it is not hardened for 500-scene deterministic rendering.

Source map:

- `src/ffmpeg_ai/video/composer.py`: ffprobe duration, Ken Burns strings, per-image clips, concat demuxer, xfade, audio merge, music ducking, captions, and final encode.
- `src/ffmpeg_ai/pipeline.py`: render plan, cache signatures, TTS/image stages, worker pool, report generation, fast-preview/storyboard/kenburns modes.
- `src/ffmpeg_ai/ai/tts.py`: Edge-TTS request/retry behavior.
- `src/ffmpeg_ai/video/captions.py`: local Whisper-to-ASS/SRT captions.
- `src/ffmpeg_ai/video/shorts.py`: centralized output specs.
- `tests/test_pipeline_failures.py` and `tests/test_pipeline_reporting.py`: failure/report expectations.

Adopt:

- Probe the rendered/generated audio and let its duration drive visual timing.
- Keep FFmpeg calls in one media boundary and pass argument lists.
- Centralize output resolution/FPS/codec settings.
- Offer a cheap proof/preview path that still exercises the real FFmpeg pipeline.
- Produce a machine-readable run report with cache hits, failures, provider attempts, and artifact paths.
- Sidechain ducking and ASS/SRT generation are useful V1.1 references.

Modify or reject:

- `_kenburns_filter()` is not production-approved; it participates in the `zoompan d` footgun described above.
- Motion and transitions use unseeded randomness, breaking reproducibility and cache stability.
- `ThreadPoolExecutor(max_workers=4)` ignores available RAM and filter cost.
- `asyncio.gather()` starts all TTS requests together and has no provider semaphore.
- Scene clips live in a temporary directory, so a crash loses completed work.
- The xfade graph grows with clip count and is unsuitable as the 500-scene default.
- Concat list quoting is too simplistic for every hostile filename; generate it defensively or avoid exposing original paths in the list.
- Edge-TTS is an undocumented fallback/reference and cannot be the load-bearing sold-product provider without an explicit distribution policy.

### 5.7 `Automated-Video-Generator`: rich lessons, not a blueprint

What it is: a large TypeScript/Electron/Remotion/FFmpeg application with multiple old and new pipelines. It contains valuable production lessons and tests, but its parallel architectures and documentation drift make it unsafe to copy wholesale.

Architecture/source map:

- `src/adapters/`, `src/application/`, `src/infrastructure/`, `src/shared/`: current hexagonal layering direction.
- `src/agentic/orchestrator/pipeline.ts` and `render.ts`: plan/acquire/verify/gate/render flow and segmented FFmpeg render path.
- `src/agentic/operations/visual-fx.ts`: input-aware `zoompan` discussion and regression tests.
- `src/agentic/orchestrator/render.ts`: known `zoompan` memory/hang bugs, streaming `scale+crop` workaround, per-scene segment normalization, captions, SAR pinning, and filter-order lessons.
- `src/infrastructure/persistence/job-store.ts`: normalized persisted job records, atomic temp-file rename, and interrupted-job recovery.
- `src/agentic/management/job.ts`: explicit legal job-state transitions.
- `src/agentic/operations/asset-cache.ts`: URL-indexed cache with file size/hash metadata.
- `src/agentic/management/render-ledger.ts`: bounded, atomic, local outcome history.
- `src/agentic/operations/wave-scheduler.ts`: bounded-wave concept and filename sanitization.
- `src/lib/path-safety.ts` and `src/agentic/operations/security.ts`: path boundary and extension validation.
- `src/agentic/pipeline/gate.ts` plus media analyzer/tests: pre/post render validation concepts.
- `tests/transitions.test.ts` and `src/agentic/operations/visual-fx.test.ts`: FFmpeg filter regression checks.
- `tools/asset-creator/src/index.js`: small worked FFmpeg examples; validate before use.

Adopt:

- Use explicit job states and legal transitions rather than scattered booleans.
- Persist enough request/state to explain and recover interrupted jobs.
- Write state atomically and tolerate a corrupt optional cache/ledger without corrupting source manifests.
- Normalize every per-scene segment and re-pin SAR at the end of motion filters.
- Validate output after rendering: readable file, duration, streams, dimensions, codec/pixel format, audio, black/frozen frames where appropriate.
- Sanitize human titles for output filenames while keeping stable internal IDs separate.
- Keep path resolution and trust-boundary validation centralized.
- Use bounded waves/pools as a concept, but derive capacity from the current machine.

Modify or reject:

- The repository has legacy, agentic, Remotion, FFmpeg, CLI, HTTP, MCP, and Electron paths with duplicated behavior. Do not reproduce this architecture.
- The render code's comments document fixes after real `zoompan` hangs and memory blowups. Prefer its lesson and tests over older formulas elsewhere in the same repo.
- `asset-cache.ts` keys primarily by URL; `spoonstill` segment/TTS cache keys must cover content bytes and every output-affecting setting.
- JSON files and best-effort swallowed persistence errors are not sufficient for the authoritative 500-scene render state; use SQLite.
- `wave-scheduler.ts` contains Windows-specific process enumeration/kill cleanup. Never kill unrelated “RAM-hogging” processes; supervise only children owned by the current job.
- Do not adopt image generation, stock search, agentic script writing, cloud upload, video scenes, timeline features, or Remotion rendering. They are outside V1.

### 5.8 `MoneyPrinterTurbo`: audio source and synchronization reference

What it is: a Python/FastAPI/Streamlit/MoviePy application with a broad provider catalog. It is strongest as a reference for narration selection, duration, subtitles, and defensive local-file handling.

Source map:

- `app/models/schema.py`: video request/aspect/transition/material schemas.
- `app/services/task.py`: ordered orchestration; `resolve_custom_audio_file()`, `generate_audio()`, `generate_subtitle()`, material resolution, and final video stages.
- `app/services/voice.py`: provider detection/dispatch, Edge/Azure/ElevenLabs and other TTS adapters, silent audio, cue/subtitle creation, and duration probing.
- `app/services/video.py`: codec probing/fallback, concat path handling, input sanitization, composition, subtitles, audio/BGM mix, and resource cleanup.
- `app/services/state.py`: in-memory versus Redis task state and atomic partial patching.
- `app/services/task_artifacts.py`: atomic artifact metadata writes.
- `app/services/cache_manager.py`: bounded/safe managed-cache scanning and cleanup.
- `test/services/test_voice.py`, `test_task.py`, `test_video.py`, and custom-audio/security tests: expected failure cases.

Adopt:

- Accept a supplied narration file as a first-class alternative to TTS.
- Validate that untrusted file paths remain within an allowed task/project root and do not leak host path existence.
- Probe duration from the actual audio artifact; reject zero/invalid duration.
- Generate subtitles from provider timing when available, or explicitly use local transcription for supplied audio. Do not fabricate a provider cue timeline.
- Keep provider-specific credentials, voices, requests, and failures behind a provider boundary.
- Release media readers/FFmpeg children in all success and failure paths, especially on Windows where open handles block replacement/deletion.
- Validate hardware encoder availability and fall back cleanly.

Modify or reject:

- The current provider dispatch is a very large conditional. Implement a Rust trait/registry with typed settings and errors.
- Its “no voice” sentinel creates project-level silent audio from estimated text length. `spoonstill` needs an explicit per-scene `Silent { duration }` source.
- MoviePy and the Python server/UI stack are not target dependencies.
- One monolithic project audio track is not enough for scene-level cache/resume. Normalize and checkpoint audio per scene.
- Configuration files containing API keys are not acceptable for the selected CLI-first architecture; use the OS keyring abstraction.

### 5.9 `remotion`: design language only

What it is: a very large React video-rendering monorepo with composition, sequencing, interpolation, springs, transitions, media, player, studio, and render infrastructure.

Relevant source map:

- `packages/core/src/Composition.tsx`: composition metadata and validation.
- `packages/core/src/Sequence.tsx` and `packages/core/src/series/`: local frame offsets, bounded durations, trimming, nesting, and sequential composition.
- `packages/core/src/interpolate.ts`: range mapping, easing, extrapolation, and clamping semantics.
- `packages/core/src/spring/`: deterministic frame/FPS-based spring calculation and duration measurement.
- `packages/core/src/use-current-frame.ts` and `use-video-config.ts`: the “current frame + immutable video spec” mental model.
- `packages/transitions/src/TransitionSeries.tsx`, `timings/`, and `presentations/`: transition overlap/duration concepts.
- `packages/timeline-utils/`: timeline calculations independent of Studio UI.
- `packages/captions/`, `packages/media-utils/`, and markup skill documents: captions and media technique references.

Adopt conceptually:

- Represent motion as a pure deterministic function of scene-local progress/frame and immutable scene/project parameters.
- Centralize FPS, duration-in-frames, dimensions, and aspect ratio.
- Clamp/extrapolate explicitly rather than relying on accidental filter behavior.
- Model transition overlap in duration math instead of subtracting it ad hoc at concat time.
- Keep reusable motion presets separate from scene data.

Reject:

- No Remotion runtime, renderer, Player, Studio, browser capture, serverless package, or React video composition in V1.
- Do not copy implementation code under the custom Remotion license.
- Remotion timing concepts do not override the target rule that final rendering is native FFmpeg and audio duration is authoritative.

### 5.10 `editly`: a different architecture, reviewed and mostly rejected

What it is: a Node CLI that turns a declarative JSON config into a video. Every prior document in this workspace describes it as missing, and the reconciliation describes it as "declarative JSON → ffmpeg, Ken Burns already implemented" and the highest-value reference for filter graphs.

**Both claims were wrong.** It is checked out (`master` / `dc46674052ea`, 100 tracked files, ~4,900 source lines, MIT), and it is not an FFmpeg-filter tool. It is a **frame server**: it renders every frame in Node with fabric/canvas and GL shaders, then pipes raw RGBA into FFmpeg, which only encodes.

Source map:

- `src/index.ts:224-256`: the whole architecture in one function — `ffmpeg -f rawvideo -vcodec rawvideo -pix_fmt rgba -s WxH -r FPS -i -`, with frames written to stdin at `:442`.
- `src/sources/image.ts`: Ken Burns, implemented as per-frame fabric canvas scaling and translation. No `zoompan`, no `scale`+`crop`.
- `src/util.ts:138-162`: `getZoomParams()` / `getTranslationParams()` — the motion math.
- `src/audio.ts:244-256`: per-track `atrim` / `adelay` / `apad`, then `amix` with weights, `loudnorm`, and an output `volume`.
- `src/easings.ts`: `easeOutExpo`, `easeInOutCubic`, `linear`.
- `src/ffmpeg.ts`: `execa` argument arrays, ffprobe duration/stream reads, rotation from both `tags.rotate` and `side_data_list`, a minimum-version assertion, and `createConcatFile()`.
- `src/transition.ts` and `shaders/`: GL transitions via `gl-transitions`.
- `src/parseConfig.ts`, `src/types.ts`: the declarative config schema.

Reject — the architecture:

- It requires a JS canvas/GL runtime (`canvas`, `fabric`, `gl`, `gl-transitions`) inside the shipping product. That is a large native-dependency surface on Windows and macOS, for a Rust-core product that has no other reason to embed a JS runtime.
- It renders the entire video in **one** FFmpeg process with no per-scene segments. There is no checkpoint, so there is no resume — directly contrary to D-042 and hard requirement 6.
- `getTranslationParams()` computes its pan range as `zoomAmount * 1000` pixels — a resolution-blind magic number. At 1080p and at 4K the same config pans a different fraction of the frame.
- `getZoomParams()` returns a flat `1.3 + zoomAmount` for left/right pans, so a pan is a static zoom plus a translation rather than a coherent Ken Burns move.

Adopt — three specific things:

- **The declarative config shape** (`src/types.ts`, `src/parseConfig.ts`) as prior art for `project.yaml`. It is the closest local example of a JSON/YAML document describing a whole video, and it is worth reading before finalizing the manifest schema.
- **`src/audio.ts`** for the V1.1 music bed. The `atrim` → `adelay` → `apad` → `amix` → `loudnorm` chain, and the comment at `:244` explaining why the first track must not be padded, is the cleanest local reference for mixing tracks of unequal length.
- **`src/easings.ts`** — three easing functions, trivially portable to Rust, useful for D-030's `z` expression.

Note for later: the frame-server approach is the *correct* answer if motion ever needs to exceed what FFmpeg filter expressions can describe. It is recorded here as a deliberate V2+ escape hatch so it is not rediscovered from scratch. See D-060.

Also note `createConcatFile()` at `src/ffmpeg.ts:46-52`: it quotes with `seg.replace(/'/g, "'\\''")`, which is better than `ffmpeg-ai`'s version and still too optimistic for arbitrary operator filenames. Prefer stable internal segment names in the concat list and keep human-facing names out of it entirely.

## 6. Full-code atlas and source-tree disposition

This section prevents Claude from mistaking a selective “important files” list for the whole repository. Each atlas accounts for the executable surfaces, source roots, tests, and code that is intentionally out of scope.

### 6.1 `Automated-Video-Generator` — 782 source files / 118,391 lines

Executable surfaces:

- `src/server.ts` starts the main TypeScript server; `src/adapters/http/` exposes HTTP controllers/routes.
- `bin/mcp.js` is the published command entry; `src/adapters/mcp/` exposes MCP tools and stores.
- `src/adapters/cli/` contains the modular, batch, preview, editor, image, audio, cleanup, and job CLIs.
- `electron/` and the `electron:*` package scripts form the desktop shell.

Source-tree disposition:

- `src/agentic/orchestrator/`: primary render-flow reference. Read `pipeline.ts`, `render.ts`, `ffmpeg.ts`, `artifacts.ts`, `source.ts`, and `types.ts` together; tests beside them capture cleanup, dimensions, captions/SFX, and regressions.
- `src/agentic/pipeline/`: plan/acquire/verify/gate stages and their tests. Reuse the stage-boundary and validation ideas, not the agentic feature scope.
- `src/agentic/operations/`: 68 tracked files covering compose, probe, motion, visual FX, audio, captions, security, path handling, retries, wave scheduling, and many unrelated creator features. The relevant subset is `visual-fx.ts`, `compose.ts`, `compose-scene-fx.ts`, `probe.ts`, `audio-track.ts`, `captions.ts`, `security.ts`, `retry.ts`, and `wave-scheduler.ts` plus adjacent tests.
- `src/agentic/management/`: job state, workspace, ledger, cleanup, and autopilot. Use the explicit state/ledger concepts; reject autopilot ownership of product state.
- `src/infrastructure/`: local filesystem, JSON job persistence, and scene-editor adapter. Use atomic-write/recovery lessons; replace authoritative JSON state with SQLite.
- `src/adapters/`: CLI is conceptually relevant; HTTP/MCP are evidence of adapter separation but are not required product surfaces.
- `src/speech/`: routes, services, backends, database, and utilities for speech providers. Consult only when defining provider error/normalization behavior; do not bring its Python service topology into the Rust core.
- `src/music-system/`, stock/image/video search libraries, uploader/downloader submodules, AI prompt/script code, and social-delivery code are outside V1.
- `src/views/`, `electron/`, `remotion/`, and `remotion-creation/` are UI/desktop/browser-render paths from this application, not the target Tauri/FFmpeg architecture.
- `sub-modules/`, `tools/`, `input/`, `assets/`, and `samples/` are bundled helpers, fixtures, media, and examples. They are not shared core modules.
- `src/**/*.test.ts` and `tests/` include the highest-value regression evidence: motion/filter construction, output dimensions, transitions, composition, path/filename safety, failure cleanup, and queue behavior.

Claude must search both `src/agentic/orchestrator/` and `src/agentic/operations/` before using a filter: older and newer pipelines coexist, and a formula in one path may be contradicted by a documented production failure in another.

### 6.2 `MoneyPrinterTurbo` — 99 source files / 42,757 lines

Executable surfaces:

- `cli.py` is the complete argparse CLI and converts arguments into `VideoParams`.
- `main.py`, `app/asgi.py`, and `app/router.py` provide the FastAPI process and routing.
- `webui/Main.py` is the Streamlit interface. It is large UI/application glue and is not the target frontend pattern.

Source-tree disposition:

- `app/models/`: request schemas, enums/constants, provider definitions, and exceptions. `schema.py` is the best way to understand the data passed into services.
- `app/config/`: configuration load/default behavior. Study option naming only; never copy secret-file storage into the target.
- `app/controllers/`: HTTP validation and memory/Redis task managers. The APIs are not a target dependency, but their boundary/error tests expose useful hostile-input cases.
- `app/services/task.py`: authoritative orchestration path for this repository. It resolves narration, subtitles, materials, audio duration, and final composition in order.
- `app/services/voice.py`, `subtitle.py`, `video.py`, and `bgm.py`: primary audio/media behavior. Read these as one flow because cue generation, probed duration, video length, and muxing depend on each other.
- `app/services/state.py`, `task_artifacts.py`, `cache_manager.py`, `material_cache.py`: persistence/cache patterns. Only atomic artifact writes and bounded safe cleanup carry over; Redis/in-memory state and cache schemas do not.
- Provider/content integrations (`llm.py`, `loomloom.py`, `sonilo.py`, `twelvelabs.py`, `elevenlabs_music.py`, uploads/posts/material search) are outside the requested V1.
- `app/utils/file_security.py` is a mandatory companion to custom-audio handling because it enforces the allowed path boundary.
- `test/services/` has 50+ focused service suites. Before implementing supplied audio, silent audio, subtitles, cache cleanup, path validation, or CLI parsing, read the same-named test module as well as the service.

### 6.3 `example-tauri-v2-python-server-sidecar` — 10 source files / 672 lines

The complete executable flow is small enough to state end to end:

1. Next.js `app/page.tsx` invokes the Rust commands and listens for backend output.
2. `src-tauri/src/main.rs` resolves/spawns the external binary, stores `CommandChild`, forwards output, prevents duplicate startup, sends a stdin shutdown command, and reacts to Tauri exit.
3. `src/backends/main.py` starts FastAPI/uvicorn and `src/backends/inference` demonstrates a backend route.
4. `package.json` builds target-triple-named PyInstaller binaries.
5. `src-tauri/tauri.conf.json` bundles the external binary and `src-tauri/capabilities/migrated.json` grants its execute scope.

There are no automated tests. Treat the entire repository as a lifecycle/configuration example and apply the production gaps listed in section 5.5.

### 6.4 `ffmpeg-ai` — 33 source files / 7,933 lines

Executable surfaces and modules:

- `src/ffmpeg_ai/cli.py` and `__main__.py`: Typer commands, validation, async runner, status/report presentation.
- `src/ffmpeg_ai/gui.py` and `ui/`: desktop GUI and display widgets; useful only for seeing exposed operations.
- `src/ffmpeg_ai/pipeline.py`: central orchestration and cache/report logic.
- `src/ffmpeg_ai/video/`: composer, captions, shorts output specs, and thumbnails. `composer.py` is the FFmpeg boundary.
- `src/ffmpeg_ai/ai/`: TTS, image generation, and OpenRouter text calls. Only `tts.py` informs the audio-provider boundary.
- `src/ffmpeg_ai/auto/` and `channels/`: YouTube/music/harvest automation and channel runners; outside V1.
- `tests/`: channel config, CLI async behavior, GUI, pipeline failure/reporting, script adaptation, and thumbnails. The failure/report tests are more relevant than the happy-path README.

Read order for a render issue: `pipeline.py` → `video/composer.py` → `video/shorts.py` → the failure/report tests. Do not consult `_kenburns_filter()` without also reading section 4.5 of this guide.

### 6.5 `ffmpeg-sidecar` — 36 source files / 5,936 lines

All Rust library modules are accounted for:

- Process core: `command.rs`, `child.rs`, `event.rs`, `iter.rs`, `log_parser.rs`, `metadata.rs`.
- Parsing/helpers: `comma_iter.rs`, `read_until_any.rs`, `ffmpeg_time_duration.rs`, `pix_fmt.rs`, `version.rs`.
- Binary discovery/acquisition: `paths.rs`, `ffprobe.rs`, `download.rs`.
- Optional transport: `named_pipes.rs`.
- Public surface and tests: `lib.rs`, `main.rs`, `test.rs`.

The 15 examples cover basic execution, progress, metadata, ffprobe, download progress, preview, raw frames/terminal video, sockets/named pipes, microphone metering, H.265, triggers, Whisper, and generated frames. For `spoonstill`, start with `progress.rs`, `ffprobe.rs`, `metadata.rs`, and `download_ffmpeg_with_progress.rs`; the remaining examples prove breadth but are not required features.

### 6.6 `lossless-cut` — 200 source files / 30,125 lines

Runtime layers:

- `src/main/`: Electron main process. `index.ts` exposes the IPC surface; `ffmpeg.ts` owns media subprocesses; `progress.ts` parses status; `configStore.ts`, logging, menu, networking, updater, and compatibility code handle desktop lifecycle.
- `src/preload/index.ts`: privileged renderer bridge. Its existence reinforces that media execution should stay outside untrusted UI code.
- `src/common/`: shared types, constants, utilities, and the ffprobe schema.
- `src/renderer/src/ffmpeg.ts`, `ffprobe.ts`, `ffmpegParameters.ts`, and `hooks/useFfmpegOperations.ts`: renderer-side orchestration and command selection.
- `src/renderer/src/components/`, `dialogs/`, and general hooks: error/progress/settings interaction patterns. `LastCommands.tsx` is the most directly reusable UX concept.
- Timeline, EDL, segment editing, smart cut, players, frame workers, and stream-selection UI are product-specific and outside V1.
- `script/` and `package.json` cover artifact creation, license generation, release metadata, icons, and E2E packaging. Use them as a release checklist, not a build-system template.
- Tests span FFmpeg utilities/progress, HTTP utility behavior, EDL/segments, and renderer utilities. The repository also has manual format fixtures; those are evidence for broad compatibility testing.

Always preserve the GPL boundary: descriptions and independently reimplemented behavior are usable; source copying requires a deliberate product-license decision.

### 6.7 `plugins-workspace` — 447 source files / 47,719 lines

All plugin directories were inventoried. Use them in these buckets:

- Directly relevant shell concerns: `dialog`, `fs`, `log`, `opener`, `process`, `shell`, `single-instance`, `store`, `updater`, and `window-state`.
- Possible post-V1 shell concerns: `autostart`, `deep-link`, `notification`, `os`, and `persisted-scope`.
- Outside current scope: `barcode-scanner`, `biometric`, `clipboard-manager`, `geolocation`, `global-shortcut`, `haptics`, `http`, `localhost`, `nfc`, `positioner`, `sql`, `upload`, and `websocket`.
- `stronghold` was inspected but is rejected for shared CLI credentials by the reconciliation decision. `cli` is plugin-development tooling, not the application CLI.

Most plugins repeat a useful structure: Rust `src/` implementation, generated/declared `permissions/`, guest JavaScript bindings, examples, and platform-specific Android/iOS code where applicable. For desktop V1, Claude should inspect the Rust implementation and permission definitions first; mobile implementations and example-app boilerplate are not relevant.

Exact high-value files beyond the earlier summary:

- Shell: `plugins/shell/src/config.rs`, `scope.rs`, `scope_entry.rs`, `commands.rs`, and `process/mod.rs`.
- Filesystem: `plugins/fs/src/scope.rs`, `file_path.rs`, `commands.rs`, and `watcher.rs`.
- Store: `plugins/store/src/store.rs`, `lib.rs`, and `error.rs`.
- Updater: `plugins/updater/src/config.rs`, `updater.rs`, `commands.rs`, and `error.rs`.
- Dialog/window lifecycle: `plugins/dialog/src/desktop.rs`, `commands.rs`; `plugins/window-state/src/lib.rs`, `cmd.rs`.
- Secrets study only: `plugins/stronghold/src/kdf.rs`, `stronghold.rs`, and `lib.rs`.

### 6.8 `remotion` — 9,028 source files / 1,264,105 lines

This is by far the largest checkout. Its complete package tree was indexed, then grouped by architectural role:

- Timing/composition concepts: `core`, `transitions`, `animation-utils`, `timeline-utils`, and `layout-utils`.
- Native/media behavior to study without copying: `renderer`, `compositor*`, `media`, `remotion-media`, `media-parser`, `media-utils`, `webcodecs`, `web-renderer`, `convert`, and `streaming`.
- Caption/transcript formats: `captions`, `openai-whisper`, `elevenlabs`, and `whisper-web`. The local `elevenlabs` package converts transcript data to captions; it is **not** the missing ElevenLabs Python API-client reference.
- Browser authoring/preview stack: `player`, `player-a11y`, `studio`, `studio-server`, `studio-shared`, `studio-protocol`, `browser-studio`, `bundler`, `preload`, and `cli`. These are rejected runtime/architecture dependencies.
- Cloud rendering: `lambda*`, `cloudrun`, `serverless*`, `vercel`, and `dockerfiles`. Entirely outside V1.
- Visual packages such as `effects`, `shapes`, `three`, `skia`, `lottie`, `gif`, `rive`, `noise`, `light-leaks`, fonts, emoji, and design/brand packages are not part of the native Ken Burns renderer.
- Template/example/promo/docs packages demonstrate Remotion products, not target modules. Tooling, lint, compatibility, licensing, skills, and test packages support the monorepo itself.

Additional behavior-only references discovered in the whole-code pass:

- `packages/core/src/random.ts`: deterministic seed-to-number concept. Reimplement independently with a documented stable algorithm; do not inherit an opaque seed scheme accidentally.
- `packages/renderer/src/call-ffmpeg.ts`: executable resolution, argument arrays, explicit environment, and cancellation wiring.
- `packages/renderer/src/parse-ffmpeg-progress.ts`: frame/time fallback parsing; prefer machine progress in the target.
- `packages/renderer/src/validate-concurrency.ts`, `pool.ts`, and `p-limit.ts`: bounded-resource and machine-limit concepts.
- `packages/renderer/src/prestitcher-memory-usage.ts`: explicit free-memory reservation and empirical pixel-cost estimation. Its constant is workload-specific; benchmark the native target rather than copying it.
- `packages/renderer/src/can-concat-seamlessly.ts`, `combine-chunks.ts`, and `combine-video-streams-seamlessly.ts`: codec/segment compatibility concerns.
- `packages/renderer/src/assets/sanitize-filepath.ts` and `test/handle-weird-file-names.test.ts`: hostile filename and URL-derived artifact tests.
- `packages/renderer/src/tmp-dir.ts`, `succeed-or-cancel.ts`, `make-cancel-signal.ts`, and `write-with-backpressure.ts`: cleanup/cancellation/backpressure concepts.

These extra references do not change the license decision: Remotion implementation code is not a copy source and Remotion remains excluded from the V1 runtime.

### 6.9 `tauri` — 539 source files / 120,311 lines

Complete workspace disposition:

- `crates/tauri`: application API. Relevant modules are `app.rs`, `state.rs`, `ipc/`, `event/`, `path/`, `scope/`, `process.rs`, and the manager/window/webview lifecycle.
- `crates/tauri-runtime` and `tauri-runtime-wry`: runtime traits and WRY platform implementation. Consult only when documented high-level lifecycle/event behavior is ambiguous.
- `crates/tauri-build`, `tauri-codegen`, `tauri-macros`, and `tauri-plugin`: compile-time command/config/plugin machinery. Usually framework internals, not target code.
- `crates/tauri-bundler`, `tauri-cli`, `tauri-macos-sign`, `tauri-driver`, schema crates, and `packages/cli`: packaging, signing, CLI, driver, and schema infrastructure. Consult for release/build questions only.
- `crates/tauri-utils`: shared config/ACL/assets/platform types. It is useful for understanding capabilities, not a place for product business logic.
- `packages/api`: frontend invoke/channel/event/window API; `packages/api/src/core.ts` is the primary IPC companion to Rust commands.
- `crates/tests` and bench directories are framework validation, not application tests.

All example groups were inventoried: `api`, `commands`, `state`, `streaming`, `run-return`, `multiwindow`, `multiwebview`, `splashscreen`, `isolation`, `file-associations`, `drag`, and `helloworld`. For this product, read `commands`, `state`, `streaming`, and the exit handling in `api`; use `run-return` only if the desktop lifecycle truly requires returning control to the host process.

The concrete Tauri implementation path should stay narrow: typed `#[tauri::command]` control calls, managed supervisor state, `ipc::Channel` or targeted events for progress, explicit exit cleanup, and least-privilege capabilities. Window/menu/tray/mobile/runtime internals are not reasons to move core rendering into the shell.

## 7. Suggested target-module-to-reference map

Names may change when the authoritative master brief is supplied, but the dependency direction should not.

```text
core domain
  project/manifest      <- reconciliation + PROJECT_BRIEF pairing convention
  scene/audio_source    <- MoneyPrinterTurbo lessons, implemented as Rust enum/trait
  motion                <- Remotion concepts + empirically verified FFmpeg recipes
  cache_keys            <- AV Generator/ffmpeg-ai lessons, complete canonical hashing

application services
  import/resolve        <- PROJECT_BRIEF convention/CSV rules + path safety lessons
  audio_queue           <- provider caps/retry/backoff; independent of render queue
  render_queue          <- bounded worker pool + persistent SQLite checkpoints
  preview               <- same renderer, low-res/single scene

infrastructure
  ffmpeg_process        <- ffmpeg-sidecar + LosslessCut
  ffprobe               <- LosslessCut typed JSON probing
  state_db              <- reconciliation requirement + AV Generator state-machine lessons
  keyring               <- missing keyring-rs reference; keep behind a trait
  tts/elevenlabs        <- missing elevenlabs-python reference; use official API contract
  tts/edge              <- optional/internal fallback behind the same trait

adapters
  cli                   <- permanent complete control/test surface
  tauri                 <- Tauri commands/channels/events, no business logic
  react                 <- review grid and settings only
```

Required dependency direction:

```text
React -> Tauri adapter -> application services -> core domain
CLI --------------------^                       -> infrastructure traits
```

The core domain must not import Tauri, React, a TTS provider SDK, or concrete process/UI code.

## 8. Implementation gates Claude must enforce

Before calling a vertical slice complete:

- A three-scene headless CLI project supports one TTS scene, one supplied-audio scene, and one silent-duration scene.
- Filenames include spaces and Unicode; commands are argument arrays and work on both supported path styles.
- Audio is normalized and probed before motion is computed.
- Every scene segment is independently rendered, ffprobed, atomically checkpointed, and reusable after restart.
- Re-running unchanged input produces cache hits and deterministic motion choices.
- Changing one scene invalidates only that scene's audio/segment and the final concat artifact.
- Cancellation leaves no valid-looking partial segment and a subsequent run resumes safely.
- Concat rejects a deliberately mismatched segment before creating the final file.
- Motion tests cover duration/FPS/aspect/source-shape matrix and prove no black edges.
- Preview uses the same FFmpeg recipe at reduced scale and cannot mutate final render state.
- Raw FFmpeg stderr is retained with a scene ID and a human-readable summary.
- Secrets never appear in logs, manifests, cache keys, command arguments, or project files.
- The selected FFmpeg binary and all copied/reference-derived code have a recorded license decision.

## 9. Coverage checklist

- [x] Tauri core and examples
- [x] ffmpeg-sidecar source and examples
- [x] LosslessCut process, probe, renderer workflow, packaging, and license
- [x] Tauri official plugins, including store/updater/dialog/fs/shell/window-state/Stronghold
- [x] Remotion core timing/composition/transition areas and custom license
- [x] Tauri v2 sidecar example config, capabilities, Rust lifecycle, frontend/backend shape
- [x] MoneyPrinterTurbo task/audio/voice/video/state/cache paths
- [x] ffmpeg-ai composer/pipeline/TTS/caption/spec paths
- [x] Automated Video Generator architecture, render, motion, cache, job, safety, tests, and license
- [ ] `edge-tts` repository — named in reconciliation but not locally present
- [ ] `elevenlabs-python` repository — named in reconciliation but not locally present
- [x] `editly` repository — checked out at `dc46674052ea` and reviewed; see section 5.10 and D-060
- [ ] `keyring-rs` repository — selected by reconciliation but not locally present

The unchecked items are a repository-availability gap, not permission to guess their APIs.

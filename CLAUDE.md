# CLAUDE.md — read this first

You are working on **`spoonstill`**: a batch renderer that turns `(still image +
narration)` pairs into one MP4 with Ken Burns motion on each still, cut on
narration boundaries. It is **not a video editor** — no timeline, no scrubber.

## Ground truth, in 30 seconds

- **M0 and M1 are complete; M2 is half done.** There *is* rendering code now:
  the filter graph, the FFmpeg process boundary, the segment profile and its
  assertion, and `still render-scene` end to end. There is now also a project
  model and `still validate`. There is still **no audio resolution, no TTS, no
  state database and no queue**, and no `still render` over a whole project —
  if a document describes those as existing, it is describing an intended
  system. Run `make gates` to see exactly where things stand.
- **Rust 1.94.0 is installed**, pinned by `rust-toolchain.toml`. Homebrew's
  rustup keeps its shims in `/opt/homebrew/opt/rustup/bin`, **not**
  `~/.cargo/bin` — that path is on `PATH` via `~/.zshrc` and is re-exported by
  the `Makefile`, so `make test` works from any shell.
- **This is a git repository**, initialised 2026-08-26. The first commit is the
  planning corpus, deliberately before any code.
- **The name is `spoonstill`; the command is `still`** (D-073). The directory is
  still `vidio/` — cosmetic, and the author's call to rename.
- `plan/` holds **10 read-only reference checkouts** (~1.7 GB) plus three
  retired planning documents. Do not build in there and do not edit them.
  It is gitignored; pinned commits are in `refergit.md` §2.
- FFmpeg 8.0.1 is installed (Homebrew, **GPL build** — dev only, see D-062).
- **The code targets macOS and Windows both** (D-071, decided 2026-08-26).
  Nothing has been *run* on Windows yet, and every number in
  `ffmpeg-findings.md` is macOS arm64. Do not claim otherwise.

## The four documents that matter

Read in this order. Later files never override earlier ones.

| File | What it is | When you need it |
|---|---|---|
| **`decisions.md`** | **Single source of truth.** Numbered decisions (D-001…), each Accepted / Open / Superseded. | Always. Before proposing any design. |
| **`plan.md`** | Milestones M0–M5, each with entry conditions, deliverables, and exit gates that are runnable commands. | Before starting work. To know what "done" means. |
| **`ffmpeg-findings.md`** | Benchmarks measured on this machine on 2026-08-24, with reproduction commands. | Any FFmpeg, filter-graph, memory, or performance question. |
| **`refergit.md`** | Index of the reference checkouts: which repo and which file to open for a given problem. | When you need prior art for a specific implementation problem. |

Precedence:

```
decisions.md > plan.md > refergit.md > ffmpeg-findings.md (evidence, not policy)
             > plan/BRIEF_RECONCILIATION.md, plan/PROJECT_BRIEF.md, plan/REFERENCES.md  (all retired)
```

`ffmpeg-findings.md` sits below policy but **above every claim in the retired
docs and in the reference repos**, because it was measured here.

## How to avoid the four mistakes that keep happening

**1. Do not trust a filter string from a reference repo.**
`ffmpeg-ai`, `Automated-Video-Generator`, and `editly` each contain Ken Burns
code that is wrong, resolution-losing, or solving a problem we do not have.
`ffmpeg-findings.md` §2 and §3 document exactly how. The production recipe is
D-030 through D-034 **plus D-037** — and it is now code, in
`spoonstill_core::motion::build_filter`, with the exact emitted string pinned by
a test. Change it there, and change `decisions.md` in the same commit.

**2. Do not re-litigate a settled decision.**
Prescale, `zoompan` vs `scale`+`crop`, `keyring-rs` vs Stronghold, CLI-first vs
Tauri-first, Edge TTS vs ElevenLabs — all settled, all with recorded reasoning.
Cite the D-number and move on. If you have new evidence, change `decisions.md`
in the same commit as the code.

**3. Do not guess an Open decision.**
Only **D-072** (captions) is still Open, and it records a default to use if you
must proceed — say explicitly that you used it. D-070 and D-071 were Open and
are now Accepted; they live under "Resolved from the Open list" so the answer is
as easy to find as the question was. D-054 through D-057 were added during M2:
read **D-056** before touching the import path and **D-057** before touching
concat or transitions.

**4. Do not invent requirements from the `kenburns-batch` master brief.**
Several documents call it authoritative. It has never been in this workspace
(D-074). Everything real traces to a document that is actually here.

## Rules that hold everywhere

- **Argument vectors, never shell strings.** Every FFmpeg invocation.
- **Audio duration is authoritative**, measured by `ffprobe` on the normalized
  artifact — never estimated from text, never trusted from a container header.
- **`project.yaml` is an input.** The renderer never writes to it. Machine state
  lives in `.spoonstill/state.db`.
- **`setsar=1` is the last filter** before `format=yuv420p`, always (D-033) —
  with `setparams` immediately before it, pinning colour (D-037).
- **Colour range is pinned, not inherited.** A JPEG is full-range and that flag
  reaches the encoder through `format=yuv420p`, producing `yuvj420p`. Measured:
  `ffmpeg-findings.md` §7b.
- **Every failure is written down as it happens** (D-016). `still diagnostics
  export` packages it into one sendable file, with credentials redacted.
- **FFmpeg does not validate concat.** A mismatched segment joins with exit 0
  and no warning — proven in `ffmpeg-findings.md` §5. We assert the segment
  profile ourselves (D-041).
- **n=500 is the design point.** Evaluate every choice there, not at n=5.
- **If the CLI cannot do it, it does not exist.** The CLI is the permanent
  complete control surface; the Tauri shell is a thin later addition that owns
  no business logic.
- **`spoonstill-core` depends on nothing concrete** — no Tauri, no React, no TTS SDK,
  no process code. This is enforced in CI, not by convention.

## Working with `plan/`

Read-only. For each borrowed idea: locate it via `refergit.md` §3, open the real
source file, label it **adopt / modify / reject** with a reason, reimplement it
in Rust, and add a test that proves the target behaviour.

Licence boundaries worth remembering: `lossless-cut` is **GPL-2.0-only** (read,
do not paste), Remotion has a custom commercial licence and is rejected as a
dependency, and the shipped FFmpeg binary needs its own LGPL build (D-062).

## Where to start

**Catch up in one command.** This prints the real state of the build rather than
what any document claims:

```bash
make gates          # M0 8/8 and M1 8/8 = intact.
                    # Also: make gates-m1 | test | lint | fixtures | help
git log --oneline   # planning corpus, then M0, then M1
```

If both milestones are 8/8, everything below is accurate and you can start work
immediately. If not, fix that first — something regressed.

Then see one render for yourself, which is faster than reading about it:

```bash
make fixtures                     # generates the two project fixtures too
cargo build --release -p spoonstill-cli

./target/release/still validate fixtures/projects/mixed/
./target/release/still validate fixtures/projects/manifest/

./target/release/still render-scene \
  --image fixtures/generated/land.jpg \
  --audio fixtures/generated/n.wav \
  --out /tmp/s.mp4
./target/release/still diagnostics export --project /tmp --out /tmp/bundle.txt
```

### State as of 2026-08-26

**M0 and M1 are complete. M2 is half done — slices 1 and 2 of 4.** `make gates`
is 8/8 for M0 and 8/8 for M1. There are no M2 gates in `make gates` yet; M2's
own exit gates are in `plan.md` §M2 and two of the four already pass.

M1 delivered the whole product in miniature: `still render-scene --image X
--audio Y --out seg.mp4` measures the narration, derives an exact frame count,
builds the filter chain, renders through an argument-vector process boundary,
asserts the full segment profile against `ffprobe`, and only then moves the file
into place.

M2 so far turns a folder into a project. `still validate DIR` reads
`project.yaml` and either a CSV manifest or the folder itself (D-050), checks
every rule that needs no disk, resolves every path inside the project root
(D-054), probes every file, and prints **every problem at once** — project-level
first, then scene by scene in render order. Detail and the four-slice table:
`plan.md` §M2.

What exists now, by crate:

- `spoonstill-core` — `motion::build_filter` (pure), `geometry`, `timing`,
  `hash`, `diagnostics`, `path_safety` (containment behind a `RealPath` trait),
  `project` (the scene model and every pure validation rule). Still zero
  dependencies.
- `spoonstill-media` — `command` (the only place a process is spawned),
  `probe` (timed, typed), `profile` (`SegmentProfile` + `assert_matches_profile`),
  `scene` (render → validate → atomic move).
- `spoonstill-state` — `logs`: the JSON Lines sink and the bundle export.
  **Still no SQLite** — that is M3.
- `spoonstill-app` — `import` (`settings`, `rows`, and the resolution stage),
  `render`, `diagnostics`, and `surface`. Owns `serde_yaml_ng` and `csv`;
  the domain model does not know what a file format is.
- `spoonstill-cli` — `still validate`, `still render-scene`,
  `still diagnostics export|where`.

**Next task is M2 slice 3** — the three audio sources. Read `plan.md` §M2
first. In rough order:

1. `AudioSource::resolve()` → `(normalized_path, Duration)`. One path for all
   three sources; the renderer must never branch on which one it was (D-020).
2. Ingest normalization to 48 kHz stereo into the cache. **The operator's
   original is never touched** (D-021), and the duration is measured by
   `ffprobe` on the normalized copy, never on the original's header.
3. `Silent { seconds }` generates a real silent track rather than becoming a
   special case in the renderer.
4. Then `still render DIR` over a whole project, which is what closes M2's
   remaining two exit gates.

Slice 4 is TTS behind a trait (ElevenLabs, BYOK, against a recorded fixture).
`.spoonstill/state.db` and the cache are **M3**, not M2.

**Open decisions:** only D-072 (captions) remains, and it does not block M2.
D-054 through D-057 were added during M2 and are Accepted — read D-056 before
touching the import path and D-057 before touching concat.

**Do not re-derive these** — they cost measurement time and are already settled
in code with tests: the exact filter string, the 90 kHz time base, the H.264
level derivation, `atrim=end_sample=` rather than a rounded decimal, and the
fact that `-color_primaries` as an encoder option does not survive.

### Two traps this project already fell into

- **A decision that is not in `decisions.md` did not happen.** The project name
  was chosen by the author, contradicted by the session that chose it, written
  down nowhere, and lost for two days. See D-073's provenance note. Write
  decisions into `decisions.md` in the same commit as the code.
- **A fixture that encodes a hazard must assert that it still encodes it.**
  `odd.jpg` silently generated as 1998×1000 — even — which would have made the
  D-033 SAR test pass for the wrong reason forever. See `ffmpeg-findings.md`
  §8b. The same shape of bug is available to any test whose fixture is
  generated rather than asserted — and to any *probe*, which is why the
  black-edge check has a control test that proves it reads 0 on a real
  letterbox (`ffmpeg-findings.md` §7c).

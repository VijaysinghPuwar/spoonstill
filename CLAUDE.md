# CLAUDE.md — read this first

You are working on **`spoonstill`**: a batch renderer that turns `(still image +
narration)` pairs into one MP4 with Ken Burns motion on each still, cut on
narration boundaries. It is **not a video editor** — no timeline, no scrubber.

## Ground truth, in 30 seconds

- **M0 and M1 are complete; M2 is three slices of four.** There *is* rendering
  code now, and it renders whole projects: the filter graph, the FFmpeg process
  boundary, the segment profile and its assertion, `still render-scene`, the
  project model and `still validate`, and — as of slice 3 — audio
  normalization, generated silence, a content-addressed cache, a **bounded
  parallel render pool**, the concat join, and `still render DIR`. There is
  still **no TTS and no state database**; if a document describes those as
  existing, it is describing an intended system. Run `make gates` to see
  exactly where things stand.
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
as easy to find as the question was. D-054 through D-057 and D-075 through
D-078 were added during M2: read **D-056** before touching the import path,
**D-057** before touching concat or transitions, and **D-076/D-077** before
touching the render pool.

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
make gates          # M0 8/8, M1 8/8, M2 9/9 = intact.
                    # Also: make gates-m1 | gates-m2 | test | lint | fixtures | help
git log --oneline   # planning corpus, then M0, then M1, then M2 slice by slice
```

If all three are green, everything below is accurate and you can start work
immediately. If not, fix that first — something regressed.

Then see one render for yourself, which is faster than reading about it:

```bash
make fixtures                     # generates the three project fixtures too
cargo build --release -p spoonstill-cli

./target/release/still validate fixtures/projects/mixed/
./target/release/still validate fixtures/projects/manifest/

./target/release/still render-scene \
  --image fixtures/generated/land.jpg \
  --audio fixtures/generated/n.wav \
  --out /tmp/s.mp4

# A whole project, four scenes at a time. Run it twice: the second run
# re-encodes nothing (D-043, D-075) and takes about a seventh of the time.
./target/release/still render fixtures/projects/renderable/ --out /tmp/film.mp4 --jobs 4

./target/release/still diagnostics export --project /tmp --out /tmp/bundle.txt
```

### State as of 2026-08-26

**M0 and M1 are complete. M2 is three slices of four.** `make gates` is 8/8 for
M0, 8/8 for M1 and 9/9 for M2. The M2 gates cover slices 1–3; slice 4 (TTS) is
what closes the milestone, and gate 7 currently asserts that a TTS scene is
*refused by name* rather than rendered.

M1 delivered the whole product in miniature: `still render-scene --image X
--audio Y --out seg.mp4` measures the narration, derives an exact frame count,
builds the filter chain, renders through an argument-vector process boundary,
asserts the full segment profile against `ffprobe`, and only then moves the file
into place.

M2 turns a folder into a project and then into a film. `still validate DIR`
reads `project.yaml` and either a CSV manifest or the folder itself (D-050),
checks every rule that needs no disk, resolves every path inside the project
root (D-054), probes every file, and prints **every problem at once**.
`still render DIR` then resolves every narration, renders every scene
**several at a time**, and joins the validated segments with a stream copy.
Detail and the four-slice table: `plan.md` §M2.

**Parallel rendering, in one paragraph.** `--jobs N` sets how many scenes
encode at once; the default is `available_parallelism() / 2` capped at 4,
because the speedup curve flattens at three while memory keeps climbing at
780 MB per worker (measured: `ffmpeg-findings.md` §10, D-076). `--audio-jobs`
sizes the other pool, which exists separately because ingest is I/O-bound and
becomes a TTS rate limit at slice 4 (D-044). **Concurrency changes the timing
and nothing else** — `--jobs 1` and `--jobs 4` produce byte-identical films,
and that is gate 3, not a comment (D-077). Two renders of one project are
refused by `.spoonstill/render.lock`; two renders of *different* projects share
nothing and run freely.

What exists now, by crate:

- `spoonstill-core` — `motion::build_filter` (pure), `geometry`, `timing`,
  `hash` (one-shot and streaming FNV-1a), `diagnostics`, `path_safety`
  (containment behind a `RealPath` trait), `project` (the scene model and every
  pure validation rule). Still zero dependencies.
- `spoonstill-media` — `command` (the only place a process is spawned),
  `probe` (timed, typed), `profile` (`SegmentProfile` + `assert_matches_profile`),
  `scene` (render → validate → atomic move), `audio` (normalize, generate
  silence, measure), `concat` (the join and the film's own assertion),
  `atomic` (write-beside-then-rename, shared by all of them).
- `spoonstill-state` — `logs`: the JSON Lines sink and the bundle export.
  **Still no SQLite** — that is M3.
- `spoonstill-app` — `import` (`settings`, `rows`, and the resolution stage),
  `audio` (the cache and `AudioSource` resolution), `pool` (the bounded worker
  pool), `film` (`still render`: two pools, the lock, the join), `render`
  (one scene), `diagnostics`, and `surface`. Owns `serde_yaml_ng` and `csv`;
  the domain model does not know what a file format is.
- `spoonstill-cli` — `still validate`, `still render`, `still render-scene`,
  `still diagnostics export|where`.

**Next task is M2 slice 4** — TTS behind a trait, which is what closes M2.
Read `plan.md` §M2 and D-023 first. In rough order:

1. The provider trait in `spoonstill-tts`, with typed settings and errors. One
   `provider` module per implementation; a giant `match` on a provider name is
   the `MoneyPrinterTurbo/app/services/voice.py` mistake.
2. ElevenLabs, BYOK via `keyring-rs` (D-014), against a **recorded fixture**
   from the first commit. The live key belongs in one integration test that is
   skipped by default.
3. Wire it into `spoonstill_app::audio::resolve`, whose `Tts` arm is currently
   one typed `AudioError::TtsNotAvailable`. The cache key is
   `hash(text, provider, voice, settings, profile)` — D-043, and with BYOK a
   miss costs the operator money.
4. Then `still render fixtures/projects/mixed/` renders, and m2-gates gate 7
   changes from "TTS is refused by name" to that render.
5. The secrets check plan.md §M2 asks for becomes real at that point: grep the
   run output, the manifest, the cache keys and the logged command lines for
   the key. Zero hits, and a test that keeps it that way.

`.spoonstill/state.db` and RAM-derived pool sizing (D-044, D-076) are **M3**.
The audio and segment caches are already on disk as content-named directories
(D-075) — M3 gives them an index, not a home.

**Open decisions:** only D-072 (captions) remains, and it does not block M2.
D-054 through D-057 and D-075 through D-078 were added during M2 and are all
Accepted — read D-056 before touching the import path, D-057 before touching
concat or transitions, D-076/D-077 before touching the pool, and D-078 before
changing what the finished film is asserted against.

**Do not re-derive these** — they cost measurement time and are already settled
in code with tests: the exact filter string, the 90 kHz time base, the H.264
level derivation, `atrim=end_sample=` rather than a rounded decimal, the fact
that `-color_primaries` as an encoder option does not survive, the parallel
speedup curve and the 780 MB per worker (§10), and the one AAC frame the
container gains at the join (§10c).

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

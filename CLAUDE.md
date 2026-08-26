# CLAUDE.md — read this first

You are working on **`spoonstill`**: a batch renderer that turns `(still image +
narration)` pairs into one MP4 with Ken Burns motion on each still, cut on
narration boundaries. It is **not a video editor** — no timeline, no scrubber.

## Ground truth, in 30 seconds

- **M0 is complete; M1 has not started.** The workspace skeleton exists —
  six crates, the D-010 boundary test, fixtures, CI. There is **no rendering
  code**: no filter graph, no FFmpeg process boundary, no state, no queue. If a
  document describes those as existing, it is describing an intended system.
  Run `make gates` to see exactly where things stand.
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
D-030 through D-034 — use it, and if you change it, re-run the benchmarks.

**2. Do not re-litigate a settled decision.**
Prescale, `zoompan` vs `scale`+`crop`, `keyring-rs` vs Stronghold, CLI-first vs
Tauri-first, Edge TTS vs ElevenLabs — all settled, all with recorded reasoning.
Cite the D-number and move on. If you have new evidence, change `decisions.md`
in the same commit as the code.

**3. Do not guess an Open decision.**
D-070 through D-074 need the author. Each records a default to use if you must
proceed — say explicitly that you used it.

**4. Do not invent requirements from the `kenburns-batch` master brief.**
Several documents call it authoritative. It has never been in this workspace
(D-074). Everything real traces to a document that is actually here.

## Rules that hold everywhere

- **Argument vectors, never shell strings.** Every FFmpeg invocation.
- **Audio duration is authoritative**, measured by `ffprobe` on the normalized
  artifact — never estimated from text, never trusted from a container header.
- **`project.yaml` is an input.** The renderer never writes to it. Machine state
  lives in `.spoonstill/state.db`.
- **`setsar=1` is the last filter** before `format=yuv420p`, always (D-033).
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
make gates          # 8/8 = M0 intact.  Also: make test | lint | fixtures | help
git log --oneline   # 2 commits: planning corpus, then M0
```

If `make gates` is 8/8, everything below is accurate and you can start work
immediately. If it is not, fix that first — something regressed.

### State as of 2026-08-26

**M0 is complete. M1 has not started.** Do not go looking for rendering code;
there is none. The six crates exist, the D-010 boundary is enforced by
`crates/spoonstill-cli/tests/architecture.rs`, and the four infrastructure
crates are documented stubs with one test each. Details, including what M0
corrected in the docs: `plan.md` §M0.

**Next task is M1**, in this order — riskiest and most testable first:

1. `spoonstill-core::motion::build_filter` — pure, no I/O. Every assertion it
   needs is already measured in `ffmpeg-findings.md` §1–§6, so this is
   test-first work with known answers.
2. `SEGMENT_PROFILE` + `assert_matches_profile`. Write it in M1 even though
   nothing concatenates until M3 — retrofitting a profile means re-rendering
   every segment that already exists.
3. `spoonstill-media` process boundary. Decide D-012 here (depend on
   `ffmpeg-sidecar`, or reimplement ~600 lines) and record the call.
4. `still render-scene` wiring them together — M1's exit gate.

**Open decisions, still unguessed:** D-070 (9:16 + 1:1 in V1 — sets the breadth
of M1's `motion_matrix`, needed before M1 closes) and D-071 (Windows day one —
does not block M1). Both have recorded defaults. Everything else is Accepted.

### Two traps this project already fell into

- **A decision that is not in `decisions.md` did not happen.** The project name
  was chosen by the author, contradicted by the session that chose it, written
  down nowhere, and lost for two days. See D-073's provenance note. Write
  decisions into `decisions.md` in the same commit as the code.
- **A fixture that encodes a hazard must assert that it still encodes it.**
  `odd.jpg` silently generated as 1998×1000 — even — which would have made the
  D-033 SAR test pass for the wrong reason forever. See `ffmpeg-findings.md`
  §8b. The same shape of bug is available to any test whose fixture is
  generated rather than asserted.

# CLAUDE.md — read this first

You are working on **`spoonstill`**: a batch renderer that turns `(still image +
narration)` pairs into one MP4 with Ken Burns motion on each still, cut on
narration boundaries. It is **not a video editor** — no timeline, no scrubber.

## Ground truth, in 30 seconds

- **M0, M1 and M2 are complete. M4's shell exists ahead of M3.** It renders
  whole projects: the filter graph, the FFmpeg process boundary, the segment
  profile and its assertion, the project model and `still validate`, audio
  normalization, generated silence, a content-addressed cache, a **bounded
  parallel render pool**, the concat join, `still render DIR` — and, as of
  slice 4, **speech**: `spoonstill-tts` with the Edge provider, wired through
  the same cache. There is also a **drop-in importer** (`still new` / `still
  add`, D-080) and a **Tauri window** in `apps/desktop`. There is still **no
  state database** (M3) and **no ElevenLabs provider**; if a document describes
  those as existing, it is describing an intended system. Run `make gates`.
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
                    # Also: make gates-m1 | gates-m2 | test | lint | fixtures
                    #       | brand | help
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

# A project the way an operator makes one: a folder, then whatever they have.
# Nothing is renamed by hand and nothing is moved out of the source folder.
./target/release/still new /tmp/demo fixtures/generated/*.jpg fixtures/generated/n.wav
./target/release/still render /tmp/demo --out /tmp/demo.mp4

# Speech. `mixed` has a .txt beside a photo, which is a line to be spoken.
./target/release/still voices en-US            # needs `edge-tts` on PATH
./target/release/still render fixtures/projects/mixed/ --out /tmp/mixed.mp4 \
  --voice en-GB-RyanNeural

./target/release/still diagnostics export --project /tmp --out /tmp/bundle.txt

# The window. Three screens: make or open, fill, review.
cargo run --release -p spoonstill-desktop
```

### State as of 2026-08-26

**M0, M1 and M2 are complete.** `make gates` is 8/8 for M0, 8/8 for M1 and 9/9
for M2 — 25 gates, all green. M2's gate 7 now renders
`fixtures/projects/mixed/`: a photo with a script, a photo with a recording,
and a photo with neither, joined into one film. On a machine with no `edge-tts`
that gate checks the other half of D-020 instead — the render must fail and
name the missing tool, never quietly substitute silence for a written line.

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

**Getting media into a project is the program's job, not the operator's**
(D-080). `still new DIR [FILE...]`, `still add DIR FILE...`, and the window's
drop target all reach `spoonstill_app::ingest`, which copies photos in as
`001`, `002`, … in natural order and pairs each with a recording or a `.txt`
script — **by stem first, then by position**. It never moves an original, never
overwrites, never writes `project.yaml`, and reports junk rather than refusing
a folder. This exists because the pairing convention was costing the operator
twenty to forty minutes of renaming per film, which was most of the time the
tool claimed to save.

**Audio is levelled, and provider padding is trimmed** (D-084). Every
artifact — spoken, supplied or silent — is brought to -16 LUFS by one measured
linear gain, so a TTS line and a phone recording sit at the same level in one
film. Synthesized speech is trimmed to `tts.trim_head` / `tts.trim_tail`
(0.10 s / 0.25 s); **a supplied recording is never trimmed**, because their
padding is a decision and a provider's is an artifact. Normalization costs two
FFmpeg passes per unique source, both behind the content cache.

**Speech, in one paragraph.** `spoonstill-tts` is the provider trait; `edge` is
the only implementation and it **spawns the `edge-tts` command line tool**
through `spoonstill_media::command` rather than reimplementing a
reverse-engineered protocol (D-081). The text goes in a file, never in an
argument, because the command line is logged (D-016) and the script is the
operator's content. The cache key is
`hash(text, provider, voice, settings)` — **no normalization in it** — and the
raw output is kept under that key, so changing the profile, the loudness target
or the trim re-normalizes every line and re-speaks none (D-084). The normalized
artifact has its own key on top of it. Every field is length-prefixed. `still voices [FILTER]` lists what a provider offers; `--voice`
overrides for one run without touching `project.yaml` (D-082).

**Parallel rendering, in one paragraph.** `--jobs N` sets how many scenes
encode at once; the default is `available_parallelism() / 2` capped at 4,
because the speedup curve flattens at three while memory keeps climbing at
780 MB per worker (measured: `ffmpeg-findings.md` §10, D-076). `--audio-jobs`
sizes the other pool, which exists separately because ingest is I/O-bound and
is the TTS provider's rate limit (D-044). **Concurrency changes the timing
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
- `spoonstill-tts` — the `Provider` trait, `Request`/`Voice`/`TtsError`, and
  `edge`. **Sits above `spoonstill-media`** (D-081), because a provider that
  shells out uses the one process boundary rather than growing a second one.
- `spoonstill-app` — `import` (`settings`, `rows`, and the resolution stage),
  `ingest` (making a project and filling it), `audio` (the cache and
  `AudioSource` resolution, speech included), `pool` (the bounded worker pool),
  `film` (`still render`: two pools, the lock, the join), `render` (one scene),
  `diagnostics`, `tts` (the re-export the control surfaces use), and `surface`.
  Owns `serde_yaml_ng` and `csv`; the domain model does not know what a file
  format is.
- `spoonstill-cli` — `still new`, `still add`, `still validate`, `still render`,
  `still render-scene`, `still voices`, `still diagnostics export|where`.
- `apps/desktop` — the Tauri 2 window (D-051's review grid, D-083's shape,
  D-085 and D-086's navigation). **Two levels: home is the operator's projects
  plus app-level Settings; a project is a left rail over one dense grid —
  Scenes, Voice, Output, Render** (D-092 removed Runs). All translation. The
  design brief and canvas it was built against are in
  `~/Downloads/Desktop application redesign` — read D-083 for what was followed
  and the one thing that was not, then D-085 and D-086 for what the author's own
  use of it changed. **Built ahead of M3**, which M4's entry condition says it
  should not have been; it is a shell over M2 and gains M3's resumability for
  free when M3 lands.

**The window has two levels, and the first one is the operator's projects**
(D-086). Home lists every folder ever opened — newest first, path written
`~/Downloads/test`, a moved project struck through with a Forget button rather
than silently dropped — plus **Settings**, which is app-level: whether the voice
service is reachable, which voice it falls back to, and the theme. The list is
Rust's, in the OS config directory, written inside `validate_project` so there
is no way to open a project and have it not appear. *Which projects has this
person opened* is not a fact about any one of them, so it is not under a
`.spoonstill/`. Inside a project there is no Project tab and no Settings tab:
both were screens that restated facts the rail and the title bar already carry.

**Choosing a voice and a destination is the window's job, not `project.yaml`'s**
(D-085). The rail carries the two standing answers — which voice, and what file
— and each is a screen: **Voice** lists the provider's whole catalogue with a
language filter, a gender filter and a search box, and auditions any row on
click through `spoonstill_app::audio::preview`, which is the same cache and the
same normalization a real scene gets (D-084), so it costs nothing the second
time. **Output** is a file name, a folder and a Browse button, with the joined
absolute path shown live; the join and every refusal happen in Rust
(`resolve_output`), because a webview that concatenates paths is a webview that
can be made to concatenate `../..`. Both are overrides for one run — nothing
here writes to `project.yaml` (D-013).

**A locale code is not a language and `default` is not a voice** (D-086). Voice
rows and the language filter read `English (United Kingdom)`, built from the
tag's *parts* via `Intl.DisplayNames` — asked for `en-GB` whole the platform
answers "British English", which files the English voices under A, B and I. The
filter opens on a language the operator can read rather than on whatever sorts
first. And `Provider::default_voice()` resolves `tts.voice: default` to a real
name everywhere it is shown, because "default" does not answer "whose voice will
I hear".

**Next task is M3** — `.spoonstill/state.db`, the resumable queue, and
RAM-derived pool sizing (D-013, D-044, D-076). Read `plan.md` §M3 first. The
audio and segment caches are already on disk as content-named directories
(D-075); M3 gives them an index, not a home.

Two things M2 deliberately left for later, in rough priority order:

1. **ElevenLabs**, BYOK via `keyring-rs` (D-014), against a recorded fixture.
   The trait and the cache are already shaped for it — `providers()` in
   `spoonstill-tts` is one line plus a module. The live key belongs in one
   integration test that is skipped by default, and the secrets check plan.md
   §M2 asks for becomes real then: grep the run output, the manifest, the cache
   keys and the logged command lines for the key. Zero hits, and a test that
   keeps it that way. `edge` already keeps the *script* out of the logs, which
   is the same discipline (D-081).
2. **One long narration split on silence.** The author records one continuous
   voiceover as often as one clip per scene; `ingest` pairs positionally today,
   so a single long recording becomes scene 1 and nothing else. Splitting it on
   silence into as many pieces as there are photos is the biggest remaining
   saving in the whole tool.

**It ships from a tag, and FFmpeg is still the operator's** (D-087). `README.md`
is the front door — the one document written for someone who has never read
`decisions.md`. Pushing `v*` runs `.github/workflows/release.yml`, which builds
the CLI and the window natively for macOS arm64, macOS x86_64 and Windows x64,
attaches a SHA-256 sidecar to every asset, and only undrafts the release once
every leg has uploaded. `scripts/install.sh` and `scripts/install.ps1` are the
one-line installers; they **verify the checksum before they install** and hand
FFmpeg to Homebrew or winget rather than shipping it. Nothing here is M5:
these builds are unsigned, un-notarized, have no updater and bundle no FFmpeg,
and the README says so in as many words. Do not delete an M5 deliverable
because a release exists.

**A path is never trimmed** (D-089). Whitespace at either end of a path is part
of the name — `~/Downloads/RANDOM vidoe ` is a folder Finder makes and macOS
keeps. Trimming it in `resolve_output` greyed out Render on a project with five
valid scenes and nothing wrong with it. A *file name* typed into a box is still
trimmed; that is the one exception and it is written down. The second half of
that decision is the one to remember: **a disabled control explains itself where
it is.** `updateRender()` in `apps/desktop/ui/app.js` is the only thing that
decides whether a render can start, and it writes the reason next to the button.

**The set of legal names is the platform's, not ours** (D-089 and D-090
together). macOS allows a folder whose name ends in a space and our code wrongly
trimmed it away; Windows genuinely forbids that name and `|` besides, so
`create_dir_all` fails with error 123 before any spoonstill code runs. Neither
trim a name down nor assume the legal set is the same on both platforms — the
hostile fixture list in `segment_integrity.rs` is split accordingly, and macOS
coverage is unchanged. This is the first concrete place where D-071's "both
platforms" means "differently".

**Progress is shown in the film's order, and a selection says the word**
(D-091). The render pool finishes scenes in whatever order workers free up, and
the live panel used to log them that way — which read as a scrambled film. It is
now one row per scene in film order, updating in place; the join was always
correct (`pool::run` returns input order, pinned by
`results_come_back_in_input_order`). On the Voice screen a row's highlight had
meant *effective* voice, so the project's default looked selected and clicking
changed nothing visible; rows now say `✓ Selected` or `Project default` outright.
Both follow one rule: **a screen that shows a true thing in a misleading order,
or marks a state without naming it, is a defect of the same kind as a wrong
number.**

**Settings is where you act, and the Runs tab is gone** (D-092). A screen that
showed one project's log inside that project answered a question nobody has —
D-093's CSV answers "what went wrong" without needing to know which folder.
The per-project JSON Lines is untouched and still in the diagnostics bundle. `Provider::install()` is part of the TTS
trait — Edge tries pipx, then brew, then a `--user` pip, and reports success only
when `availability()` says Ready, not when the installer exits zero. The machine
also holds a **fallback voice**; precedence is run override → project's
`tts.voice` → machine fallback → provider's own, and none of them writes to
`project.yaml`. `still voices --install` is the CLI half.

**One CSV of everything, beside the operator's other machine state** (D-093).
Every event goes to the project's JSON Lines *and* to
`<config>/spoonstill/runs.csv`, composed with `spoonstill_state::Tee`. That file
is the one to open when the question is "what went wrong" rather than "what went
wrong in this project" — every field quoted, column order pinned by a test,
rolling at 16 MB. Reachable from **Settings > Activity log** and from
`still diagnostics where`. Failures there are silent by design: a render must
never fail because a spreadsheet could not be written.

**The logo, in one paragraph.** The mark is final as of 2026-08-26 (D-079):
three stacked stills, the front one carrying the image. It is **one ink at
three opacities**, not three greys — each still is an opaque black plate under
a translucent stroke, which is why the two rear strokes brighten where they
cross. It is **defined on black** and is not background-agnostic; the black
plates are load-bearing, so every square use carries its own black tile. Do not
hand-edit any SVG or icon: the geometry lives once in `scripts/gen-brand.py`
and `make brand` emits all of them, stdlib only. The JPEG in
`assets/brand/reference/` is provenance, not an asset. The window palette in
`apps/desktop/ui/styles.css` continues the same ink series and should not
acquire a colour that is not a failure or a warning.

**The window moves, and the strip that moves it is Tauri's, not CSS's**
(D-088). `titleBarStyle: "Overlay"` puts the webview under the system title
bar, so dragging is `data-tauri-drag-region="deep"` on `.titlebar` plus
`core:window:allow-start-dragging` in `capabilities/default.json` — that
permission is **not** in `core:default`. `-webkit-app-region` is Electron's and
WKWebView ignores it without a word, which is how the bar shipped looking
draggable and not being. Anything in the window a test cannot assert has to be
clicked before it is called done.

**Open decisions:** only D-072 (captions) remains, and it does not block
anything. D-087 was added after M2: read it before touching a workflow, an
installer or `README.md`, and D-089 before touching a path or a disabled
control, D-090 before touching a hostile-name fixture, and D-091 before
touching the live panel or the voice list, D-092 before touching Settings or a
provider's tooling, and D-093 before touching a log sink. D-054 through D-057 and D-075 through D-082 were added during M2 and
are all Accepted — read D-056 before touching the import path, D-057 before
touching concat or transitions, D-076/D-077 before touching the pool, D-078
before changing what the finished film is asserted against, D-079 before
touching anything with the logo in it, D-080 before touching `ingest`, and
D-081/D-082 before touching a provider, D-083, D-085, D-086 and D-088 before
touching the window, and D-084 before touching anything in the audio path.

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

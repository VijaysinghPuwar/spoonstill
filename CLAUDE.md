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
  those as existing, it is describing an intended system. Run `make gates` — 37 checks, M2 now 21.
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
- **FFmpeg 9.0.1** is installed (Homebrew, **GPL build** — dev only, see D-062).
  This said 8.0.1 until 2026-09-04: the major version moved underneath the
  project and nothing noticed (D-151). `ffmpeg-findings.md`'s 8.0.1 is correct
  **as provenance** — those numbers were measured on that build — and must not
  be changed. `still doctor` prints the version now, and every render records
  it.
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
Only **D-072** (captions) is still Open, and **its recorded default is now
superseded by D-106**: captions are burned into the picture, not a sidecar
`.srt`. What is still open there is word-level karaoke, which needs provider
word boundaries. D-070 and D-071 were Open and
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
make gates          # M0 8/8, M1 8/8, M2 21/21 = intact.
                    # Also: make gates-m1 | gates-m2 | test | lint | fixtures
                    #       | brand | tts-live | help
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

# Subtitles, burned into the picture (D-106). Six looks; `still subtitles`
# says what each is for. A scene is captioned when it has words — the script it
# speaks, or a .txt beside its recording.
./target/release/still subtitles
./target/release/still render /tmp/demo --out /tmp/demo-subs.mp4 --subtitles boxed

# Size and shape (D-143). The number is the SHORT edge, so 4K vertical is
# 2160x3840 — `shorts`, `reel`, `tiktok` and `story` all mean 9:16.
./target/release/still resolutions
./target/release/still render /tmp/demo --out /tmp/demo-4k.mp4 --resolution 4k
./target/release/still render /tmp/demo --out /tmp/short.mp4 \
  --aspect shorts --resolution 1080p

# Every external program this needs, and the offer to install what is missing.
# The first thing to run against a report that says the app "won't open".
./target/release/still doctor
./target/release/still doctor --install

./target/release/still diagnostics export --project /tmp --out /tmp/bundle.txt

# The window. Three screens: make or open, fill, review.
cargo run --release -p spoonstill-desktop
```

### State as of 2026-09-05 — the first session run on Windows: the GPU question, and a window with no name

**Everything below was executed on Windows 11, 16 cores, 15.2 GB, an RTX 3060
and an AMD Radeon.** Until today every number in `ffmpeg-findings.md` was macOS
arm64 and the top of this file said so. `ffmpeg-findings.md` **§13** is the
first section that is not.

**D-159 — the graphics card is detected by using it, and it is still not what
renders.** The question was *does it detect the GPU, and if not fix it*, and it
did not: D-036 settled the encoder in M1 in one sentence with three clauses —
*"probe availability at runtime, expose it as an explicit fast draft mode, and
always fall back to libx264"* — and only the third was ever built, by never
leaving it. `spoonstill_media::hardware` is the first clause, and it **encodes a
frame** rather than reading `ffmpeg -encoders`, because on this machine that
listing names **seven** H.264 encoders of which **four** work: `h264_qsv` is
listed on a machine with no Intel graphics, and `h264_vaapi` — a Linux API — is
listed on Windows. A detector that grepped the list would report seven, which is
worse than reporting nothing. The probe feeds **software** frames on purpose:
D-030 through D-037 all run on the CPU, so an encoder that needs the frame
already on the GPU is not a drop-in for libx264 here, and `h264_vaapi`,
`h264_d3d12va` and `h264_vulkan` are correctly reported unusable for that
reason. Surfaced on `still doctor` (1.7 s) and in the diagnostics bundle;
**nowhere on the render path**, which is D-151's split. Nothing there can fail
`doctor` — a missing `h264_qsv` on an AMD machine is the correct pairing.

**Running it found two defects in it, neither of which a review would have.**
The first draft reported FFmpeg's *last* stderr line, which for `h264_qsv` is
the muxer saying *"Nothing was written into output file"* — a consequence three
components downstream that would have sent the operator to look at their output
settings. The cause is the **first** line naming the encoder, because FFmpeg
reports causes before cascades. And a test caught the parser reading the
`-encoders` legend line ` V..... = Video` as an encoder named `=`.

**The GPU is worth 1.23x, measured rather than inferred.** D-144 put any encoder
change at a 1.17x ceiling using `ultrafast` as a stand-in, because that machine
has no NVIDIA hardware. This one does. 4K, 112 frames, the shipped chain: filter
alone **4.53 s**, shipped `libx264 -preset medium` **5.86 s**, `h264_nvenc`
**4.78 s**, `h264_amf` **5.00 s**. So the encoder is **22.7% of a 4K render
here against macOS's 14%**, NVENC is worth **1.23x**, and it saves **zero**
memory — the memory is the 11520x6480 prescale canvas on the CPU. D-036's
default stands, and `still doctor` says so in two lines under the list every
time, because a list of usable encoders invites exactly one wrong conclusion.
**The fast draft mode is still not built**: it trades quality for 1.23x and
would touch the segment cache key (D-107) and the D-041 profile assertion, which
are two of the load-bearing invariants here. That is a decision to make from the
numbers above, not a thing to slip in.

**D-144 was verified on the platform it was reported from, and it holds.** The
4K render that froze a machine now picks **3 workers rather than 4** and says
why. Peak was **7 163 MB — 2 388 MB per worker**, against the model's 2 755 MB:
the model sits **15% above** the Windows measurement and 5% above the macOS one,
which is the direction `capacity.rs` is built on. No freeze.

**D-076's cap of 4 was derived on macOS and is correct on Windows too.** 32
scenes at 1080p: jobs 1 **47.2 s**, 2 **27.4**, 3 **22.2**, 4 **19.9**, 6
**18.6**, 8 **18.2**, 12 **19.8**. Same shape as §10's macOS curve on a machine
with 60% more cores — it flattens after four and **regresses at twelve**. Going
4 -> 8 buys 9% for twice the memory, which is exactly the trade D-076 declined.
Worth checking because a 16-core machine is where one would expect it to be
wrong; it was not.

**And orchestration is not where Windows time goes** — the thing to rule out on
a platform with expensive process creation and a virus scanner over every
written file. `still validate` on 32 scenes is **0.25 s**, and a fully cached
re-render — 32 probes, the join and the D-041 assertion, no encoding at all — is
**1.16 s**, or **36 ms a scene**. There is no Windows-specific overhead to
remove; the time is in the filter graph, where this file has always said it is.
**So "optimise for Windows" has a measured answer and it is that both levers are
already set correctly** — which is a result, not a shrug, and the numbers above
are what would overturn it.

**D-160 — the window had no name on Windows, and the config could not give it
one.** `apps/desktop` builds on Windows for the first time (D-132 excluded it
because `tauri-winres` wanted `llvm-rc` on the Mac): 2m08, 18 tests green, and it
opens and renders its real content. What it also had was a **blank native title
bar** and a nameless taskbar and Alt-Tab entry. `tauri.conf.json` sets
`title: ""` because macOS draws its own header under `titleBarStyle: Overlay`,
but D-132 already established `title_bar_style` is macOS-only — so Windows keeps
a real title bar and an empty title leaves it empty. **The obvious fix is wrong**:
`TitleBarStyle::Overlay` maps to `titlebar_transparent` + `fullsize_content_view`
and **not** to `title_hidden` (read in the pinned `tauri-runtime-wry`, not
assumed), so a title in the config would be drawn over the page's own header on
macOS. It is set at runtime under `#[cfg(target_os = "windows")]`, which is
D-132's own pattern and leaves the other machine provably untouched.

**What could not be established, stated rather than implied.** Launched
repeatedly from an automated non-interactive shell, the window exits cleanly
(code 0, no stderr, no event-log entry) about two seconds in, roughly two runs in
three. That looked like a regression from the morning's commits and **it is
not**: at n=5 the installed **v0.1.7** does the same thing four times in five,
and the two trials that suggested otherwise were n=1 each. Almost certainly the
launch context rather than the product, and D-131 already records that this
project has no GUI automation. Proven: it builds, passes its tests, opens,
renders, and now has a name. Not proven: unattended lifetime. **Do not bisect
`master` for those two seconds** — that is most of what this session cost.

**D-142's FFmpeg search is confirmed on a real winget install.** FFmpeg is
**not on PATH** on this machine: `Gyan.FFmpeg` publishes no shim and lands under
`WinGet/Packages/Gyan.FFmpeg_…/ffmpeg-9.0.1-full_build/bin`. `still doctor`
finds it and reports `9.0.1-full_build-www.gyan.dev`. Note the **major version**:
9.0.1 here against the 8.0.1 every earlier measurement used, which is D-151's
point about nothing having recorded it.

### State as of 2026-09-05 — Hindi came out as a row of boxes reading NO GLYPH

**D-157 — a caption is drawn by a face that has the glyphs, and shaped.**
Reported with a screenshot: a Hindi caption rendered as small black-and-yellow
boxes, each reading **NO GLYPH**. That is not a rendering fault — it is
**Inter's `.notdef` glyph**, drawn once per character, because Inter covers
Latin, Greek and Cyrillic and has no Devanagari at all.

**Coverage was the smaller half.** A font with the glyphs still draws Hindi
wrong laid out one character at a time: `स` + `्` + `त` is one conjunct, and `ि`
is drawn to the **left** of the consonant it follows. Three weights of **Noto
Sans Devanagari** (OFL, 742 KB, licence in `THIRD-PARTY-NOTICES.md`) plus
**`harfrust`** for shaping; a run is a maximal span of one face, a space always
belongs to the primary face so a Hindi sentence is one run per word, and shaping
is cached **per word in font units** because `balance` bisects.

**Latin is deliberately not shaped, and that is the load-bearing decision.** One
path would be tidier and would re-kern every caption ever burned in (fontdue
reads `kern`, HarfBuzz reads GPOS). Measured, not assumed: three Latin cues
rendered by the build before and the build after are **byte identical**, and two
of those hashes are pinned in a test that says what breaking it costs.

**The band grew where it had to** — `line_metrics` takes the max over the faces
*these lines use*, so Devanagari's high marks are not clipped (D-117's defect in
another term) and a Latin caption keeps Inter's numbers exactly.

**What is still not drawn is said out loud.** Bengali, Tamil, Arabic, Chinese
and emoji still box; bundling a Noto face per script is tens of megabytes.
Stopping there is defensible, being silent is not — `caption::undrawable` names
the characters and `ProblemKind::UndrawableCaption` carries them to `still
validate`, the window and the render, one line for the project, re-decided for
the run's own `--subtitles` answer.

**D-129's audit gate did its job on its first new dependency since.**
`rustybuzz` was written first and `cargo audit --deny warnings` refused it —
unmaintained as of 2026-07-11 (RUSTSEC-2026-0206), whose own advisory names
`harfrust` as the maintained replacement. Adding the crate and then an ignore
entry for it is the wrong way round. The port was one commit; every test passed
unchanged, byte-identity included.

**D-158 — the script chooses the voice, when nobody else has.** Found by
rendering the caption D-157 had just fixed: **a Hindi project could not render
at all.** `default` means `en-US-AvaNeural`, the service accepts the request and
returns **no audio**, and the message blamed *the operator's text* — *"a line
with no speakable words in it"* — over a perfectly ordinary sentence.

`spoonstill_core::language::of` reads a BCP-47 subtag off the characters (the
dominant non-Latin script, so one English word in a Hindi line changes nothing)
and the Edge provider uses it **only when the voice is `default`** — a voice the
operator named is obeyed whatever the line says. **Latin returns `None`**, so
every project already made keeps its voice, its cache key and its audio.

A script is not a language, and this errs towards the most-written one, because
refusing to guess here means the render *fails*. The one ambiguity worth
resolving is: **Han with kana is Japanese, Han alone is Chinese.** The 21 voices
are **named, not searched** (a catalogue Microsoft edits must not move a cache
key); **Punjabi, Odia and Armenian are detected and deliberately absent**,
because Edge has no voice for any of them and `Invalid voice` is not a better
failure than no audio. A `make tts-live` test asserts the table has not rotted.

And `wrong_script` names the real cause when a chosen voice cannot read the
line, with the voice to use instead. The line is quoted **plainly** in both
places — `{:?}` had been printing the operator's Hindi as `नमस\u{94d}त\u{947}`
(D-150).

**Measured end to end:** the project that failed now renders with
`voice=hi-IN-MadhurNeural` in `runs.csv`, captions correct at 1080p.
**`make gates` is still 37** — a real Hindi render needs the voice service, and
D-134 already ruled a gate must not depend on that.

### State as of 2026-09-05 — a new project was not a project until you opened it

**D-156 — making a project opens it.** Reported from a fresh install: *New
project → choose a folder → drag the photographs in → nothing happens*, the
window stuck on the "Choose photos…" screen; doing it several times eventually
worked, which made it read as flaky. It is not flaky. **`create_project`
returned a path and told `Session` nothing**, and every command that writes is
bound to the project `Session` says is open (D-127) — so the drop that follows
reached `add_media` and was refused with *"no project is open in this window"*,
in the status line, on the one screen whose whole job is to receive that drop.
The recovery was to open the folder you had just made, which runs
`validate_project` and adopts it.

Two halves. `create_project` adopts the folder it made — **after** it exists and
never on the way to a failure, so a refused folder leaves the window open on
what it had. And `newProject` hands the returned path to **`load`**, the way
`openProject` always has: **a project is opened in one place.** The page used to
assemble a `ProjectView` of its own, which also carried none of the fields the
grid reads, never reached `remember` (so a new project was missing from Home —
the hole in D-086's rule), and took the name by splitting on `/`.

Checked rather than assumed: a brand-new folder holds only D-153's starter
`project.yaml`, so `import::load` returns **one** `NoScenes` problem, which is
exactly what `ProjectView::empty` is defined by — the same fill screen as
before, now with Rust knowing about it. An `Err` there would have bounced the
page to the start screen, worse than the defect.

**Both tests were run against the unfixed code.** `making_a_project_opens_it`
drives the reported sequence end to end — create, then drop a photograph
through `add_media_inner` — and fails with the operator's own message;
`ui_contract.rs`'s half fails when the page assembles its own view. That second
test's first version **passed against the broken page**, because its
600-character window reached the definition of `load` a few lines below the call
it was looking for — D-116's trap, inside a test written for D-116's shape of
defect. **`make gates` is still 37.**

### State as of 2026-09-04 — `master` was red, and only the Windows runner could say so

**D-155 — a test's stand-in child is a program, and a program is not a portable
name.** The CI leg that runs on the other platform D-071 puts in scope failed
on two tests written the session before, in D-149's poll-interval work: they
spawned **`/bin/echo`** and **`/bin/sleep`**, and Windows has neither — no
`echo` or `sleep` binary exists there at all, both are `cmd` builtins. 104
passed, 2 failed, and nothing about `wait_until` itself is wrong on Windows.

**The obvious repair is a shell, which is the one thing `no_shell_strings.rs`
refuses** (D-011). `ping -n 31` and `Start-Sleep` each swap one unstated
assumption about the machine for another, and `timeout.exe` refuses to run when
stdin is redirected — which is how every child here is spawned.

**The stand-in is the test binary itself**: `current_exe()` re-run with
`--exact` against one of two `#[ignore]`d helpers in the same module, one that
prints a marker and one that sleeps. No shell, no installed tool, arguments
still a vector. **A filter that matches nothing runs no tests and exits 0** —
measured — so the fast half asserts the *marker*, not the exit status, and a
renamed helper fails loudly instead of passing.

**Where the platform rule reaches is the lesson.** D-128 fixed the product's
quoting and D-132's cross-compile catches Windows lints and type errors;
neither can see a path that only exists at run time inside `#[cfg(test)]`.
**Wait for the Windows leg before calling a session finished** — D-132 says it
for a tag, and it is the same sentence for `master`. **`make gates` is still
37.**

### State as of 2026-09-04 — M3's goal is already met, and not by a database

**D-154 — before writing a schema, M3's own exit gates were run.** A 500-scene
1080p fixture was built for it. The milestone's *goal* is met; its *deliverable
list* is not, and those are different things.

| gate | measured |
|---|---|
| 1 clean run | **161 / 151 / 154 s**, peak RSS **2873 MB**, 30 000 frames, 1000.021 s against 1000.000 |
| 2 kill and resume | killed at 60 s with **167 of 500** done → resume **reused exactly 167**, film identical to a clean run, 4 `.partial-` files swept to 0 |
| 3 determinism | **three** cold runs, all byte-identical |
| 4 single-scene invalidation | one narration edited → **499 of 500 reused** |
| 5, 6 | exist as tests under other names (`profile.rs` per-field; `cancellation_leaves_no_valid_looking_stub`) |

**Resume works because the cache is content-addressed, not because anything
remembers.** A killed run leaves finished segments that the next run finds by
asking the same question again — nothing to reconcile, nothing to migrate, and
`.spoonstill/` stays deletable at any point *because it holds no authority*.

**The first attempt at gate 2 was vacuous and that is the lesson.** It ran with
a warm cache, so the render finished in 14 s and the `kill -9` at 45 s hit a
process that had already exited — then reported a clean resume. It looked like a
pass. **A resume gate must start cold, or it measures nothing.** Gate 1's first
number was wrong more dully: an RSS sampler and a cold page cache made one run
506 s against a true ~155 s. Both were thrown away and re-measured.

**What M3 still owes:** `state.db` **as an index for reporting** — which is what
this file has always said (*"an index, not a home"*) and explicitly **not**
load-bearing for correctness; transitions (D-057), still `cut` until `xfade` is
measured, which the fixture now makes cheap; and an integration test for gate 5,
the one gate that should not stay a unit test because FFmpeg returns exit 0 on a
bad join. **D-144 already delivered M3's RAM-derived pool sizing.**

**Delivered with it:** plan.md §M3's *"a test asserts `project.yaml`'s mtime and
hash are unchanged after a full render"* — promised in `settings.rs`'s module
note since M2 and never written. Gate 7g now, and it matters more since D-153
made `create_project` write a starter file: the rule became "written exactly
once, at creation, and never again". **M2 is 21 gates; `make gates` is 37.**

### State as of 2026-09-04 — a reorder is free now, and no film already made moves

**D-153 — the motion seed is versioned.** `findings.md` F-21, which is D-140
reopened on the terms D-140 itself set. Reproduced first: eight scenes,
`still move 008 1`, render again → `8 narrations from cache, **0 segments
reused**`. Every scene re-encoded and every scene's motion changed, for one row
moved. About four minutes per drag at chapter size; eighteen at 500 scenes.

**The author chose this option** over opting in by hand, changing it for
everybody, or leaving it.

- **`MotionSeed::V1` is frozen.** `MotionSpec::seeded` is untouched and a test
  pins the six descriptors D-140 recorded. **Absent means V1**, and absent is
  every `project.yaml` written before the key existed.
- **`V2` is `project id + occurrence + content hash`** — no scene index.
  `occurrence` is how many earlier scenes use the same still, carrying the one
  thing the index was genuinely for (D-035: one photograph shown twice must not
  move identically) without depending on position. Counted by **path**, because
  the content hash is computed inside the pool (D-146) and re-reading every
  photograph here would put back the serial pass D-149 removed. The seed is
  tagged, or V2's first occurrence would hash identically to V1 at index 0.
- **The index was in the segment filename too.** With the seed fixed the
  measurement was *still* `0 segments reused`: `seg-{index:04}-{key}.mp4` names
  a file after where a scene sits. It is `seg-{key:016x}.mp4` now. Checked that
  `scene_index` reaches nothing in the encoded bytes first — three log fields
  and one `seeded` call that only `still render-scene` reaches.
- **`still new` writes a starter `project.yaml`** — the one exception to
  "ingest writes no settings file", and narrow: nothing touches a file that
  exists, `add_media` still writes none, and a folder that says nothing renders
  under V1 **forever, including folders made tomorrow**.

**Measured, and it is the claim everything rests on:** a project with no
`motion_seed:` key rendered by the build before all of this and the build after
it — in folders with the **same basename**, because `project_id` is the
basename and it seeds the move (gate 7e already fell into that once) — is
**byte-identical**. With `motion_seed: v2` it differs, as it must. A v2 reorder
reuses **6 of 6**; a v1 reorder still reuses 0, deliberately.

**One migration cost, one-time and invisible:** a project already rendered holds
old-style segment names, so its next render re-encodes once and produces a
byte-identical film. The old names are matched by `is_our_segment`, so D-109
bounds them rather than stranding them. Same trade as D-107 and D-118.

**M2 is 20 gates; `make gates` is 36.** Gate 4g's v1 half asserts `0 segments
reused` — asserting that a documented cost *persists*, because that is the
promise being kept.

**Running it found a gate that had been passing by doing nothing.** D-147's
insertion split a two-line `check "…" \` call and orphaned
`gate_reuse_checks_length` on the line below, so **gate 4e reported PASS while
running no command** for three sessions — and the orphaned function ran at the
top level, its `ls:` errors landing outside the harness's capture where they
read as noise. `check` now fails loudly on a call with no command, verified by
orphaning it again. **A gate count is not evidence that every gate ran.**

Once running again it failed for a *second*, real reason: it globbed
`seg-0000-*` to find its two segments, and the index had just left the name. It
asks each segment its frame count now — 60 against 240 — which needs no
filename and survives the next rename.

### State as of 2026-09-04 — a README was being read aloud

**D-152 — `findings.md` F-22.** `TEXT_EXTENSIONS` holds `md`, and D-080's
importer pairs **by stem first, then by position** — so `still new proj
photos/*` over a folder holding a `README.md`, which is most folders, paired it
with a photograph and **spoke it**. Billable under D-014, and easy to miss in a
list of thirty rows.

**The harm is the guess, not the extension.** A `.md` whose stem names a still
is a statement of intent, and projects made by earlier builds contain exactly
that. `POSITIONAL_TEXT_EXTENSIONS` is `["txt"]` and `assign` takes the predicate
deciding what its second pass may consider: `.md` still pairs by stem, and is
never dropped into a still that did not name it.

**Removing `md` from the list was the obvious fix and is worse** — a `.md`
already in a project would go silent with no explanation, and the constant is
shared with the folder scan, so one edit would change two surfaces invisibly.
Both halves are tested. The refused file is reported as `Skipped` with what to
do about it; silence there is the same defect one layer along.

Worth knowing, because it is why nothing is lost: **markdown markup is spoken
literally** — `# Chapter One` reaches the provider as those characters. `.md`
was never a good narration format. **`make gates` is still 35.**

### State as of 2026-09-04 — the log and the docs stop lying

**D-151 — `findings.md` F-16, F-17, F-19 and F-20.** Four places where what
this program wrote down was not what it did.

- **F-16** 200 rows of `runs.csv` say `voice=default` while the argv on the same
  row says `--voice en-US-AvaNeural`. D-086 already ruled that **`default` is
  not a voice** and fixed it for every displayed surface; the log never got the
  same treatment. `Spoken` carries the voice that actually spoke now — from the
  **provider**, which is the thing that resolves it, rather than the caller
  working the rule out a second time (D-111's lesson).
- **F-17** "film complete" carried `duration_s`, `scenes` and `frames` and not
  the one number that says *why a run took the time it did*. A 112-scene
  re-render finished in 9.9 s and the log could not say whether the cache had
  worked. It records `reused_segments`, `reused_audio`, `spoken` and
  `freed_bytes` now, so a performance question is answerable from the file
  instead of by repeating the render.
- **F-19** `CLAUDE.md` said FFmpeg **8.0.1**. It is **9.0.1** — the major
  version moved underneath this project between two sessions and nothing
  noticed, because D-041 asserts a strict profile against whatever it finds and
  so kept passing. The line is corrected, `still doctor` prints the version, and
  **every render records `ffmpeg=` and `ffmpeg_path=`** — one `-version` spawn
  per run, not per file, and one implementation (`tools::version_line`) shared
  with the diagnostics bundle so the two cannot disagree about which build made
  a film. **`ffmpeg-findings.md`'s 8.0.1 is correct as provenance and must not
  be changed.**
- **F-20** the handoff record stopped at D-141 and the test count was stale.
  D-142 is in the read-before-touching list now, and the counts move with the
  work.

**`make gates` is still 35.**

### State as of 2026-09-04 — seven wrong messages, one of them in `still --help`

**D-150 — `findings.md` F-01 through F-07.** None of them produces a wrong
film; all seven are a message an operator cannot act on, which D-091 already
called the same class of defect as a wrong number.

- **F-01** `still new` reported every `.txt` as a line to speak. Since D-106 a
  `.txt` beside a recording is that scene's **caption**, so one folder answered
  two ways — *"3 scripts to speak"* against `still validate`'s *"2 narrated"* —
  and under D-014's BYOK it **overstated the bill**. `lines_to_speak` and
  `captions` split them by the renderer's own rule, and the printed row shows
  the caption instead of dropping it.
- **F-02** `MediaError::Exit` said *"ffmpeg exited"* over an `ffprobe` argv on
  the next line. It carries the program now, taken from the path launched.
- **F-03** one problem list, two standards: a broken photo got a sentence, a
  broken recording got `ffprobe exited 1` and a `[mp3 @ 0x…]` line. Not
  deliberate — a truncated JPEG still *probes*, so it reached D-052's mapping,
  while a broken MP3 makes `ffprobe` exit and never got there. A failed probe
  now leads with a sentence; the evidence follows it, indented (D-105: a
  terminal loses nothing).
- **F-04** integer megabytes truncated, so every size between D-126's 256 KiB
  ceiling and 1 MB said **`0 MB`** while being refused for being too big.
  `human_size` reports bytes, KB, then one decimal of MB.
- **F-05** six sites cited **D-099** (Gatekeeper quarantine) for the arrange
  work, which is **D-100** — and two were doc comments, so the wrong number
  **printed in `still --help`**. `cited_decisions.rs` now fails on a `D-nnn`
  that is not a heading in `decisions.md`; it is honest that this cannot catch
  D-099-for-D-100, and what it does enforce is that **a decision cited in code
  is written in the same commit** — it failed on this change until D-150
  existed.
- **F-06** `still diagnostics where` printed the project log's status *after*
  the `runs.csv` line, so `(not created yet)` read as belonging to a file whose
  size had just been reported.
- **F-07** a move renumbers, so the list it printed read `001  001.jpg`, … —
  it could not confirm the one thing it existed to confirm. `move_to` returns
  `Moved` now: `003 → 001   moved from 3 to 1 of 3`, and the files.

**Deliberately not done:** the window does not fold F-03's technical half behind
a disclosure. `NotUsableMedia` carries one string and splitting it is a model
change plus a component; the sentence is first in both surfaces, which is the
half that was wrong. **`make gates` is still 35.**

### State as of 2026-09-04 — `still validate` was eleven seconds of waiting

**D-149 — a poll interval is a floor under every child, and one probe at a time
is a floor under `still validate`.** `findings.md` F-08, F-09 and F-10 as one
number: **200 scenes took 11.82 s** on a ten-core machine and almost none of it
was work.

| 200 scenes, best of three | one at a time | eight at a time |
|---|---|---|
| 20 ms fixed poll (before) | **11.82 s** | 1.65 s |
| doubling 1→20 ms | — | 1.62 s |
| proportional (now) | 7.64 s | **1.31 s** |

**9.0x.**

**The poll.** `POLL_INTERVAL` was a flat 20 ms and its own comment stated the
premise it failed on — *"irrelevant against a multi-second encode"*, true of an
encode, and `still validate` spawns two `ffprobe` calls per scene and touches no
encoder. It is now `clamp(waited / 8, 1 ms, 20 ms)`: **a look is never further
from the last than an eighth of how long this child has already run**. A plain
doubling was tried first and is measurably not enough — 1, 2, 4, 8, 16 hits the
ceiling after 31 ms of cumulative wait, exactly where a probe lives, and costs
19%. `waited` is time **slept**, not the clock, so the schedule is asserted
exactly rather than timed.

**It is a `still validate` fix, not a render fix**, and the numbers say so: 60
scenes cold, 7.57 s → 7.50 s, about 1%. Recorded so nobody re-derives it hoping
for more.

**The probes.** `import::load` resolved rows one at a time; they now run
`probe_jobs()` at a time (twice the render pool's default — a probe waits and
holds no canvas, so D-144's memory rule does not reach it). `MediaCheck` gained
a `Sync` bound on the **trait**, so every implementation promises it.

**The blocker F-09 named is not the real one.** It warned the *problem list*
must stay deterministic — but `order_by_scene` re-sorts it afterwards, so a
shuffled merge would still print correctly and a test of that would pass against
the broken code (**D-116's trap**). What the merge really carries is `files`,
indexed by row: get it out of order and scene 3 renders scene 7's photograph
silently. That is what is asserted, and it fails when the merge is reversed.
The concurrency itself is asserted **without a clock** — a stand-in that will
not answer until a second check arrives — and fails after its ten-second
deadline when the pool is forced to one worker.

**F-10 was already fixed by D-146**, which moved `plan_scene` and the still's
hash inside the segment pool.

**No gate, deliberately**: what changed is how long something takes, and a
wall-clock assertion on a shared runner fails for reasons that are not defects
(D-130's rule). The schedule, the ordering and the concurrency are unit-tested.
**`make gates` is still 35.**

### State as of 2026-09-04 — the log that answers "what went wrong" was written by one command in fourteen

**D-148 — `runs.csv` recorded renders and nothing else.** `findings.md` F-15.
D-093 built that file to be *the one to open when the question is "what went
wrong"*, and then constructed it **inside `render_project`** — so `still
validate`, `new`, `add`, `voices`, `doctor`, `remove`, `move` and the **entire
window** wrote to it not at all (`apps/desktop/src/main.rs` held no `record`
call). The file covers 2026-08-31 with **1508 rows** and the failure the author
reported that day — the wall of Python on the Voice screen, D-141 — **is not in
it**.

**Why is worth more than the fix.** The sinks are normally two `Option`s, and
D-093's `Tee` composes two sinks that both *exist*: expressing the real case
took a `zip`, a `map` and a four-arm match — eleven lines. So there was exactly
one call site. **A sink that is awkward to build is a sink that gets built
once.** `Journal` owns the pair and is itself a `Diagnostics`; `Tee` is
removed. Three existing sites got simpler for free, and `still render-scene`
and the window's voice audition now reach both sinks instead of only the
project's.

**Asking about a folder is not adopting it.** `Journal::for_surface` opens the
machine index always and the project's own log **only where `.spoonstill/`
already exists**, because `FileLog::open` *creates* it — `still validate
~/Pictures` must not leave a state directory behind.

**One wrapper, above `dispatch`**, covers all fourteen CLI commands, so a
fifteenth inherits it. The scope is a `&'static str` match and not anything
derived from `clap`: a scope that moved when a help string was reworded would
make an old `runs.csv` unfilterable. **Two rows in the terminal, one in the
window** — the `invoked` row is what says a command started and never came
back, which is how D-144 was diagnosed; a window command that hangs leaves an
operator describing a spinner instead.

**The window records now.** Sixteen commands hand their outcome to
`journalled`; four are exempt **by name with a reason** (`resolve_output`,
`subtitle_preview`, `activity_log`, `cancel_render` — pure and local).
`ui_contract.rs` fails on any `#[tauri::command]` returning `Result` that
neither journals nor is on that list; verified by removing it from `voices`.

**Measured with `HOME` redirected**: every command produces its rows, a failed
`validate` carries `detail=… is not a project folder`, the refused folder is not
created, and a rendered project carries the same rows in its own JSON Lines.
**Gate 7f** asserts all of it plus that a render still writes its detailed
events to both sinks — D-093 must not be traded for D-148. Against the old code
it fails, 18/19. **M2 is 19 gates; `make gates` is 35.**

### State as of 2026-09-03 — 861 MB protecting 26.5 MB

**D-147 — the cache keeps what cost money and sweeps what cost a second.**
`findings.md` F-14: the author's folders held **861 MB of normalized WAV around
26.5 MB of spoken MP3 — 32:1**, on a network volume, because D-109 swept
segments and spared the audio cache entirely.

D-109's sentence — *"a segment is CPU, a narration is a network call and under
D-014 money"* — is **right about the spoken layer and does not describe the
derived one**. D-084 already keys them as two layers precisely so that changing
the loudness target re-normalizes every line and re-speaks none; the normalized
WAV is one local FFmpeg pass from an MP3 that is still sitting beside it.

`AudioCache::is_derived_name` matches exactly what `path_for` writes and refuses
`spoken-` explicitly as well as by extension. Same live-set-plus-two-generations
bound, same refusal to delete a stranger's file, same `--keep-cache`, same
only-after-the-join placement.

**One rule, one implementation.** `prune_segments` and `prune_audio` are two
lines each over a shared `prune` that takes the ownership predicate as its only
parameter — the two caches differed in exactly that, and everything else would
have drifted as two copies. The spare budget now counts **distinct** live files,
since many scenes resolving to one narration is the ordinary case (D-108).

**Measured**, eight scenes under six voices: derived **70.5 MB / 48 files →
35.2 MB / 24 files**; spoken **2.25 MB / 48 files, unchanged**. The 31:1 ratio
between the layers reproduces the author's 32:1 from an independent direction.
The bound is three generations however many renders happen.

**Flipping back is still free** — re-rendering under a voice from two
generations ago reports `4 narrations from cache, 4 segments reused`. Keeping
only the live set would also be bounded and would make every flip a
re-normalize.

**A voice audition is derived too, and that is correct**: `preview` writes into
the same cache, so a render may sweep an audition's normalized half and can
never sweep its spoken half — auditioning that voice again is a local pass, not
a call.

**Gate 4f** asserts all four properties and uses D-146's voice-service
stand-in, now the shared `stub_voice_service` helper. With the audio sweep
removed it fails, 17/18. **M2 is 18 gates; `make gates` is 34.**

### State as of 2026-09-03 — a quarter of every render had the CPU idle

**D-146 — a scene needs its own narration, not everybody's.** `findings.md`
F-12, out of the production log: `film.rs` ran `resolve_audio` to completion and
only then `render_segments`, and in **4531 rows of real renders the two phases
never once overlapped**. For 78 seconds of chapter 1 ten cores did nothing while
`edge-tts` waited on the network; then the network sat idle for 228 seconds of
encoding. **24-28% of every cold render.**

`pool::pipeline` sits beside `pool::run` with the same guarantees — both stages
bounded by their own job count, both result vectors in input order, D-045
cancellation in both — and starts a scene's segment the moment *that scene's*
narration is measured.

**D-077 is proven, not argued.** `plan_scene` is the whole of what decides a
segment's identity and none of it can see which worker ran it. The same
twelve-scene project rendered by the old code and the new produced **one
byte-identical film**; ten cold renders in a row produced the same hash.

**Measured with the network stood in for.** The live service is unusable as a
benchmark (three paired runs: 22.4/25.1/38.1 s against 15.5/24.3/68.2 s, one
plainly a D-094 retry), so `edge-tts` was replaced by a script that sleeps and
then answers — which is what a network is here: waiting, not computing. Twelve
scenes, `--jobs 4 --audio-jobs 2`: **22.43/25.07/23.15 s before,
14.34/16.32/15.03 s after — 35%, three times out of three.**

**It costs no memory, and that was measured** because the two pools now run at
the same time, which is what D-144 sizes. Peak resident across every child:
**2900 MB before, 2904 MB after** at `--audio-jobs 8`; 2909 vs 2905 at
`--audio-jobs 16`. A segment worker holds a prescale canvas and an audio worker
holds nothing, so widening the stage that holds nothing does not move the peak.

**The trade F-12 named is handled.** Under two barriers a failed narration was
known before a frame was encoded; `pipeline` therefore **stops admitting
stage-two work as soon as any stage-one item fails**. Verified end to end: one
failing line out of twelve gives exit 1 in 11 s with **0 segments encoded** and
the scene named. D-002/D-094's pre-flight is unchanged and still runs before
both pools.

**Gate 7e** renders six spoken scenes through the stand-in and asserts a
narration is reported *after* the first segment — an ordering of events, never a
wall-clock number. Against the code before this change: *"first segment at line
8, last narration at line 7"*. **Its first version was wrong in a way worth
remembering:** it copied the project to `overlap-twin` for the byte-identity
half and the films differed — correctly, because `project_id` is the **folder
name** and it seeds the Ken Burns move (D-035). Two renders of "the same
project" have to mean the same project by the renderer's definition.

**M2 is 17 gates; `make gates` is 33.**

### State as of 2026-09-03 — the photographs were smaller than the frame

**D-145 — a still that does not cover the frame is a warning now, and
`still render` prints warnings at all.** This is the first finding in the
project that only *real films* could produce: `findings.md` F-13, read out of
the author's own production log. **699 of 741 rendered scenes are 1376x768
stills going into a 1920x1080 frame** — the whole of chapters 1, 2 and 3-5.
D-034's cover-fit enlarges them 1.41x and the Ken Burns zoom takes another 1.12x
on top, so **every frame of every chapter is a ~1.57x upscale**, and
`still validate` said **"no problems"** for all 699.

**Why nothing caught it:** every fixture in this repo is *larger* than the frame
(1999x1001 up to 4000x3000). The only stills ever tested at size are the ones
the author does not use. Gate 7d now renders the author's real geometry.

**It is a warning, not an error** — upscaling is a trade, and D-089 says a
refusal has to be actionable. What is actionable is the number, so the message
carries it: `2 of 3 stills are smaller than the 1920x1080 frame and will be
enlarged to fill it — the smallest is scene 001 at 1376x768; `--short-edge 756`
renders every scene at its own detail`. **One line for the project, not one per
scene** (`ToolingMissing`'s reasoning, second case): 699 identical lines state
one fact 699 times and the fix is one setting.

Three things worth knowing about the arithmetic. It measures **display** pixels,
so SAR is applied — an anamorphic 1000x720 at 4:3 covers 1333 pixels of frame
and a stored-width check would call it too small. It is **per aspect**, because
the shape decides which edge binds: 1376x768 is **756 landscape and 432
portrait**. And every suggested edge is rounded down to one `OutputSpec::new`
accepts (a multiple of 18 for 16:9, 2 for square), because naming a size the
next command refuses is worse than saying nothing.

**The geometry is carried back rather than probed twice.**
`MediaCheck::check` returns `Result<Option<SourceGeometry>, String>` now: the
probe that decides whether a still is usable already knows how big it is.
`--no-probe` measures nothing and so warns about nothing — the absence of a
measurement is not a small photograph. And the warning **follows D-143's
override**: `apply_geometry_override` recomputes it, or a `--resolution 4k` run
would print a sentence about 1920x1080 at exactly the size where the
enlargement is worst.

**`still render` used to discard the whole warning list.** It counted errors and
threw the rest away, so the surface most operators use never mentioned an
unpaired recording either. `FilmEvent::Warned` carries each one to both control
surfaces, **before the pool starts** — D-144's rule, because a line under five
minutes of progress output is a line nobody reads.

**Round trip, measured:** two 1376x768 stills and one 4000x3000 warn at 1080p,
name `--short-edge 756`, and render silently at 756. Gate 7d reads the number
out of the message rather than writing it down, so the gate cannot agree with
the code by construction. **M2 is 16 gates; `make gates` is 32.** The README
said "30 checks" in one place and "31" in another, because only one phrasing was
ever tested; `readme_claims.rs` now checks both.

### State as of 2026-09-03 — the pool learned what a 4K frame costs

**D-144 — the render pool is sized against the frame, not just the cores.**
Reported from a Windows machine with a GPU: a *small* project at 4K did not
fail, it **froze the machine** hard enough to need the power button.
`runs.csv` ends mid-render with four `ffmpeg` children started 17 ms apart and
then nothing — no completion, no error, no stderr. **That silence is the
evidence**: this code logs a non-zero FFmpeg exit (it does so six times earlier
in the same file), so a missing error line means the process never lived to
write one.

D-076 sized the pool from the core count and **its whole table is 1080p**.
D-143 made 4K reachable from `still render` the night before and nothing
revisited the pool, so `--resolution 4k` kept four workers while asking each for
2.3x the memory. D-044 required RAM-derived capacity and deferred it to M3; a
frozen machine moved it forward.

**Measured** (`ffmpeg-findings.md` §12), peak RSS of one FFmpeg child: 720p
369 MB, 1080p **768 MB**, 1440p 1219 MB, 4K **2630 MB**. The cost tracks the
**prescale canvas** (D-032), not the output frame — 4K's is 11520x6480, four
times 1080p's. Aggregate over eight scenes at `--jobs 4`: **2.9 GB at 1080p,
6.6 GB at 4K**. On 24 GB both finish; on 8 GB the second one is the machine.

`spoonstill_app::capacity` is the new module. A worker costs
`192 MB + 36 bytes x prescale pixels` — a fit that sits **above** every
measurement by 5-29%, because over-estimating costs a worker and
under-estimating costs the machine. It was 128 MB first, clearing 1080p by
**0.7 MB**, which is ordering rather than headroom; a test now demands 2%. The pool is the lower of D-076's core rule and
`70% of RAM / that cost`, floored at one. **`RenderProjectOptions.jobs` is now
`Option<usize>`**: a worker's cost depends on the geometry, and the geometry is
not known until `apply_geometry_override` has run, so a number fixed in
`for_project` was chosen before the question was asked.

**It narrows and never widens.** 8 GB at 1080p still gets four workers and
prints nothing new; only 4K drops, to two. An explicit `--jobs` is **obeyed and
warned about, never overruled** (D-076), the warning names the number to use
instead, and it is printed **before the pool starts** — nothing printed after a
machine freezes gets read.

**The GPU was the reported suspicion and it is the wrong fix.** At 4K: filter
alone 3.59 s, filter + x264 `medium` 4.19 s, filter + x264 `ultrafast` 3.64 s.
The encoder is **14%** of the work, so D-036's "filter-bound" holds at 4K and
holds *harder*. NVENC's ceiling is 1.17x and it would cut memory by **zero** —
the memory is the prescale canvas in the CPU filter graph.

**`SPOONSTILL_MEMORY_BUDGET_MB`** overrides the budget, as much for testing as
tuning: `plan_within`/`pressure_within` take it as an argument, because on this
24 GB laptop every size affords four workers and a test calling `plan()` would
assert the right property on an input that cannot exhibit the defect — D-116's
trap. **CI then caught the same trap in the gate itself**: gate 7c first
compared the two *automatic* worker counts, which passed on this ten-core
laptop and failed on the macOS runner with *"4K got 1 workers against 1080p's
1"* — a small runner derives one worker for every size, so it cannot express
the difference. The budget can be stated; **the core count cannot**, so the gate
now asserts the core-independent form: the same explicit `--jobs 4` fits at
1080p and is warned about at 4K. Checked three ways — here, with
`default_jobs()` forced to 1, and against the unfixed code. **M2 is 15 gates;
`make gates` is 31.**

### State as of 2026-09-02 — sizes, shapes, and the door that was never cut

**D-143 — a size is a name, and the one command that makes a film could not be
told one.** Asked for: render at 2K and 4K, and render a vertical YouTube Short.
Checking what already existed before building anything is the whole reason this
was small: `Aspect` has had all three ratios since D-070, `OutputSpec` has taken
any legal short edge since M1, D-114's ceiling is *exactly* 4K (32 400
macroblocks against 36 864), D-034's cover-fit is aspect-agnostic and D-106's
captions are fractions of the frame. **Nothing about 4K or 9:16 needed
inventing.**

What was missing was the door. `--aspect` and `--short-edge` were on
**`still render-scene`**, which renders one segment; `still render`, the command
that makes a film, had neither, and the window — which cannot edit
`project.yaml` (D-013) — could not ask at all. So you could render one 4K
*segment* and no 4K *film*, and a Short meant hand-writing YAML.

Four things now, and only the first is new capability:

- **`Resolution`** in core — `720p`, `1080p`, `1440p`, `2160p`, with `2k`, `4k`,
  `qhd`, `uhd` and the bare numbers as aliases. It resolves to a **short edge**
  and hands it to `OutputSpec::new`; every rule about evenness and 16:9's
  divisibility by 9 stays in the one place that has ever had them. The short
  edge is what gets named because it means one thing across aspects: **4K
  vertical is 2160x3840**, and an operator who has to work that out will get it
  backwards once. `2k` is documented as 2560x1440 (the consumer usage); DCI 2K
  is 2048x1080 and is a different number.
- **The destination is the shape.** `shorts`, `youtube-shorts`, `reel`,
  `tiktok`, `story` all parse as 9:16. Nobody making a Short is thinking "nine
  by sixteen".
- **`still render` takes geometry as an override** — `--aspect`,
  `--resolution`, `--short-edge`, `--fps`, and `project.yaml` gains
  `resolution:`. `--resolution` and `--short-edge` `conflicts_with` each other
  and naming both in `project.yaml` is a `Problem`: two spellings of one
  setting, so letting one win silently is what D-055 refuses. Applied by
  replacing `project.settings.output_spec` **once**, because the spec is read in
  five places and two of them disagreeing is a segment cached under one geometry
  and asserted against another.
- **The window can ask** — two `<select>`s on Output, filled from
  `output_formats()`, whose pixel dimensions are computed **in Rust per aspect**
  (D-010: a webview that multiplies them is a second `OutputSpec`).

**The cache was already right and that is worth knowing:** `segment_key` carries
`{w}x{h}@{fps}`, so 4K misses every segment, switching back hits every one, and
D-109's two spare generations make flipping between two sizes free after the
first pair. Measured: a second 4K render reused all six segments, **byte
identical**, in 0.69 s.

**One real defect, found by using it.** The scenes grid decided its thumbnail
crop with `project.geometry.startsWith("1080x1920")` — one size of one aspect —
so a 4K Short showed landscape thumbnails. `ProjectView` carries `aspect` and
`short_edge` now, and a `ui_contract` test forbids that pixel string. Both
halves of that test were run against the unfixed page and seen to fail.

**A second defect, same shape.** The Subtitles preview was drawn at a hardcoded
16:9 because the themes are fractions of the frame — true of **scale**, not of
**shape**. The same sentence wraps to two lines landscape and four in a Short,
so a landscape preview of a vertical film is wrong about legibility, which is
the one thing D-106 says it exists to be right about. `subtitles::preview` takes
an aspect now, and choosing a shape redraws it.

**Measured 2026-09-02**, six scenes, `--jobs 2`: every geometry produced a film
of exactly **18.054667 s** — geometry changes the pixels and the narration
decides the length (D-021). 4K landscape 12 MB, 4K vertical 9.5 MB, 2K 7.4 MB.
A one-scene 4K vertical film with `punch` captions burned in took 4.5 s and the
band scaled with the frame.

**M2 is 14 gates and `make gates` is 30.** Gate 7b renders six combinations,
probes each, asserts every duration is identical, asserts 8K is refused, and
asserts the two spellings cannot both be given. `README.md` gained a size table;
`readme_claims.rs` keeps the counts honest.

**Deliberately not done:** no check against a platform's duration limit. A
YouTube Short is three minutes today and was one a year ago, and a gate that
goes stale on someone else's product decision is a gate people learn to ignore.
The *frame* is 9:16 whatever the limit becomes, and that is what is named.

### Later on 2026-08-30 — an outside read, and what running things found

**Two decisions, D-135 and D-136, both from executing something rather than
reading it.** An outside assessment of the tree was checked claim by claim
before any of it was acted on, which is the only reason the two real defects
below were found — neither was in the assessment.

**Check the assessment, not just the code.** Its headline factual claim — *"the
repo is ahead of the tag, `still --version` is 0.1.5 while the latest release is
v0.1.4"* — was **stale**: `v0.1.5` is tagged at `HEAD`, pushed, published and
undrafted with exactly D-133's six assets. D-102's gate had already done its job.
Two claims were true and are now fixed (the README said `make gates` runs 25
checks; it runs **29**), and one was true but not mine to act on (the licence is
D-062 and D-134, and is the author's).

**D-135 — the activity log's lock never worked on Windows.** The Windows CI leg
was **red on `master`** at the commit `v0.1.5` was cut from, in D-122's own test.
It is a race, not a regression: the same code passed that leg twice and failed
once. `append_line` opened `runs.csv` with `.create(true).append(true)`, which
std maps to `FILE_GENERIC_WRITE & !FILE_WRITE_DATA` — neither `GENERIC_READ` nor
`FILE_WRITE_DATA` — and `File::lock` is `LockFileEx`, which needs one of them.
`std::fs::File::lock`'s own documentation says so outright. So the lock returned
`Err` on **every Windows write this program has ever made**, and D-122's comment
("failure here falls through to the write") meant it wrote unlocked. Shipped in
v0.1.1 through v0.1.5. The fix is `.read(true)`. **What hid it is the tolerance,
not the lock** — a failed lock is survivable by design, which is exactly why a
permanent failure produced no signal. D-122's own test comment claimed the lock
could not be isolated; Windows disproved it, and it is corrected in place. Every
other file lock was checked: **D-113's render lock is correct** and was never
affected.

**D-136 — the installers had never been executed anywhere.** Not `install.ps1`
(no PowerShell on this machine, D-128), not `install.sh` in CI. Both are the
first thing a stranger runs and both are piped into a shell. They now run on
every push, and the first run found two more things:

1. **A workflow GitHub rejects runs no jobs at all** — no logs, no annotations,
   `gh run view --log-failed` answering *"log not found"*. `runner.temp` is not
   an allowed context in a job-level `env:`. From which: **CI cannot check its
   own syntax**, so `make workflows` (wired into `make lint`) is local and fails
   loudly when `actionlint` is missing. `actionlint` was **already installed on
   this machine** when the bad workflow went out — `.github/` was simply the one
   corner of the tree no gate covered.
2. **`install.sh`'s error message was unreachable.** Under `set -euo pipefail` a
   failing `curl -f` killed the script at the pipeline, so the `die` on the next
   line could never print. Every likely failure — a rate-limit 403, an offline
   machine, no releases — gave a bare `curl: (56)`. That is D-123's rule broken
   in the installer's first network call. `install.ps1` did **not** have this
   defect and passed on its first execution ever.

Both installer jobs are green, and non-vacuously: the macOS job downloads the
real assets, verifies the checksums, installs, asserts `still --version` carries
the released tag, and asserts `/Applications/spoonstill.app` has **no
`com.apple.quarantine` attribute** — D-099 proven for the first time rather than
reasoned about.

**D-137 — the exit gates run where the author is not.** `make gates` ran in no
CI job at all; 29 checks that render real media had only ever executed on this
laptop. Now on every push, green in ~10 minutes. A shallow clone breaks M0's
fifth gate (`git log | tail -1` is HEAD at depth 1), found with a real
`git clone --depth 1` before pushing. `edge-tts` is deliberately absent on the
runner, so M2's gate 7 finally exercises the half this machine cannot: without a
provider a written line must **fail and name the missing tool** (D-020).

**D-138 — the published one-line installer was broken for every user.** Both
installer jobs failed on the same file within three seconds. That reads exactly
like a flaky CDN, and the drafted fix was `--retry-all-errors`. Running the
installer locally killed that theory: `/releases/latest` was returning
**v0.1.2**, which predates D-133 and has no `SHA256SUMS.txt`, so both installers
correctly refused to install unverified. Cause: `release.yml` ended with an
unconditional `gh release edit "$TAG" --draft=false --latest`, and a re-push of
an old tag marked *itself* latest. `--latest` is now conditional on the tag being
the greatest published version by `sort -V`. **So a 404 is deliberately not
retried in either installer** — retrying would have buried the defect under a
plausible story. The test for the fix also lied at first: it ran under zsh, which
does not word-split, while the workflow runs bash.

**D-139 — the unmaintained parser was not the risk; the caption band was.** An
outside read called `ttf-parser` the project's one supply-chain exposure, "in the
path that processes operator-supplied text". It is not: `ttf-parser` parses
**font files**, and every font here is `include_bytes!`d — nothing loads a font
from a path, config, env or project. Checking that premise found the real defect
in *our* code: a hostile-text corpus produced a **119 420px band on a 1080px
frame**, reachable through the documented pipeline (D-126 permits a 256 KiB
script, D-106 makes a `.txt` beside a recording a caption, and `cues()` bounds
cue *count* by duration, so a short scene forces very long cues). 291.8 MB of
RGBA for one cue, 49 s, times `--jobs`. This is **D-114 one layer down**. Clamped
to the frame — not to `max_lines`, which would silently shorten captions in films
that already exist. D-130's byte-identical assertions still pass.

**D-140 — reordering re-renders the film, and the alternative is worse.** Found
by *using* the tool: a pure reorder reports "0 segments reused", because
`MotionSpec::seeded` puts the scene **index** in the seed, so a photo's move
depends on where it sits. Moving one scene to the front of a 500-scene film
re-encodes all 500 **and changes the motion on scenes nobody touched**. The
obvious fix is refused for D-118's recorded reason. Also fixed there: the CLI
printed `[  1/4] 004`, a completion counter that reads as film order — D-091's
defect, in the surface D-091 calls permanent.

**A hundred scenes, measured** (`ffmpeg-findings.md` §11). Everything in §10 was
fixtures. 100 photos of seven sizes, each with its own recording: import 0.06 s,
validate 5.7 s, cold render **59 s**, film 340.021 s against 340.000 expected —
inside one frame across a hundred joins. Captioned, `--jobs 1` is 2m02 and
`--jobs 8` is 1m02, **byte-identical**: gate 3 asserts that on four scenes, and
it holds at a hundred where the pool saturates and every worker rasterizes text.

**Still not done, and still the author's call:** **the licence (D-062)** —
deliberately not taken, because releasing under an open licence is an
irrevocable grant and the one thing here a commit cannot undo; renaming
`vidio/`; pinning the installer one-liner's mutable `master` URL (it trades
against D-133's six-row release page); a GUI smoke test (**check first whether
`tauri-driver` supports macOS** — it may be Windows/Linux only, which would
confine it to the runner); and M3 versus splitting one long narration on
silence. `PROCESS.md` is done.

### State as of 2026-08-30 — an audit, worked through end to end

**Twenty-eight decisions landed in one session, D-107 through D-134.** They came
from an external audit of the whole tree, and the working rule was: *reproduce
it before believing it, fix it, prove the test fails without the fix.* Several
findings did not survive contact — where the audit was wrong, the decision says
so and says what is true instead.

The whole set, in one line each, newest last:

| | |
|---|---|
| D-107 | a segment's cache key holds the narration it contains |
| D-108 | work for one cache key is done once, not once per worker |
| D-109 | the segment cache keeps three generations, not all of them |
| D-110 | a segment is reused only at the length the plan asked for |
| D-111 | the folder scan sorts, folds case, and refuses to guess |
| D-112 | the film's destination is contained by the same code as its inputs |
| D-113 | the render lock is the operating system's, not the file's |
| D-114 | output geometry has a ceiling, derived from the level table |
| D-115 | the window supervises its own render, and a killed run leaves no litter |
| D-116 | no cue is a flicker, and the count was never the guarantee |
| D-117 | a shadow fits inside the canvas it is drawn on |
| D-118 | a key that holds an operator's words is length-prefixed |
| D-119 | `move_into_place` is one rename, because Rust's already replaces |
| D-120 | a scene file is never seen holding part of a photograph |
| D-121 | an interrupted renumber is finished or undone, never left |
| D-122 | the activity log is locked, and nothing in it can be run |
| D-123 | an installer says why it failed, and a temporary cleans itself up |
| D-124 | what is embedded in the binary is named in the binary |
| D-125 | a release is pinned, scoped, and checked by name |
| D-126 | a file is measured before it is read |
| D-127 | the window's authority is the project it has open |
| D-128 | Windows is a different shell and a different PATH |
| D-129 | eighteen warnings nobody read become one gate that fails |
| D-130 | the caption rasterizer is measured, and then it is fast |
| D-131 | coverage went where the defects were |
| D-132 | Windows is checked by compiling for Windows |
| D-133 | the release page is five downloads and one list |
| D-134 | the README opens with a real render, and it is generated |

**Everything is one commit.** The cache keys changed (D-107, D-118), so the
first render of any existing project re-renders every segment once. That is
correct and it is a one-time cost — about 7½ minutes for 200 scenes here.

**Windows is checked by compiling for Windows** (D-132). Two defects on the
platform D-071 puts in scope and nothing here has ever run, both found by
thirty seconds of machine time rather than by reading:

```bash
rustup target add x86_64-pc-windows-msvc
RUSTFLAGS="-D warnings" cargo check --target x86_64-pc-windows-msvc --all-targets \
  -p spoonstill-core -p spoonstill-media -p spoonstill-state \
  -p spoonstill-tts -p spoonstill-app -p spoonstill-cli
```

(`apps/desktop` is left out: `tauri-winres` wants `llvm-rc`, which this machine
has no linker for. A host limitation, not a code one.) First, D-128 gave
`windows_quote` a `dead_code` exemption and never wrote the symmetric one, so
`posix_quote` is dead code on Windows and `ci.yml`'s `RUSTFLAGS: -D warnings`
would have failed the Windows leg on the **next push** — it had never failed
because it had never been pushed. Second, `titleBarStyle: "Overlay"` is
macOS-only (`title_bar_style` is `#[cfg(target_os = "macos")]` in the pinned
`tauri-runtime-wry`), so Windows keeps its native title bar and `.titlebar`'s
82px of traffic-light room became an empty indent beneath it. `app.js` writes
the platform onto the root element and the stylesheet gives that padding back;
the rule is **positive** (`[data-os="windows"]`) because the negated form would
match in the moment before the module loads and flash macOS. Absent means
macOS, so that machine is unchanged. `ui_contract.rs` pins all three.

A **third** defect followed, and only the Windows runner could find it:
`display_quoting_survives_a_hostile_filename` asserted the POSIX escape against
`shell_quote`, which is the platform-dependent function, so it failed the first
Windows leg that ever got far enough to run a test. The dialect tests
themselves were always right — what was wrong is a test going through the
platform-dependent path and expecting one platform's answer.

**Run that cross-check before cutting a tag, and wait for the Windows CI leg
before pushing the tag.** Compiling for Windows catches lints and type errors;
only the runner catches a wrong expectation. Both are cheap; neither replaces
the other.

**The release page is five downloads and one list** (D-133). v0.1.4 showed
fourteen rows, six of them `.sha256` twins nobody downloads and a `.msi` that
installs the same Windows app as the `.exe` — a question put to the person
downloading that they cannot answer. The build jobs still checksum each asset
and the publish gate still verifies every one **before** undrafting; after that
it folds them into `SHA256SUMS.txt`, uploads it, and only then deletes the
twins. Both installers read the one file and refuse a name that is not in it —
a check that passes by finding nothing to check is not a check. Six rows now,
counting the two source archives GitHub attaches and will not let anyone
remove.

Cutting the tag then found one more, older than D-133: the publish job verifies
checksums inside `mktemp -d`, and `gh` asks `git` which repository it means, so
every call there failed with *"not a git repository"* — six green build legs and
nothing published. That block is **D-125's** and had never executed, because
v0.1.4 predates it and the gate it replaced counted assets without fetching
any. `GH_REPO: ${{ github.repository }}` on the step fixes it, and a test
asserts both that it is set and that the step still leaves the checkout.

**The README opens with a real render** (D-134). `assets/demo/render.gif` —
four scenes, fifteen seconds, motion, cuts on the spoken line, captions burned
in — produced by `make demo`, which runs the actual `still render` through the
actual filter chain. The stills are **generated** by `scripts/gen-demo.py`, the
way the logo is (D-079): the author's photographs are their work, and stock
would be showing off someone else's picture. 640px/10fps/96 colours was picked
by measuring — 720px is 6.2 MB, and at 560px the burned caption starts to break
down, which is the one detail the frame exists to prove. Not in `make gates`
and not in CI: it needs the voice service, and a gate that fails on somebody
else's afternoon is a gate people learn to ignore.

Two neighbouring suggestions from the same outside read were **not** taken, and
D-134 says why: a licence is D-062 and the author's to choose, and a desktop
end-to-end test is D-131's stated omission rather than an oversight. Also
fixed while there: `spoonstill-state` and `spoonstill-tts` had `description`
fields advertising SQLite and ElevenLabs, **neither of which exists** — the
exact thing the top of this file warns about.

#### If you are checking this work

Run `make gates` first: **M0 8/8, M1 8/8, M2 21/21**, plus `cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings` and `cargo test
--workspace` (558 tests). Then `cargo audit --deny warnings` (D-129), which is
new and is the one check that can fail without the code changing.

Each decision names its own reproduction, and most can be re-run in a scratch
folder in under a minute. The four worth re-doing by hand, because their
evidence is the most surprising:

- **D-107** — render a scene, swap the narration for a different one of the same
  length, render again. It used to produce a byte-identical film.
- **D-121** — a 2000-scene project, `still remove` killed 120 ms in. It used to
  leave 433 files invisible and `still validate` saying "no problems".
- **D-120** — needs a non-APFS volume: `hdiutil create -fs "MS-DOS FAT32"`. On
  APFS `fs::copy` is a clone and the window is invisible.
- **D-130** — `cargo test --release -p spoonstill-media --test caption_bench --
  --ignored --nocapture` prints the table.

**Three things are deliberately not covered**, each stated in its own decision:
GUI automation (D-115's close handler and D-127's bound commands are tested at
the seam, not clicked), PowerShell (D-128's `install.ps1` is reviewed, not run —
there is no PowerShell on this machine), and a 900-second installer timeout
(D-123's classification is unit-tested rather than waited for).

**Not done, and not started:** M3. The audit found nothing that changes it.

### State as of 2026-08-26

**M0, M1 and M2 are complete.** `make gates` is 8/8 for M0, 8/8 for M1 and
13/13 for M2 — 29 gates, all green. The four newest all guard the cache:
gate 4b, **a narration replaced by a different one of the same length must
re-render** (D-107); gate 4c, **sixteen scenes sharing one narration generate
it once** (D-108); gate 4d, **the cache is bounded and the last two generations
stay free** (D-109); and gate 4e, **a cached segment of the wrong length is
re-rendered, not reused** (D-110).
M2's gate 7 renders
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

**The one network call is classified, retried, and asked about first** (D-094).
A failing `edge-tts` puts a Python exception on the last line of stderr; that
line is matched on the exception's *class name* and decides everything.
Transient — `aiohttp`, a websocket, a 429, our own timeout — is retried three
times with a growing pause. Permanent — `NoAudioReceived`, `Invalid voice`, and
**anything unrecognised** — is attempted once and reported as a sentence
naming the row and the fix, never as a traceback. The ceiling on one line is
60 s + 30 ms/char rather than a flat 90 s, because 5 980 characters measured
37.6 s here. And `check_voice_service` asks the provider whether it works
**before the pool starts** (D-002), counting only the lines not already in the
speech cache — so a finished project still re-renders on a machine that has
since lost the tool. Two test suites: `edge_retry.rs` fakes the tool with a
script and runs offline in `make test`; `edge_live.rs` provokes each failure
against the real service and is `#[ignore]`d behind **`make tts-live`**, which
is what re-checks the recorded stderr after an `edge-tts` upgrade.

**Long form is split into requests** (D-095). Measured here: 62 000 characters
— a scene's full hour — is one 245-second all-or-nothing request, and a drop at
3:59 costs all of it. A line is split at **9 000 characters**, at paragraph then
sentence then word boundaries, packed so an hour is a handful of requests rather
than fourteen hundred, and joined by **concatenating the bytes** (MPEG frames
need no container fixed up; each seam gains ~26 ms, and duration is measured
anyway — D-021). It is also 2.6× faster than one big request. The 9 000 comes
from the author's own `setuptts`, which drives this service in production; what
was *not* copied is its payload limit, which exists because it uses the Python
library while we use the CLI. And **a line no scene could hold is refused before
the first request** rather than spoken for eleven minutes and then rejected for
its measured duration.

**A probe that decodes is timed by what it decodes** (D-096). Rendering a
six-hour film — six scenes of an hour, `--jobs 4` — failed all four of the
first round at once, not because the segments were wrong but because
`ffprobe -count_frames` could not decode 108 000 frames inside the flat 30-second
`DEFAULT_PROBE_TIMEOUT`. Measured: 18 000 frames take 15.4 s. The ceiling is now
30 s + 5 ms/frame (the multiple is four workers sharing cores, not padding).
`scene.rs` is the **only** place that counts frames; the film's assertion and the
reuse check read headers and are as fast on six hours as on four seconds. Every
gate in this project measures many short scenes — this is the other axis, and it
broke at the first scene.

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
  pure validation rule), `remedy` (a missing tool, as three fields — D-105),
  `captions` (six themes as fractions, and cue splitting — D-106).
  Still zero dependencies.
- `spoonstill-media` — `command` (the only place a process is spawned),
  `probe` (timed, typed), `profile` (`SegmentProfile` + `assert_matches_profile`),
  `scene` (render → validate → atomic move), `audio` (normalize, generate
  silence, measure), `concat` (the join and the film's own assertion),
  `caption` (the subtitle rasterizer — D-106; the one place `fontdue` and the
  bundled Inter weights are used), `atomic` (write-beside-then-rename, shared
  by all of them).
- `spoonstill-state` — `logs`: the JSON Lines sink and the bundle export.
  **Still no SQLite** — that is M3.
- `spoonstill-tts` — the `Provider` trait, `Request`/`Voice`/`TtsError`, and
  `edge`. **Sits above `spoonstill-media`** (D-081), because a provider that
  shells out uses the one process boundary rather than growing a second one.
- `spoonstill-app` — `import` (`settings`, `rows`, and the resolution stage),
  `ingest` (making a project and filling it), `audio` (the cache and
  `AudioSource` resolution, speech included), `pool` (the bounded worker pool),
  `film` (`still render`: two pools, the lock, the join), `render` (one scene),
  `diagnostics`, `tooling` (every external program: checked, and installed —
  D-105), `tts` (the re-export the control surfaces use), `subtitles` (the
  theme list and the renderer's own preview — D-106), and `surface`.
  Owns `serde_yaml_ng` and `csv`; the domain model does not know what a file
  format is.
- `spoonstill-cli` — `still new`, `still add`, `still validate`, `still render`,
  `still render-scene`, `still voices`, `still subtitles`, `still doctor`,
  `still diagnostics export|where`.
- `apps/desktop` — the Tauri 2 window (D-051's review grid, D-083's shape,
  D-085 and D-086's navigation). **Two levels: home is the operator's projects
  plus app-level Settings; a project is a left rail over one dense grid —
  Scenes, Voice, Subtitles, Output, Render** (D-092 removed Runs). All
  translation. The
  design brief and canvas it was built against are in
  `~/Downloads/Desktop application redesign` — read D-083 for what was followed
  and the one thing that was not, then D-085 and D-086 for what the author's own
  use of it changed. **Built ahead of M3**, which M4's entry condition says it
  should not have been; it is a shell over M2 and gains M3's resumability for
  free when M3 lands.

**A scene can be removed and moved, and nothing is ever deleted** (D-100).
`spoonstill_app::arrange` is the module; `still remove DIR SCENE...` and
`still move DIR SCENE POSITION` are the CLI half, and the window's Scenes rows
grew ↑ ↓ and Remove. **The order is the numbers** — under D-050's convention a
scene is every file sharing a numeric stem, so moving one means renaming files,
and a `position:` column would be a second source of truth for order. Four rules
with tests behind them: it works **only where stills are numbered** (a project of
`opening.jpg` is refused, not renumbered); **nothing is deleted** — files move
to `removed/`, which the folder scan never sees; **renaming is two passes**,
because writing `001` over a live `001` destroys a file on Unix and errors on
Windows; and **the whole scene moves together**, or a still is silently unpaired
from its narration and renders with the wrong voice. `still remove` takes
several ids highest-first, since removing 002 renumbers everything after it.

**Every column is on screen at every size** (D-101). The scenes grid was a
`table-layout: auto` table, so the browser sized its columns from their content
— and one line of narration is content 3 000px wide. At the shipped default of
1180x820 the table was 1458px inside a 1318px pane: narration was cut mid-word,
and Audio, Resolved and **the whole arrange column** were outside the window,
which is how an operator concluded there was no way to remove a scene. It is
`table-layout: fixed` now, every fixed width is a token on `:root`, and the
narration column takes the slack. The rule that follows: **a fixed table clips
nothing on its own** — every cell either ellipsizes or wraps, or it runs
silently under its neighbour. Six breakpoints from 1280 down to 820 give up
*content* (a note, a filename, a thumbnail) and never a control; below 1080 the
Remove button becomes ✕ and keeps the word as its tooltip and accessible name.
The floor is the window's own 900x600, not the 1100x700 the stylesheet used to
claim. Three things changed that are not about width: the arrange controls are
visible at rest rather than `opacity: 0` (you cannot hover what you do not know
is there), narration is edited in a **textarea that grows to fit**, and the
render pane has one scroll region instead of two. Verified with a layout audit
across every screen at nine sizes from 760x560 to 2560x1400 in both themes, and
in the real WKWebView window.

**A release asset is named for the person downloading it** (D-098), and
**Gatekeeper's brightest button deletes your download** (D-099). Assets are
`spoonstill-macOS.dmg`, `still-macOS-AppleSilicon.tar.gz` and so on — the
version is in the tag, not three more times in the filename — and `install.sh`
/ `install.ps1` construct those names, so renaming one without the other 404s.
The dialog an unsigned `.dmg` produces on macOS offers **Move to Trash** as its
default; "right-click > Open" is what every document including ours said, and
**Apple removed it in macOS 15**. `install.sh` now installs the window too and
clears `com.apple.quarantine` itself, so the operator never meets the dialog.

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

**Subtitles are burned in, drawn by us, and one of the six is "none"**
(D-106). `subtitles: {enabled, theme, position}` in `project.yaml`,
`--subtitles THEME` / `--no-subtitles` on the command line, `still subtitles`
to list them, and a **Subtitles** screen in the window whose preview is drawn
by the renderer rather than imitated in CSS. Six themes — `classic`, `boxed`,
`band`, `card`, `punch`, `minimal` — every length a fraction of the frame, so
one theme is one design at 720p and at 4K. **Off by default**, because burning
text into pixels is irreversible and the reverse mistake costs one flag.

The load-bearing fact: **`brew install ffmpeg` no longer has `libass` or
`libfreetype`**, so neither the `subtitles` nor the `drawtext` filter exists on
the FFmpeg our own README tells operators to install (measured 2026-08-29:
`No such filter: 'drawtext'`). So `spoonstill_media::caption` rasterizes the
text — `fontdue` plus three bundled Inter weights — and FFmpeg composites it
with `overlay`, which every build has. One `rawvideo` input and one `overlay`
per cue, appended **after** the motion chain's tail so D-033 and D-037 are
untouched, and the existing D-041 profile assertion is what proves they stayed
untouched. **No path ever enters the filter graph** — that is the single reason
this is the same design on Windows and macOS, and there is a test named after
it. Windows are `gte(t,S)*lt(t,E)`, half-open, because `between()` draws two
cues on the frame they share.

Two rules came from rendering the author's own film, which is narrated art with
words already drawn into the pictures: **a cue never ends on a dangling
conjunction** (`carry_weak_endings`), and **placement is an override on both
surfaces** — `--subtitle-position top`, and the window now *sends* what its
position box says instead of only previewing it.

A scene is captioned when it has words: an explicit `caption` column, else the
script it speaks. **A `.txt` beside a recording is now the caption, not a
D-020 conflict** — that turns an error into a working scene, and without it an
operator who records their own voiceover could never have subtitles. Cost at
1080p is now inside run-to-run noise (D-130 re-measured it), no extra memory.

**Coverage went where the defects were** (D-131). Re-measured after all of the
above: **75.09% -> 77.35%** lines overall, `film.rs` **42.9 -> 61.1**, the window
**10.0 -> 18.1** — and `concat.rs` and `media/audio.rs` unchanged at 35.7 and
51.0, because nothing here went near them. The CLI went *down*, 18.5 -> 17.5,
since `still licences` and `--keep-cache` added surface nothing prints in a
test; a number that only moves up is a number being managed. All nine CI tests
the audit asked for now exist, each run against the unfixed code first.
Deliberately open: **no coverage threshold** (a gate on a percentage pushes
effort towards small pure-function tests, which is the opposite of the advice),
the shell gates stay **macOS-only** (Windows CI runs `cargo test --workspace`,
which does render real media through real FFmpeg), and there is **no GUI
automation**, so D-115 and D-127 are tested at the seam and not clicked.

**The caption rasterizer is measured, and then it is fast** (D-130). The audit
asked for benchmarks; the benchmark found **604 ms for one `punch` cue at 4K**,
against 41 ms at 1080p — 14.6x for a 4x area, because cost grew with the
*fourth* power of the resolution. The two free themes, `boxed` and `band`, are
the two with no outline and no shadow, which named the culprits. `dilate` walked
~pi*r^2 disc offsets per pixel; a disc is the union of its rows, so it is now a
sliding-window maximum per row — `O(area*r)`. `box_blur` re-added all `2r+1`
samples per pixel; consecutive windows differ by one at each end, so it carries
a running sum — `O(area)`. Result at 4K: punch 604→**62 ms**, card 238→**19**,
minimal 107→**8**. At 500 scenes that is ten minutes of drawing text becoming
one. **The output is byte-identical** — the originals are kept in the tests as
the definition of right and asserted equal across edges, radii wider than the
mask, and an empty band, and a real `punch` film hashes the same before and
after. Also settled: the "about 5%" cost at 1080p is now **inside run-to-run
noise** (card measured *faster* than no subtitles), so 4K is the only place the
per-cue numbers matter. `caption_bench.rs` is `#[ignore]`d and prints a table —
a timing assertion on a shared runner fails for reasons that are not defects.

**Eighteen warnings nobody read become one gate that fails** (D-129). The
audit's numbers reproduce exactly — 516 crates, **zero vulnerabilities**, 18
warnings — and the finding was not that any is dangerous but that nothing
recorded having looked, and nothing would notice a nineteenth. Triaged per
target with `cargo tree --target <triple> -e normal -i`: the whole GTK3/glib
group **including the only unsound advisory** is absent from macOS and Windows
and present only on Linux, which this project does not ship — so **12 of 18 are
in nothing an operator can download**. Six do ship: five `unic-*` in the window
via `tauri-utils`, and **`ttf-parser` through `fontdue` in every binary on every
platform**, which draws every subtitle. There is **nothing to upgrade to** —
crates.io's newest is 0.25.1, exactly what `Cargo.lock` holds, so it is
unmaintained *at its latest release*. `.cargo/audit.toml` records all eighteen
with the target each reaches and a review date, and CI runs
`cargo audit --deny warnings` as its own job — the one check that can fail
without our code changing. Verified by removing an entry: exit 1, named.

**Windows is a different shell and a different PATH** (D-128). Two places
where code written on macOS is wrong on the platform D-071 says is in scope and
nothing has run on. `shell_quote` emitted POSIX `'\''` for an embedded quote;
PowerShell escapes one by **doubling** it, so `Dad's photos` came out as a line
that will not parse — and that form exists precisely so an operator can paste a
failed command into a terminal (D-016). `posix_quote` and `windows_quote` are
separate now, **both compiled and tested on every platform**, because a rule
that only compiles on Windows is a rule nobody here checks. `cmd.exe` is
deliberately not served — it has no single quoting, so no one form fits both.
The function is `windows_quote` and not `powershell_quote` because the latter
tripped `no_shell_strings.rs`, whose bluntness is the point: **move, do not
widen the guard**. And `install.ps1` tested PATH membership by *substring*, so
a `…\spoonstill\bin-old` entry counted as `…\spoonstill\bin` and the real
folder was never added — `still` not found after a successful install
(reproduced as a shell stand-in). Both installers now stage beside and replace,
like D-119/D-120. **No PowerShell on this machine**: the `.ps1` is reviewed, not
run, and a test asserts the two wrong shapes have not come back.

**The window's authority is the project it has open** (D-127). Not exploitable
today — static CSP, local frontend, no remote content — and both halves are a
rule this codebase already wrote being applied to some commands and not others.
The scope grant's own comment said *"allowed to read them — and only them"* over
an `allow_directory`, which is every recording, script, `removed/` and
`.spoonstill/` besides; it is one `allow_file` per displayed still now. **The
obvious fix is a trap**: Tauri's scope checks its *deny* list first and nothing
removes from allow, so `forbid_directory` on a project navigated away from would
block it for the session and reopening it would show broken thumbnails (checked
in `tauri-2.11.5/src/scope/fs.rs`, not assumed). And `Session`'s note — *"the
frontend must never hand a path to a command that opens something"* — covered
the two commands that open a file while `set_narration`, `add_media`,
`remove_scene`, `move_scene` and `preview_voice` took `root` straight from the
webview. They go through `project_root` now, which **compares** the page's value
rather than ignoring it. **Limit stated:** clicking Remove in the window is
still not covered; what replaces it is a `ui_contract` test that every one of
the five still sends `root`, since a page that stopped would have every edit
refused silently.

**A file is measured before it is read** (D-126). A 191 MB `001.txt` took
`still validate` to **607 MB resident** — the bytes, the `String` and the
trimmed copy — and was reported as a valid narrated scene with **"no problems"**,
because D-095's length refusal lives in the provider and does not run until
render. `MAX_SCRIPT_BYTES` is **derived**: `MAX_SCENE_SECONDS` x
`SPEECH_CHARS_PER_SECOND` x 4 bytes/char = 249 120, rounded to 256 KiB, with a
`spoonstill-tts` test keeping it above every speakable line because the two
constants live in crates that cannot see each other (D-114's shape). The same
folder now validates in **14 MB** and names the problem. `project.yaml` gained a
1 MiB ceiling, labelled honestly as a round number rather than a derivation.
**And `hash_file` existed twice** — `scene.rs` read a whole still into memory
while `film.rs` streamed — two implementations that had to agree on a *cache
key*, or `still render` and `still render-scene` would key the same scene
differently. One now, in `spoonstill-media`, streaming. Not done, and recorded:
the window's recent-projects JSON, which this program writes itself.

**A release is pinned, scoped, and checked by name** (D-125). Every action was
a moving tag — `@v4`, `@stable`, `@v2`, and `cargo-binstall@**main**`, which can
change between two runs of the same workflow — and all four are full commit
SHAs now, **resolved from GitHub rather than written from memory**. The Tauri
CLI was `--version '^2'` (resolves at release time, so two tags could bundle
different CLIs with nothing recording which); it is `=2.11.4`, matching the
`tauri` 2.11.5 in `Cargo.lock`, and `cargo tauri build` gained `--locked`.
`contents: write` was declared once at the top and applied to every step
including the ones that only compile; it is `read` there now, with the four
release jobs asking for `write` themselves. And the publish gate was
`[ "$count" -ge 12 ]` — **twelve of the wrong files is still twelve**, which is
exactly D-098's rename where the release looks complete and every installer
404s. It asserts the **exact set** now and verifies every checksum before
undrafting; the logic was run against four cases first, including twelve-with-
one-renamed. The names live in three files nothing type-checked, so
`release_assets.rs` asserts the workflow, the gate and both installers agree —
in both directions, verified by renaming an asset in one place.

**What is embedded in the binary is named in the binary** (D-124). Three
weights of Inter are compiled in with `include_bytes!` (D-106), and their
licence sat beside the `.ttf` files and went **nowhere else** — release archives
were `tar -czf … still`, one file, and the Tauri bundle declared no resources.
The OFL's condition 2 asks in as many words that *"each copy contains the above
copyright notice and this license"*. Now `THIRD-PARTY-NOTICES.md` ships in both
archives and the window's bundle, **`still licences`** prints it from
`include_str!` so every copy carries it however it was obtained (the second form
the OFL itself offers — which is why the installers were left alone), and a test
asserts the notice contains the licence **read from the fonts' own file**, so
replacing the fonts fails the build until the notice follows. Deliberately not
claimed: the Rust dependency notices, which need generating from `Cargo.lock` at
release time to avoid being wrong by the next `cargo update` — the gap is
written into the notices file itself. `README.md` now separates spoonstill's own
licence (still undecided, D-062) from the third-party material inside it.

**An installer says why it failed, and a temporary cleans itself up**
(D-123). `tools::install` matched every failure with
`Err(_) => "not on this machine"`, but that expression fails four ways and only
`BinaryMissing` means that. The realistic one is **`Timeout`**: the ceiling is
900 s, `brew install ffmpeg` on a slow connection outlives it, and the operator
who just watched Homebrew run for fifteen minutes was told it was not installed
— a *wrong* diagnosis, not a vague one. Reproduction is honest here: the two
failures a test can reach are already handled correctly (a non-executable
candidate is skipped by `locate`; an unexecutable one returns `exited 126`), so
the evidence is the code path plus a unit test over the four error values.
Separately, `Edge::speak` removed its parts in a loop placed **after**
`join_mp3(..)?`, so a failed join left them in the **audio cache — which nothing
sweeps**, unlike the segment directory. Now a `Scratch` guard holds every
temporary and `Drop` removes them; a cleanup loop is only reached on the paths
somebody remembered.

**The activity log is locked, and nothing in it can be run** (D-122). Two
defects in the machine-wide `runs.csv`. It is written by **different processes**
— the one case a per-project lock cannot cover — and had none: four concurrent
renders into a fresh log produced **three header lines, two welded onto other
rows**. Fixed by the file's own lock held across deciding *and* writing,
"fresh" read from the locked file's **length** rather than `path.exists()`, and
one `write_all` instead of three; eight rounds clean afterwards. Second: RFC
4180 quoting is about *parsing*, and every spreadsheet strips the quotes and
then reads a leading `=`, `+`, `-`, `@` or tab as a formula — reachable through
the operator's folder name, which is the `project` column (verified with folders
called `=1+1`, `@SUM(A1:A9)`, `-2+3`). A leading `'` defuses them; **numbers are
left alone**, because a value that parses as `f64` cannot be a payload. The
concurrency test fails against the original but does **not** isolate the lock —
in one process the other two changes carry it — and says so in its own comment.

**An interrupted renumber is finished or undone, never left** (D-121). Every
step of `renumber` is `rename(..)?`, so an interruption leaves files parked
under `.arranging-…` names — which the folder scan ignores as dotfiles.
Reproduced: a 2000-scene project, `still remove` killed 120 ms in, left **433
files parked and 434 scenes gone**, with `still validate` reporting **"1566
scenes — no problems"**. Nothing was deleted (D-100 held) but the photographs
were invisible, and every arrange command then *refused* the project — so the
one surface that could repair it was the one that turned it away. **The journal
is now the filename**: `.arranging-<from>-to-<wanted>.<ext>`, which makes
recovery decidable from the disk — destination free means finish the job,
destination taken means roll back, neither means leave it parked rather than
overwrite a photograph. `recover` runs at the top of `scenes`, so it happens
before the project is *read*. Old-format names are recovered too, because a
folder damaged by the shipped build must be repairable by the build that fixes
it — verified on the real damaged project: 1566 scenes back to 1999, all 2000
photographs accounted for. And `validate` now reports `InterruptedRename`,
counted **before** the dotfile skip that hid them.

**A scene file is never seen holding part of a photograph** (D-120).
`copy_in` opened the **real** destination with `create_new` and then filled it,
so `001.jpg` held incomplete media for the whole copy. **Invisible on APFS**,
where `fs::copy` is a copy-on-write clone — so it was reproduced on the
filesystem the operator's media lives on: a FAT32 volume, where an interrupted
`still add` of a 400 MB photo left **232 MB at `001.jpg`**, a broken scene and a
consumed scene number. The copy now goes to `atomic::partial_path` beside the
destination and the name is claimed *after* it. **What it does not achieve:**
the destination still exists empty for the two syscalls between claim and
rename — removing that needs `hard_link`, which **FAT32 answers `Operation not
supported`** to, on the very volume this matters on. And three obvious tests
here pass against the broken code, because they inspect the end state and the
defect is a window — D-116's trap, walked into while fixing something else. The
test that distinguishes watches the destination's *size* during a 64 MB copy and
catches 1 MB of it.

**`move_into_place` is one rename, because Rust's already replaces** (D-119).
It unlinked the destination first, on a premise in its own doc comment —
*"`fs::rename` … fails on Windows when it already exists"*. True of the raw
`MoveFile` API and of several other languages; **not true of Rust**, whose
`rename` is documented as *"replacing the original file if `to` already exists"*
and which calls `MoveFileExW(.., MOVEFILE_REPLACE_EXISTING)` on Windows
(checked in the pinned toolchain's own source, not from memory). The removal
bought nothing and cost two things: a **window with no artifact in it** — this
function moves the finished *film*, so re-rendering over yesterday's film could
lose yesterday's film — and a **race it then had to handle**, where two workers
sharing one cache entry both unlinked and the loser failed the render. Both
vanish by deleting four lines; `rename` is last-writer-wins. The test is the
property, not the implementation: one thread replaces a file 300 times while
another asks only *is it there*, and it fails on the first replacement against
the old code.

**A key that holds an operator's words is length-prefixed** (D-118).
`segment_key` joined the subtitle fields on `\u{1f}` and handed that to
`fnv1a_fields`, whose own separator is `0x1f` — and whose doc comment states the
precondition *"cannot occur in a path, a project id, or a hex digest"*. That was
**true when written**; D-106 later began feeding it subtitle text, and nothing
checks a precondition whose symptom is a silent cache hit. Constructed: two cues
`["alpha beta", "gamma"]` and one cue `["alpha beta\u{1f}0.000>2.000:gamma"]`
join to the *same string* — two different films, one segment, so one renders the
other's subtitles. `0x1f` **is not whitespace** (measured), so it survives
`normalize` out of a `.txt`. Fixed with `hash::fnv1a_prefixed`, where a boundary
is *stated* as a length rather than *looked for* in the data. **`MotionSpec::seeded`
was deliberately left alone**: its `project_id` is the folder name, so the
precondition fails there too, but a collision needs the following field to line
up and that field is a hex digest — not constructible — while changing the seed
would change the Ken Burns move on every scene of every film already made.

**A shadow fits inside the canvas it is drawn on** (D-117). `Mask::shadow`
runs three box blurs — spread `3r` — and the band reserved `shadow_blur * 2`,
so the outermost ring was cut off against the canvas edge. **Measured rather
than assumed: it is real and it is tiny**, alpha 1-2 of 255 on one row for
`card` and `minimal` (the nonzero edges on `band` and `boxed` are their plates,
which have no blur at all). Fixed anyway because it is one constant, the comment
already claimed to reserve the blur in every direction, and the error grows with
`shadow_blur`. Two things explain the evidence: only the **bottom** clipped,
because the offset moves the shadow down so that direction binds and is short by
`r - outline` — which is why wide-outline themes showed nothing; and it clipped
**identically at 720p, 1080p and 4K**, which is D-106's scale-invariance
faithfully reproducing a bug. The count is now `SHADOW_BLUR_PASSES`, used by
both the blur and the margin, because a `3` in one place and a `2` in the other
*is* the defect. The test asserts a property of the **output** — a theme that
casts a shadow leaves its canvas edge transparent — since a test that recomputed
the margin would agree with whatever the code did.

**No cue is a flicker, and the count was never the guarantee** (D-116). The
audit found a tail join whose `.max(1)` produced two cues where one was allowed
— 0.872s + 0.128s in a 1.0s scene, and a **0.192s** cue at 1.5s it did not try.
Reproducing it found the real defect: capping the cue *count* says nothing about
*duration*, because time is shared out by character count with **no floor**, so
a legal three cues in a 3.0s scene ended in one of **0.042s** — a single frame.
The fix merges the worst offender into the neighbour it reads with until every
cue clears `MIN_CUE_SECONDS`; a scene shorter than one readable cue still gets
one cue of the whole scene, which is the floor rather than a violation. The
`.max(1)` removal is kept but is **not** what fixes it — tested by restoring
each defect alone. And `a_short_scene_never_produces_a_flicker` asserted exactly
the right property on a 2.0s scene with balanced pieces, where neither defect
can fire: **a test that checks the right thing on an input that cannot exhibit
the bug is not coverage.** Its replacement sweeps six texts x 120 durations x
three budgets.

**The window supervises its own render, and a killed run leaves no litter**
(D-115). Three claims, and testing them changed the fix. **FFmpeg is *not*
orphaned** — every child gets piped stdin, stdout and stderr, so killing the
parent closes the pipes and four workers were gone within 2 s; do not go looking
for that leak. What *is* left is **temporaries**: the same kill left four
`.seg-….partial-<pid>-N.mp4` files, and nothing removed them — not even D-109's
sweep, which matches only names a segment earned. They are safe to sweep because
D-113's lock is exclusive per project, so any `.partial-` still there when the
film joins is litter from a run that is gone (a stranger's `.notes.txt`
survives). Separately, the window's render slot was released by a statement
**after** `handle.await…?`, so a panicked render left it claimed and every later
render was refused with "a render is already running in this window" until
restart — it is an `ActiveRender` guard now, which a `?` cannot skip and which
runs while a panic unwinds. And there was **no `on_window_event` at all**: the
CLI has had a signal ladder since D-045 and the window had nothing, so
`CloseRequested` now requests cancellation. The close is deliberately not
prevented. **Coverage is partial and the decision says so:** the guard and the
cancel path are unit-tested and fail without the fix; the handler firing on a
real close mid-render needs GUI automation this project does not have.

**Output geometry has a ceiling, derived from the level table** (D-114).
`OutputSpec::new` had no upper bound and unchecked arithmetic, and it is
reachable from `project.yaml`, `still render-scene --short-edge`, **and the
window's `subtitle_preview` IPC command**, whose `short_edge: u32` comes
straight from the webview. Three failures from one missing bound: debug
**panicked** on overflow, release **wrapped** and accepted
`3340530112x4294967292` as *"no problems"*, and `1431655776` produced a
prescale canvas of `3340530176x`**32** — both output dimensions plausible while
the prescale height had wrapped. A non-overflowing `90000` gave a 57 GB RGBA
frame. The cap is **36 864 macroblocks**, which is H.264 level 5.2's `MaxFS` —
*not chosen*: past the table `h264_level` returns 52 regardless, so a larger
frame gets labelled 5.2 while being unplayable by any 5.2 decoder, and D-041
then confirms the label we wrote ourselves. 4K renders; 8K is refused because we
cannot honestly label it. `MAX_MACROBLOCKS` is in core, which cannot import the
table, so a `spoonstill-media` test pins their agreement. With the cap, the
prescale multiply cannot overflow — the invariant belongs to the constructor.
Also fixed the message this exposed: `reason` strings are noun phrases now, so
`` `short_edge`: "…" is not a size this renders `` reads as a sentence instead
of doubling its subject.

**The render lock is the operating system's, not the file's** (D-113). The
lock was a file whose *existence* was the lock — `create_new` to take,
`remove_file` on `Drop` to release, `--force` to overwrite — and the three
compound. Demonstrated: A renders, B `--force`s in while A is alive, **both
run**; B finishes and its `Drop` deletes the shared file, so the project is
unlocked with A still going and a **third** render starts unrefused.
`std::fs::File::try_lock` is stable since Rust 1.89 and this workspace pins
1.94, so the kernel's lock is available with no dependency and no `unsafe`.
**The file's existence now carries no authority and it is never deleted.** Both
defects disappear: a lock cannot go stale (the OS releases it when the holder
dies — verified by killing a render and re-rendering with the file still there
and **no flag**), and one run cannot unlock another (releasing is closing a
handle). `--force` is kept, does not override, and says why. Two things had
encoded the bug: the unit test ended `Lock::take(&root, true).expect("forced")`,
and gate 5 wrote `pid 999999` into the file and demanded a refusal — which was
demanding the tool stay stuck after a crash.

**The film's destination is contained by the same code as its inputs**
(D-112). `destination` checked `output:` **lexically** — no absolute path, no
`..` — and a symlink is neither, so a project carrying `escape -> /tmp` and
`output: escape/film.mp4` wrote its film outside the folder and reported
success. This was already the stated rule (the `output:` setting "is manifest
data and is held to D-054 like every other path"); only a weaker check was
implemented. `path_safety` had done it properly since M2, but `resolve_within`
answers for an *input*, so an absent path is an error — and a destination is
normally absent. The decision is now factored into `resolve_contained`, with
`resolve_destination_within` on top: **reading a file and writing one cannot
drift into two ideas of what "inside the project" means.** Held fixed and
tested: `--out` still goes wherever the operator points it, a symlink that
stays inside resolves to where it really points, a nested destination that does
not exist yet still works, and D-089's trailing-space folder still renders.

**The folder scan sorts, folds case, and refuses to guess** (D-111). Three
defects out of two lines in `from_convention`. `Shot.jpg` beside `shot.wav` was
**two** groups, because the raw stem was in the key — so a recorded voiceover
rendered as a silent still with a "pairs with no image" warning; meanwhile
`ingest::stem_of` had folded case all along, so `still add` paired those files
and the folder scan did not — **one convention implemented twice, disagreeing**.
`001.jpg` beside `001.png` kept whichever `read_dir` yielded first, discarded
the other, and printed **"no problems"** over a scene built from a still nobody
chose; `ProblemKind::DuplicateId` had documented that exact case since M2 but
convention mode collapsed the group before validation could see it, so the
problem it was written for was unreachable. New `AmbiguousScene { slot,
candidates }` at **Error**, naming the files — the operator needs to know which
one to delete. And the scan **sorts before it pairs**, because `read_dir` order
is unspecified and differs by filesystem (D-071). Measured honestly: the winner
was *stable* across creation orders on APFS here, so the audit's claim of
run-to-run nondeterminism is **not** confirmed — what is true is that nobody
decided png should beat jpg.

**A segment is reused only at the length the plan asked for** (D-110). The
reuse check asserted the `SegmentProfile` — container, codec, colour, geometry,
frame rate — which **says nothing about length**, so a file with a segment's
name and shape was reused at any duration and the frame count printed for it
was the *planned* one, which nothing had checked. The film's own D-041
assertion did catch the short film, so no wrong film ever reached an operator;
what it did instead was leave the project **permanently unrenderable** — the
bad entry survived (only a profile failure removed one), so every later render
repeated a failure that blamed a vanished temp file and named no scene, and the
only way out was deleting a folder nobody knows exists. The fix compares the
declared `nb_frames` already sitting in the probe and thrown away, so it is
free: 200 scenes fully cached re-render in 12.35 s against 12.2 s. It stays a
**header read** — D-096 forbids a decoding probe here. Gate 4e asserts the
*recovery*: the entry is dropped, that one scene re-renders, the other is still
reused, and the film decodes to exactly the right frame count.

**The segment cache keeps three generations, not all of them** (D-109).
Nothing ever removed a superseded segment, so a project gained one dead
generation per render forever. Measured in the author's own folder: **1.6 MB of
photographs and scripts had produced 159 MB of `.spoonstill/`** — 52 segments
for 10 scenes, of which at most 10 could be live. Keeping only what the film
used would have been worse, though: choosing a theme means rendering A, B, then
A again, and that would re-encode 200 scenes each flip. So the sweep keeps the
live set **plus two spare generations**, oldest-first — bounded at three times
the film, and flipping back to either recent answer is free. Both halves are one
gate, because each is trivial alone and useless without the other. It runs
**only after the join succeeds**, touches **only names `render_segments` wrote**
(a `holiday.mp4` in that folder survives), never fails a render, and
`--keep-cache` turns it off. **Audio is deliberately not swept** — a segment is
CPU, a narration is a network call and under D-014 money.

**Work for one cache key is done once, not once per worker** (D-108).
`resolve` checked the cache with `path.exists()` and did the whole job on a
miss, so eight audio workers all saw an empty cache and all spoke the same line:
sixteen scenes sharing one narration called `edge-tts` **eight times**. Many
scenes resolving to one entry is the *ordinary* case here — one recording used
throughout, a repeated line, stills with no narration — and against a metered
provider (D-014) that is eight times the bill. It is now one mutex per key, with
the work done under it after a second look. **The fast path takes no lock**, so
distinct keys never contend: 200 scenes cold-render in 433 s against a 459 s
baseline at the same 795 MB. **Only the locked look evicts a bad entry**, because
an unprobeable file may be one another worker is mid-write. The gate uses silent
stills, so it provokes the collision with no network at all.

**A segment's cache key holds the narration it contains** (D-107). The key
had the image, the frame count, the move, the geometry, the encoder and the
subtitles — and not the audio. Frame count is derived from the narration's
*duration*, so duration was standing in for identity, and two takes of the same
length are not the same film: re-recording a line reused the old segment and
the operator got their previous take in a film that reported success. The
identity was already computed — `audio::resolve` stores the normalized artifact
under a content key and used to throw it away — so the fix is `ResolvedAudio.key`
threaded into `segment_key`. **Gate 4 could only ever exercise the hit**; gate
4b renders, swaps a 440 Hz second for an 880 Hz second, and asserts a miss, a
different film, *and* that putting the original back reproduces the first film
byte for byte. It was run against the unfixed key and seen to fail first.

**A missing tool is something to press, not a sentence to read** (D-105). The
Voice screen reported `` `edge-tts` is not on this machine. Install it with
`pip install edge-tts` (or `brew install edge-tts`), press Install in Settings,
or point SPOONSTILL_EDGE_TTS at it. `` — four instructions, three needing a
terminal, one an environment variable, over an empty list, with the only button
that could act on it one level up under Settings. **The screen reported a
problem it could have ended.** (That the machine *had* `edge-tts` at
`/opt/homebrew/bin` is D-103's bug: the installed app was v0.1.0, built before
the fix. **The fix existed and had not shipped**, which is why a version bump
is part of this decision.) A missing tool is now `spoonstill_core::Remedy` —
three fields, not one string: `need` is one plain sentence with no paths and no
flags, `install` is the tool id the window turns into a button, `detail` is the
path and the stderr line, behind a disclosure and in the bundle. `Display`
still prints all of it, so a terminal loses nothing but the button it never
had. `drawFix` in `app.js` is the one component that draws a `Remedy` and it is
on **every** screen that can report a missing tool — Voice, Render, both
Settings cards — and on success it reloads what the tool was blocking, so the
screen you are already looking at becomes the screen that works. **FFmpeg got
the button the voice service already had**: `edge-tts` had one since D-092
while FFmpeg, which every render needs, offered `brew install ffmpeg` and no
way to run it — the screen with the more serious problem could do less about
it. `ffmpeg_status` runs at project open and feeds `renderBlocker()`, so a
machine without it shows a disabled Render button that explains itself next to
the fix (D-089) rather than a wall of broken photographs. `spoonstill_app::tooling`
owns which subsystem holds which binary, because D-010 forbids the window from
reaching `spoonstill-media`; `still doctor [--install]` is the CLI half. This
does not reopen D-012 — every installer is the platform's own package manager,
pressed for, and a test asserts none of them reaches for a URL. And
`apps/desktop/tests/ui_contract.rs` now asserts that every `el("id")` exists in
the markup and every `invoke("cmd")` is registered: both fail **silently** in a
webview — a listener on `null` throws and takes every listener after it, and
the window opens looking correct with half its controls dead.

**D-105's fix had a sibling gap, and it was the one an operator actually hit**
(D-141). Reported 2026-08-31: a Voice screen with `edge-tts` installed and
`provider_status` reporting Ready still showed a wall of Python — an
`aiohttp.client_exceptions.ClientConnectorDNSError`, an `SSLContext` memory
address, `[nodename nor servname provided, or not known]` — with no button
under it. `speech.platform.bing.com` was confirmed reachable from a second
machine at the same time (`dig`, `curl`, `python3 socket.getaddrinfo` all
answered normally); the reported Mac had no route to it at that moment, which
`classify()` already retries correctly as `Transient` (D-094). What was missing
was everything *around* the correct retry: `voices()`'s exhausted message was
the raw stderr line verbatim, and `loadVoices()`'s second `try` — around
`invoke("voices", …)` — was the one branch on the whole screen that caught an
error without calling `drawFix`, the exact defect D-105's own paragraph
describes, in the one place D-105's fix did not reach. `network_hint()` now
turns `classify()`'s own `TRANSIENT` markers into a sentence worth acting on —
never got a socket open (DNS, a refusal) says "check your internet connection,
then a VPN or firewall"; a socket that opened and stopped answering, or a
timeout, says the same; a 429/503/token-skew answer says wait and retry — and
refuses to guess for anything it does not recognise, same restraint as
`classify`'s own default. The Voice screen's `voices()` catch now draws
`voice-fix` too, so "Check again" is there instead of a dead end. **This does
not and cannot make an offline Mac reach Microsoft's service** — `still voices`
and the diagnostics bundle (D-016) read the friendlier sentence for the same
reason a person does, because both go through the one `voices()` function.

**A binary is located, never named** (D-103, D-104). "I open the application
and open the project and it's not opening" was a folder of six photographs
opening as **zero scenes**: a macOS app launched from Finder gets launchd's
`PATH`, `/usr/bin:/bin:/usr/sbin:/sbin`, Homebrew is on none of it, `ffprobe`
was a bare name, and every still failed its D-052 probe. **Every test we own
runs in a shell, which is why nothing caught it.**
`spoonstill_media::tools::locate` now resolves a program to an absolute path —
`PATH` first, then the short list of prefixes the package managers in the
README write to, system *and* per-user (`~/.local/bin` for pipx,
`~/Library/Python/3.x/bin` for `pip --user`, read from the disk because the
number changes). This is not what D-012 refuses: D-012 refuses *downloading* a
build nobody chose, and an absolute path is more reproducible than a bare name,
not less. Three things go with it. A probe that cannot start is asked
`ready()` **once, before any file is read**, so a machine with no FFmpeg
reports one project-level `ToolingMissing` naming `brew install ffmpeg` rather
than one error per photograph. `ProjectView.empty` is decided in Rust, so
"Choose photos…" is for a folder with no photos and a project with problems
goes to the grid where the problem list is. And D-104 applied the same rule to
the *other* binaries the window spawns: `edge-tts` had the identical bug one
screen to the left — the Voice screen said "not installed" over a working
installation — and the Install button that exists to fix that spawned bare
`pipx`, `brew` and `python3`, so the one GUI recovery path was broken by the
same cause as the problem it recovers from. The diagnostics bundle now carries
`edge tooling` and the raw **`PATH`**, which is the one line that settles this
whole class of report. The rule: **`Command::new` is never given a bare name in
this codebase.**

**The tag is the version, and a tag can hold a commit the branch does not**
(D-102). `[workspace.package] version` said `0.1.0` through the v0.1.1 and
v0.1.2 releases, so every published binary answered `still --version` with
`0.1.0`. The number now lives in `Cargo.toml` and `apps/desktop/tauri.conf.json`
and the release workflow's **first** step refuses the job if either disagrees
with the tag — bumping a version is a commit, and the tag goes on that commit.
Separately: `v0.1.2` pointed at a commit that was not on `master` and carried a
`Co-Authored-By` trailer, which is why GitHub listed two contributors while
`git log` on the branch listed one. Re-committing does not remove the old
commit; the tag was moved onto the identical tree on `master`. **Commits in
this project carry no co-author or session trailer.**

**A pinned toolchain owns the targets** (D-097). `rust-toolchain.toml` pins
1.94.0, and that pin beats whatever `dtolnay/rust-toolchain` installed — so its
`targets:` input added the target to *stable* while `cargo build` used 1.94.0,
which has only the host's `std`. Invisible on all three native legs and fatal on
the one cross-compiled leg (`x86_64-apple-darwin` on an arm64 runner), which is
why the first release sat as a draft: D-087's publish gate correctly refused to
undraft at ten assets of twelve. Every job now runs `rustup target add` against
the active toolchain. Every upload uses `--clobber`, so a failed leg is re-run
into the same draft rather than needing the tag re-cut.

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
`<config>/spoonstill/runs.csv`, through `spoonstill_state::Journal` (D-148
replaced D-093's `Tee`). That file
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

**Open decisions:** only D-072 (captions) remains, it does not block anything,
and D-106 has already superseded its default. Read **D-106 before touching
`captions`, `caption`, a theme, the subtitle path, or `move_into_place`.** D-087 was added after M2: read it before touching a workflow, an
installer or `README.md`, and D-089 before touching a path or a disabled
control, D-090 before touching a hostile-name fixture, and D-091 before
touching the live panel or the voice list, D-092 before touching Settings or a
provider's tooling, **D-101 before touching the scenes grid, a breakpoint, or
anything that can be long enough to clip**, and D-093 before touching a log sink, and **D-094 before touching the Edge
provider, a retry, or the pre-flight check in `film.rs`, and D-095 before
touching how a long line is split or joined, and D-096 before giving a probe a
constant timeout, and D-097 before touching a release workflow's toolchain
setup, D-098 before renaming an asset, D-099 before touching an installer's
quarantine handling, D-100 before touching `arrange` or the Scenes rows, and
D-102 before cutting a release, bumping a version, or writing a commit
message, D-103/D-104 before spawning a program, touching `tools::locate`,
the `MediaCheck::ready` pre-flight, or the window's empty-project screen, and
**D-105 before touching `Remedy`, `spoonstill_app::tooling`, `drawFix`, or
anything that reports a missing program to an operator**, **D-143 before
touching `Aspect`, `Resolution`, `spoonstill_app::formats`, the geometry
override in `film.rs`, or the Output screen's choosers**, **D-145 before
touching `SourceGeometry`, `MediaCheck::check`, `undersized_sources`,
`ProblemKind::UndersizedSources` or `FilmEvent::Warned`**, **D-146 before
touching `pool::pipeline`, `resolve_and_render`, `plan_scene`, or the order the
narration and segment stages run in**, **D-147 before touching `prune`,
`prune_audio`, `AudioCache::is_derived_name`, or anything that removes a file
from a cache**, **D-148 before touching `Journal`, `runs.csv`, the CLI's
`run`/`dispatch` split, or the window's `journalled` wrapper**, **D-149 before
touching `Backoff`, `wait_until`'s loop, `probe_jobs`, or the `Sync` bound on
`MediaCheck`**, **D-150 before touching `Ingested::summary`, `unreadable`,
`human_size`, `arrange::Moved`, or a cited D-number**, **D-151 before touching
`Spoken::voice`, `tools::version_line`, the "film complete" event's fields, or
the FFmpeg version line in this file**, **D-152 before touching
`TEXT_EXTENSIONS`, `POSITIONAL_TEXT_EXTENSIONS` or `ingest::assign`**, **D-153
before touching `MotionSeed`, `MotionSpec::seeded`, the segment filename,
`occurrences_of`, or what `create_project` writes**, and **D-154 before starting
M3, writing `state.db`, or assuming resume needs one**, **D-155 before writing
a test that spawns a program, or naming a binary a `/bin/` path**, **D-156
before touching `create_project`, `newProject`, `Session`'s root, or anything
that decides which project the window has open**, **D-157 before touching
`Face`, `Fonts::runs`, the shaper, a bundled font, `caption::undrawable` or
`ProblemKind::UndrawableCaption`**, **D-158 before touching
`spoonstill_core::language`, `DEFAULT_VOICES`, `default_voice_for` or
`wrong_script`**, **D-144 before
touching `spoonstill_app::capacity`, `pool::default_jobs`, `RenderProjectOptions.jobs`,
or anything that decides how many scenes render at once**, and **D-142 before touching
`without_verbatim_prefix`, a `canonicalize` call site, or where the Windows
FFmpeg is looked for**, and **D-141 before
touching `network_hint`, `classify`'s markers, or a failure path on the Voice
screen** — the sibling of D-105 for a transient network error rather than a
missing tool. D-054 through D-057 and D-075 through D-082 were added during M2 and
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

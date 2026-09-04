# findings.md — the open list, macOS measured and Windows pending

**What this file is.** A working backlog of everything a full scan on
2026-09-02 turned up, one row per finding, each with the evidence that produced
it and the fix it implies. It is **not policy**. It ranks below every document
in `CLAUDE.md`'s precedence block:

```
decisions.md > plan.md > refergit.md > ffmpeg-findings.md > findings.md
```

A finding leaves this file by becoming a **D-number in `decisions.md` in the
same commit as its fix**, or by being struck through with the reason it was
refused. Nothing here is decided until it is there.

**How it gets used.** One at a time, in the ranked order at the bottom. The
goal is a tool that is equally good on **macOS and Windows** (D-071), and half
of this file cannot be filled in from this machine — §6 says exactly what to
capture on the Windows box so the other half stops being guesswork.

---

## 0. Provenance — what was actually run

Everything below was executed on 2026-09-02 against `5dedba6` (tag `v0.1.6`),
the tree fully in sync with `origin/master` (0 ahead, 0 behind).

| | |
|---|---|
| Machine | macOS 25.6.0, arm64, 10 cores |
| FFmpeg | **9.0.1** (Homebrew) — *not* the 8.0.1 the docs claim |
| Rust | 1.94.0, pinned by `rust-toolchain.toml` |
| `cargo test --workspace` | **475 passed, 0 failed**, 25 suites, 7 ignored |
| `make gates` | 29/29 green (previous session, same tree, unchanged since) |
| Production data | `mac-runs.csv`, 4531 rows, 2026-08-29 → 2026-09-02, 18 renders, 741 scenes, **0 warnings, 0 errors** |

The production data is the author's own work — five chapters of *Bringing The
Farm To Live In Another World*, rendered mostly onto a network volume
(`/Volumes/home`). It is the first evidence in this project from real films at
real length rather than from fixtures, and four of the findings below exist
only because of it.

**Nothing found produces a wrong film.** Every item is a wrong *message*, a
wasted *resource*, or a blind spot. The 18 runs in the log all completed.

---

## 1. Correctness — wrong output, verified in the tree

### ~~F-01 … F-07~~ · **Done — D-150.** Seven wrong messages

- **F-01** `still new` counted captions as lines to speak and contradicted
  `still validate`; the row dropped the caption entirely. Both fixed —
  `lines_to_speak` and `captions` split by the rule the renderer uses.
- **F-02** `ffprobe exited 1` now, not `ffmpeg exited 1` over an `ffprobe` argv.
- **F-03** an unreadable recording leads with a sentence; the argv and stderr
  follow it, indented. *(The window's disclosure is deliberately not built.)*
- **F-04** `human_size`: `is 390 KB of text`, not `is 0 MB of text`.
- **F-05** six D-099 citations corrected to D-100, two of which printed in
  `still --help`. `cited_decisions.rs` now fails on a `D-nnn` that is not in
  `decisions.md` — which cannot catch D-099-for-D-100 and says so.
- **F-06** `still diagnostics where` no longer prints the project log's status
  under the machine-wide line.
- **F-07** `still move` prints `003 → 001   moved from 3 to 1 of 3` and the
  files that travelled.

---

## 2. Efficiency — measured on this machine

### ~~F-08, F-09, F-10~~ · **Done — D-149.** `still validate`, 11.8 s → 1.31 s

The poll is `clamp(waited / 8, 1 ms, 20 ms)` — proportional, not fixed and not a
plain doubling, which measurably gives most of the win back (1.62 s against
1.31 s). Rows are probed `probe_jobs()` at a time. **F-10 was already fixed by
D-146**, which moved `plan_scene` — and the still's hash with it — inside the
segment pool.

| 200 scenes | one at a time | eight at a time |
|---|---|---|
| 20 ms fixed poll (before) | **11.82 s** | 1.65 s |
| proportional (now) | 7.64 s | **1.31 s** |

Render, honestly: 60 scenes cold, 7.57 s → 7.50 s. **About 1%** — it is a
`still validate` fix, exactly as F-08 predicted.

### F-11 · Two quadratics that are not currently reachable — *no action*

`unlisted_images` does a `canonicalize` syscall plus a `Vec::contains` per
image file (`import/mod.rs:517–548`), and the scene loop does a
`drafts.iter().position()` per scene (`:322`). Both are O(n²) on paper.
**Measured: the non-probe cost scaled exactly 2x from 100 to 200 scenes**, so
neither is material at real sizes. Recorded so the next reader does not
re-derive it. Revisit only if a project exceeds a few thousand scenes.

### Checked and found already optimal

Fonts are `OnceLock`-cached (`caption.rs:422`); `prune_segments` uses a
`HashSet` and one `read_dir`; the caption rasterizer is D-130's fast version;
`hash_file` is streaming and exists once (D-126). No other hot path does work
twice.

---

## 3. From the production log — findings only real films could produce

### ~~F-12~~ · **Done — D-146.** The two phases overlap now

24-28% of every cold render had the CPU idle while `edge-tts` waited.
`pool::pipeline` starts a scene's segment as soon as **its own** narration is
measured. Measured with the network stood in by a script that sleeps: twelve
scenes, `--jobs 4 --audio-jobs 2`, **23.6 s → 15.2 s (35%)**, three reps out of
three, and the film is byte-identical to the one the old code produced. Peak
memory 2900 → 2904 MB. A failed narration still stops the run before segments
are encoded.

### ~~F-13~~ · **Done — D-145.** An undersized source is a warning now

699 of 741 real scenes are 1376x768 into a 1920x1080 frame and nothing said so.
`ProblemKind::UndersizedSources` is one project-level warning naming the count,
the frame, the smallest still and the largest `--short-edge` that renders every
scene at its own detail. It follows D-143's geometry override, and `still
render` prints warnings at all now — it used to discard the whole list.

### ~~F-14~~ · **Done — D-147.** The derived layer is swept; the spoken one is not

D-109's rule now covers the normalized WAV, which is one local FFmpeg pass from
the MP3 beside it, and never `spoken-*`, which is a network call and D-014's
money. Measured on eight scenes under six voices: derived **70.5 MB → 35.2 MB**,
spoken unchanged at 2.25 MB with all 48 files intact. The bound is three
generations however many renders happen. Flipping back to a recent voice still
resolves nothing.

### ~~F-15~~ · **Done — D-148.** Every command reaches the log now

The sink was built inside `render_project`, so renders were all it ever held —
the DNS failure of 2026-08-31 is absent from a file with 1508 rows for that day.
`Journal` replaces D-093's `Tee` and owns the `Option` pair, so a surface asks
for one instead of writing eleven lines. One wrapper above `dispatch` covers all
fourteen CLI commands; sixteen window commands hand their outcome to
`journalled`, with four exemptions named and a `ui_contract` test that fails on
a fifteenth. Asking about a folder does not adopt it: no `.spoonstill` is
created by a question.

### ~~F-16, F-17~~ · **Done — D-151.** The log records what happened

`Spoken` carries the voice that actually spoke, reported by the provider that
resolves it rather than by the caller applying the rule twice. "film complete"
carries `reused_segments`, `reused_audio`, `spoken` and `freed_bytes` — measured:
a cold run reads `spoken=3`, the run after it reads `reused_segments=3;
reused_audio=3; spoken=0`.

### F-18 · The join tail tracks the destination volume — *information, not a defect*

Concat start → film complete: **6.4 s** writing to `~/Downloads`, **38.9 s**
writing to `/Volumes/home`. The join is a stream copy, so this is the network
and nothing else.

Worth knowing rather than fixing: `--out` onto local disk is ~30 s cheaper per
chapter than rendering straight to the NAS.

---

## 4. Documentation drift

### ~~F-19, F-20~~ · **Done — D-151.** Which FFmpeg made this film

`CLAUDE.md` said 8.0.1; `still doctor` now says **9.0.1** and prints it. Every
render records `ffmpeg=` and `ffmpeg_path=` — one `-version` spawn per run, one
implementation shared with the diagnostics bundle so the two cannot disagree.
`ffmpeg-findings.md` keeps its 8.0.1: it is provenance. D-142 is in the
read-before-touching list, and the counts move with the work.

---

## 5. Standing, recorded, not new

### ~~F-21~~ · **Done — D-153.** The motion seed is versioned

`motion_seed:` in `project.yaml`. Absent is `v1` — every folder made before the
key existed, rendering the film it always rendered, verified **byte-identical**
against the pre-change build. `v2` drops the scene index, so a photograph's move
is a property of the photograph: **6 of 6 segments reused** after a reorder.
`still new` writes the key. The index was in the segment *filename* too, which
is why fixing only the seed measured no improvement at all.

### ~~F-22~~ · **Done — D-152.** A `README.md` is no longer guessed into a scene

The harm was the *guess*, not the extension. `POSITIONAL_TEXT_EXTENSIONS` is
`["txt"]`, so a `.md` still pairs by stem — `001.md` beside `001.jpg` keeps
working, which removing `md` from the list would have broken silently — and is
never dropped into a still that did not name it. The refused file is reported
with what to do about it. Markdown markup is spoken literally anyway, so nothing
worth having is lost.

### ~~F-23~~ · **Measured — D-154.** M3's goal is met; its deliverables are not

M3's own exit gates, run against a 500-scene 1080p fixture: cold render
**161/151/154 s**, peak RSS **2873 MB**, three cold films byte-identical, killed
at 60 s with 167 of 500 done → **resume reused exactly 167** and produced an
identical film, one edit → **499 of 500 reused**. **Resume works because the
cache is content-addressed, not because anything remembers.**

Still owed: `state.db` **as an index for reporting** (not for resume),
transitions (D-057), and an integration test for the mismatched-segment refusal.
RAM-derived pool sizing was already delivered by D-144. plan.md §M3 now carries
the numbers.

---

## 6. Windows — what is unknown, and exactly what to capture

**Nothing in §0–§5 was measured on Windows.** Every number above is macOS
arm64. D-071 puts Windows in scope; D-132 checks it by compiling for it; D-137
runs the gates in CI. What has **never** happened is a real project rendered
end to end on Windows with the log kept — which is precisely what D-142 was
found by, and D-142 was found by hand rather than by any gate.

These are the open questions, written as predictions so the data can refute
them.

### F-W1 · Is the D-142 fix actually complete on a real folder?

`without_verbatim_prefix` (`path_safety.rs:304`) strips `\\?\` at the four
`canonicalize` sites in `spoonstill-app`. It handles UNC deliberately:
`\\?\UNC\server\share\x` becomes `\\server\share\x`, and it declines to
shorten a `\\?\Volume{…}` GUID or anything that would land at or past
`MAX_PATH`, because there the prefix is the only reason the file opens.

**Unknown, and the highest-risk item in this file:** the concat demuxer
resolves each relative entry against the list file's own directory using its
own parser — that is exactly what broke on `\\?\`. A share path
`\\server\share\...` is *also* not a drive-letter path. Nobody has joined a
film whose segments live on a UNC share. **The author's real media lives on a
network volume**, so on Windows this is the likely shape, whether as a UNC path
or a mapped drive letter (which behaves differently again). Also unknown:
whether any other path reaches the concat list or a filter graph by a route
that skips those four sites.

### F-W2 · Does `winget install Gyan.FFmpeg` get found, on a machine that did it the documented way?

D-142 added `WinGet\Packages` scanning. Verified against a planted layout in a
test; never verified against a real winget install.

### F-W3 · How much worse is F-08's poll floor on Windows?

Prediction: **worse, possibly much worse.** `CreateProcess` costs far more than
`fork`/`exec`, and Defender scans each spawn. If a probe there takes 60–80 ms,
a 20 ms floor costs proportionally less per call but the calls themselves cost
more — so `still validate` on Windows may be slow for a different reason than
on macOS, and F-09's parallelism would matter more, not less. **Unknown until
measured.**

### F-W4 · Does the render pool size sensibly?

`available_parallelism() / 2` capped at 4 (D-076), with 780 MB per worker
measured on macOS. Memory per worker on Windows is unmeasured.

### F-W5 · Has `install.ps1` ever been run by a person?

D-128 says it is reviewed, not run — there is no PowerShell on the macOS
machine. D-136 put it in CI, which is a runner and not an operator's box with
an existing PATH, an antivirus, and a `Program Files` that needs elevation.

### F-W6 · Does the window look right?

D-132 fixed `titleBarStyle: "Overlay"` being macOS-only and gave
`[data-os="windows"]` its padding back, and `ui_contract.rs` pins all three
rules. Nobody has looked at the window on Windows.

### What to capture on the Windows machine

Run a real chapter — the same media, so the two logs are comparable — and send
these five things:

```powershell
still --version
still doctor
still validate "<the project folder>"
still render "<the project folder>" --out "<somewhere local>" --jobs 4
still diagnostics where          # prints where runs.csv is
```

Then upload:

1. **`runs.csv`** — `%APPDATA%\spoonstill\runs.csv`, the same file as
   `mac-runs.csv`. This is the one that answers F-W1, F-W3, F-W4 and most of
   §3 all at once.
2. **`still doctor` output** — answers F-W2 outright.
3. **`still diagnostics export --project <folder> --out bundle.txt`** — carries
   the raw `PATH`, which D-104 says is the line that settles this whole class
   of report.
4. **Wall-clock times** for validate and render, so F-08/F-09/F-12 get a
   Windows column.
5. **A screenshot of the window** on the project screen — F-W6, and the only
   one a log cannot answer.

If the render **fails**, that is more useful than if it succeeds: send the
error verbatim and the `.spoonstill/logs/` JSON Lines beside `runs.csv`.

---

## 7. The order to take them in

Ranked by what it returns, not by what it costs.

| # | Finding | Why it is here | Becomes |
|---|---|---|---|
| ~~1~~ | ~~**F-13**~~ | ~~Every frame of every real film is upscaled and nothing says so~~ | **D-145** |
| ~~2~~ | ~~**F-12**~~ | ~~A fifth of every cold render, on both platforms~~ | **D-146** |
| ~~3~~ | ~~**F-14**~~ | ~~861 MB on the author's NAS, 97% of it recreatable locally~~ | **D-147** |
| ~~4~~ | ~~**F-15**~~ | ~~The log that exists to answer "what went wrong" misses most of it~~ | **D-148** |
| ~~5~~ | ~~**F-08 + F-09 + F-10**~~ | ~~`still validate` 15 s → ~1.5 s; render ~1.5%~~ | **D-149** |
| ~~6~~ | ~~**F-01 … F-07**~~ | ~~Seven wrong messages, all small, all independent~~ | **D-150** |
| ~~7~~ | ~~**F-16, F-17, F-19, F-20**~~ | ~~The log and the docs stop lying~~ | **D-151** |
| ~~8~~ | ~~**F-22**~~ | ~~A `README.md` in the folder gets spoken and billed~~ | **D-152** |
| ~~9~~ | ~~**F-21 (D-140)**~~ | ~~Largest wart; D-118 already refused the obvious fix~~ | **D-153** |
| ~~10~~ | ~~**F-23 (M3)**~~ | ~~The milestone~~ | **D-154** — goal met, deliverables scoped |

**§6 is not in this order.** It runs as soon as the Windows data arrives, and
it may reorder everything below it — a broken join on a UNC share outranks all
ten.

---

## 8. Status

Nothing in this file is fixed yet. Update this table as each lands, and delete
the row's finding from §1–§5 once its D-number exists.

| Finding | Status | D-number | Notes |
|---|---|---|---|
| F-13 | **done** | **D-145** | undersized sources are a warning, and `still render` prints warnings |
| F-12 | **done** | **D-146** | the narration and segment stages overlap; 35% off a cold render |
| F-14 | **done** | **D-147** | the derived audio layer is swept, the spoken layer never |
| F-15 | **done** | **D-148** | every command and every window handler reaches `runs.csv` |
| F-08, F-09, F-10 | **done** | **D-149** | `still validate` 11.8 s → 1.31 s; F-10 was already D-146 |
| F-01 … F-07 | **done** | **D-150** | seven wrong messages, one of which printed in `still --help` |
| F-16, F-17, F-19, F-20 | **done** | **D-151** | the log records the voice, the reuse counts and the FFmpeg build |
| F-22 | **done** | **D-152** | a `.md` pairs by stem only, and says so when it does not |
| F-21 | **done** | **D-153** | the motion seed is versioned; a reorder re-encodes nothing |
| F-23 | **measured** | **D-154** | M3's goal is met by the cache; the database is an index |
| F-11, F-18 | open | — | recorded as *no action* and *information*; nothing to do |
| F-W1 … F-W6 | **blocked — awaiting Windows data** | — | see §6 |

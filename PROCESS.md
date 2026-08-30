# How this project is worked on

`decisions.md` records *what* was decided. This file records *how* — the working
rules that produced it, each one earned by a defect that got past an earlier
version of the same rule.

It is short on purpose. Every claim below names a decision you can open, and
most name a reproduction you can run in a scratch folder in under a minute.

---

## The rule everything else comes from

**Reproduce it before believing it. Fix it. Then prove the test fails without
the fix.**

All three parts do work.

*Reproduce it* is what stops a plausible report from becoming a change. An
outside audit of this tree produced twenty-eight findings; several did not
survive contact, and where the audit was wrong the decision says so and says
what is true instead. D-111 is the clearest: the audit claimed a folder scan was
non-deterministic run to run. Measured here, the winner was **stable** across
creation orders on APFS. That claim is not confirmed and the decision says so —
while the two *real* defects in the same two lines are fixed.

*Prove the test fails without the fix* is what stops a test from being
decoration. It has caught tests that could never have failed (D-107), tests that
asserted the right property on an input that could not exhibit the bug (D-116),
and tests that had the bug **written into them as the expected answer** (D-113).

---

## Thirteen things this project learned the hard way

**1. A test that checks the right thing on an input that cannot exhibit the bug
is not coverage.** `a_short_scene_never_produces_a_flicker` asserted exactly the
right property — on a 2.0-second scene with balanced pieces, where neither
defect could fire. Its replacement sweeps six texts × 120 durations × three
budgets, and found a cue of **0.042 s**: one frame. (D-116)

**2. Some defects are windows, not states.** `copy_in` filled the real
destination in place, so `001.jpg` held a partial photograph for the whole copy.
Three obvious tests pass against that code, because they inspect the end state.
Worse, it is **invisible on APFS**, where `fs::copy` is a copy-on-write clone —
so it was reproduced on a FAT32 volume, where an interrupted `still add` of a
400 MB photo left 232 MB at `001.jpg`. The test that distinguishes watches the
destination's *size* during the copy. (D-120)

**3. A precondition that was true when written is not a precondition.**
`fnv1a_fields` documented that its `0x1f` separator "cannot occur in a path, a
project id, or a hex digest". True — until D-106 began feeding it subtitle text.
`0x1f` is not whitespace, so it survives normalization out of a `.txt`, and two
different cue sets join to the same string. A boundary is now *stated* as a
length rather than *looked for* in the data. (D-118)

**4. A check that passes by finding nothing to check is not a check.** The
release gate was `[ "$count" -ge 12 ]`. Twelve of the *wrong* files is still
twelve — which is exactly the rename where a release looks complete and every
installer 404s. It asserts the exact set now, and verifies every checksum before
undrafting. (D-098, D-125)

**5. Asserting the shape says nothing about the size.** A cached segment was
reused if it matched the `SegmentProfile` — container, codec, colour, geometry,
frame rate — none of which is length. A file with the right name and shape was
reused at any duration. (D-110)

**6. Deriving identity from a proxy is not deriving identity.** A segment's
cache key held the image, the frame count, the move, the geometry and the
subtitles — and not the audio. Frame count comes from the narration's
*duration*, so duration was standing in for identity, and two takes of the same
length are not the same film. Re-recording a line returned the previous take, in
a film that reported success. (D-107)

**7. A tolerated failure is a silent failure.** The activity log's lock was
allowed to fail — losing an operator's log row is worse than a rare interleave,
which is right. It is also why nobody noticed that on Windows the lock failed
**every single time**: the file was opened append-only, and `LockFileEx` refuses
such a handle. A lock whose failure is survivable must still be a lock whose
failure is loud. (D-135)

**8. An error path that cannot execute is not an error path.** `install.sh` ran
under `set -euo pipefail`, so a failing `curl -f` killed it at the pipeline and
the carefully written message on the next line could never print. Every likely
failure — a rate-limited 403, an offline laptop, a repository with no releases —
produced a bare `curl: (56)`. (D-123, D-136)

**9. Quoting is about parsing; spreadsheets do not parse.** RFC 4180 quoting was
correct and irrelevant: every spreadsheet strips the quotes and *then* reads a
leading `=`, `+`, `-`, `@` or tab as a formula — reachable through the
operator's own folder name. Verified with folders called `=1+1` and
`@SUM(A1:A9)`. Numbers are left alone, because a value that parses as `f64`
cannot be a payload. (D-122)

**10. A fixture that encodes a hazard must assert that it still encodes it.**
`odd.jpg` was generated at 1998×1000 — **even** — which would have made the
odd-dimension SAR test pass for the wrong reason forever. The same trap is open
to any generated fixture, and to any probe: the black-edge check has a control
test proving it reads 0 on a real letterbox.

**11. Measure before optimising, and assert the output did not move.** The
caption rasterizer was benchmarked because an audit asked for benchmarks, not
because anything felt slow. It was **604 ms for one cue at 4K** — cost growing
with the *fourth* power of resolution. Two algorithmic fixes took it to 62 ms,
and the originals are kept in the tests as the definition of right: the output
is asserted **byte-identical**. (D-130)

**12. A number that only moves up is a number being managed.** Coverage went
from 75.09% to 77.35% overall — and the CLI went *down*, 18.5 to 17.5, because
new surface arrived that nothing prints in a test. That is reported rather than
smoothed. There is deliberately **no coverage threshold**: a gate on a
percentage pushes effort toward small pure-function tests, which is the opposite
of where the defects were. (D-131)

**13. The plausible diagnosis is the dangerous one.** Two CI runners, two HTTP
clients, one file, the same second — a flaky CDN, obviously. The drafted fix was
to retry the 404. Running the installer locally instead showed the 404 was
honest: the API's *latest release* had been re-pointed at a version two releases
old, which predates the checksum file, so the download was for a file that
genuinely was not there. **The retry would have hidden it** — and both
installers were already behaving correctly by refusing to install unverified.
A 404 means the file is not there, which is information. (D-138)

Its companion, from the same hour: the six-case test written to check the fix
**gave the wrong answer first**, because it ran under zsh, which does not
word-split an unquoted variable, while the workflow runs under bash, which does.
The logic was right and the test was lying. A test in the wrong shell is a test
of the wrong program.

---

## Three ways a check can be in the wrong place

**A check that runs only where the author sits will only ever find the author's
bugs.** Every test here runs in a shell; a macOS app launched from Finder gets
launchd's `PATH`, which has no Homebrew on it. So `ffprobe` was a bare name,
every still failed its probe, and the report was *"I open the application and
open the project and it's not opening"* — six photographs opening as zero
scenes. (D-103)

**A check can be impossible to run where it is written.** A workflow GitHub
refuses to parse runs **no jobs at all** — no logs, no annotations. So any
validation step placed inside the workflow is precisely the thing that does not
execute, and workflow validation has to happen before the push. `actionlint` was
already installed on this machine when a bad workflow went out; the tool was
there and nothing ran it. (D-136)

**A check can be written and then never wired up.** `make gates` is what the
README calls "the honest answer to *does this work?*" — 29 checks, every
milestone's exit defined by them — and it ran in **no CI job at all** until
D-137. It had only ever executed on one laptop.

---

## Compiling for a platform is not running on it

The code targets macOS and Windows. Nothing was ever *run* on Windows, so
Windows is cross-compiled before a tag and tested on a Windows runner on every
push. Four defects came out of that, and the order is the lesson:

- Two were found by **compiling**: dead code that would have failed the Windows
  leg's `-D warnings` on the next push, and a macOS-only title-bar style leaving
  an empty indent under a native Windows title bar. (D-128, D-132)
- One was found by **running a test**: an assertion that went through a
  platform-dependent function and expected one platform's answer. (D-132)
- One was found by **running a test that only sometimes fails**: the lock in
  point 7 above, which passed two CI runs and failed the third. Code that passes
  twice and fails once was always wrong and usually got away with it. (D-135)

Compiling catches lints and type errors. Only a runner catches a wrong
expectation. Both are cheap; neither replaces the other.

---

## When an outside reader disagrees

Two suggestions from an outside audit were **not** taken, and the decisions say
why rather than quietly dropping them: a licence is entangled with a separate
recorded constraint and is the author's to choose (D-062, D-134), and a desktop
end-to-end test is a *stated omission* in D-131 rather than an oversight — the
window's failure-path logic is tested at the seam, and the decision says exactly
what that does not cover.

A later reader disagreed again, on both. That disagreement is now argued in the
open rather than settled by whoever wrote last.

The rule underneath: **a decision that is not written down did not happen.** The
project's own name was chosen by the author, contradicted by the session that
chose it, recorded nowhere, and lost for two days. (D-073)

---

## What is deliberately not covered

Stated here so that absence reads as a decision rather than an oversight — each
has a decision of its own explaining the cost:

- **No GUI automation.** The window's render-slot lifecycle and its filesystem
  scope are tested at the seam, not clicked. (D-115, D-127, D-131)
- **No coverage threshold**, for the reason in point 12.
- **`install.ps1` is executed only in CI.** There is no PowerShell on the
  author's machine. Until D-136 it had never been executed anywhere at all.
- **The shell gates are macOS-only.** Windows runs `cargo test --workspace`,
  which does render real media through real FFmpeg. (D-131, D-137)
- **No performance numbers for Windows.** Every measurement in
  `ffmpeg-findings.md` is macOS arm64 and says so.

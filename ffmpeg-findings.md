# ffmpeg-findings.md — measured, not assumed

Every number in §1–§9 was produced on this machine on **2026-08-24**; §10 was
added on **2026-08-26** when rendering became parallel. Each section gives the
command, so any session can re-run it and disagree with data.

**Why this file exists.** The Ken Burns filter is the single largest technical
risk in `spoonstill`, and every planning document so far handled it with warnings
and speculation — "known-bad", "memory trap", "worth benchmarking", "do not
trust any copied filter string". That was correct advice and it left the
decision unmade for three documents running. This closes it.

The evidence here **outranks every claim in the reference repos and in the
retired planning docs**, and it feeds decisions D-030 through D-034 and D-041 in
`decisions.md`. It does not outrank `decisions.md` itself: if a measurement and
a decision disagree, the measurement is a reason to change the decision, not a
licence to ignore it.

## 0. Test environment

| | |
|---|---|
| OS | macOS 26.6.2 (build 25G83), arm64 |
| FFmpeg | 8.0.1, Homebrew `8.0.1_4` |
| Build flags | `--enable-gpl --enable-version3 --enable-libx264 --enable-videotoolbox --enable-neon` |
| Encoders present | `libx264`, `h264_videotoolbox`, `hevc_videotoolbox`, `aac`, `aac_at` |
| Node | v26.6.0 |
| Rust | **not installed** — `rustc` and `cargo` are absent. See `plan.md` M0. |

> The GPL build is fine for development and **not shippable**. See D-062.

Fixtures:

```bash
ffmpeg -y -f lavfi -i "testsrc2=size=4000x3000:duration=1:rate=1" -frames:v 1 land.png
ffmpeg -y -f lavfi -i "gradients=size=3000x4000:duration=1:rate=1:c0=0x203040:c1=0x9090a0" -frames:v 1 port.png
ffmpeg -y -f lavfi -i "testsrc2=size=1999x1001:d=1:r=1" -frames:v 1 odd.png   # odd dimensions
```

`testsrc2` is deliberately high-frequency: it exposes stepping and resampling
loss that a smooth photograph would hide. `odd.png` has odd width *and* height,
which is what triggers §4.

---

## 1. The prescale question — settled

**Claim under test.** `plan/PROJECT_BRIEF.md` §8 proposes
`scale=8000:-1:flags=lanczos` before `zoompan`, and warns it is "not free".
Reconciliation §4.3 calls it "a memory trap": *"At 8000×4500 an RGB frame
buffer is ~108 MB, before ffmpeg's internal copies, times N parallel workers."*

**Method.** Worst case for `zoompan`'s per-frame zoom quantization: a 10 %
zoom spread over 10 s at 1080p30. If the quantizer rounds two consecutive
frames to the same zoom factor, those frames are byte-identical, and
`mpdecimate` will drop one. Unique frames out of 300 is therefore a direct,
objective measure of visible stepping.

```bash
ffmpeg -y -i land.png \
  -vf "scale=-2:${PRESCALE_H}:flags=lanczos,\
zoompan=z='min(1+0.10*on/300,1.10)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=300:s=1920x1080:fps=30,\
setsar=1" \
  -frames:v 300 -c:v ffv1 out.mkv

ffmpeg -i out.mkv -vf mpdecimate=hi=1:lo=1:frac=0.0001 -f null -   # count survivors
```

**Result.**

| Prescale height | × output | Unique / 300 | Duplicate frames | Peak RSS | Wall (300 f) |
|---|---:|---:|---:|---:|---:|
| 1080 | 1× | 188 | **37 %** | 744 MB | 1.22 s |
| 2160 | 2× | 280 | 7 % | 749 MB | — |
| **3240** | **3×** | **300** | **0 %** | **761 MB** | **1.96 s** |
| 4320 | 4× | 300 | 0 % | 772 MB | — |
| 6480 | 6× | 300 | 0 % | 805 MB | 4.73 s |

**Conclusions.**

1. **3× output height is both the floor and the ceiling.** Below it motion
   visibly steps; above it nothing improves and the encode gets slower.
2. **The memory-trap claim did not reproduce.** Across a 6× prescale range peak
   RSS moved 61 MB — 8 %. Wall time moved 288 %. Prescale is a CPU cost.
   `zoompan` fed a single non-looped still holds one input frame, not `d` of
   them, so the 108 MB × N arithmetic does not apply to this form.
3. **A fixed pixel number is the wrong shape of answer.** `8000` is 7.4× for
   1080p and 4.2× for 1080×1920 — the same constant meaning different things per
   aspect ratio. Derive it: `3 * output_height`.

**Control.** A plain static encode of the same still, no `zoompan` at all:

```bash
ffmpeg -y -loop 1 -framerate 30 -t 10 -i land.png \
  -vf "scale=1920:1080:flags=lanczos,setsar=1,format=yuv420p" \
  -c:v libx264 -preset medium -crf 18 base.mp4
```

Peak RSS **1,418 MB** — nearly double every `zoompan` variant above. The
encoder dominates memory, not the motion filter. Size the render pool
accordingly (D-044).

→ **D-032.**

---

## 2. `zoompan`'s `d` parameter — the four forms

**Claim under test.** Reconciliation §4.4 calls `d` "the classic footgun";
`Automated-Video-Generator/src/agentic/orchestrator/render.ts:920` records a
five-hour encode from it; `refergit.md` §4.5 warns of "an apparent infinite
render". None of the documents says which form is correct.

`d` is **output frames emitted per input frame.**

| # | Input form | `d` | Bound by | Measured result |
|---|---|---|---|---|
| 1 | still, no `-loop` | `N` | `-frames:v N` | ✅ exactly N frames, exact duration |
| 2 | `-loop 1` | `N` | `-t <dur>` | ✅ 120 frames / 4.000 s — correct |
| 3 | `-loop 1` | `N` | **nothing** | ❌ **unbounded** |
| 4 | still, no `-loop` | `1` | `-frames:v` | ❌ 1 frame / 0.033 s |

Form 3, the actual hang:

```bash
timeout 25 ffmpeg -y -loop 1 -i land.png \
  -vf "scale=-2:2160,zoompan=z='min(zoom+0.0015,1.2)':d=120:s=1920x1080:fps=30" \
  -c:v libx264 -preset ultrafast -crf 30 f2.mp4
# killed at 25 s: 8,400 frames, 311,857,828 bytes, still accelerating
```

`-loop 1` feeds infinite input frames; `d=120` multiplies each into 120 output
frames; nothing terminates it. That is the five-hour "hang" — it was never
hung, it was succeeding forever.

**`ffmpeg-ai` is form 2, and it works.** `refergit.md` §4.5 flags
`src/ffmpeg_ai/video/composer.py:146-154` as an untrustworthy experiment for
using `-loop 1` with `d=<total_frames>`. Measured: it produces a correct
4.000 s / 120-frame clip, because line 150 passes `-t str(duration)`. The
pattern is not wrong; it is one missing flag away from form 3.

> `ffmpeg-ai` has a **different** real defect at the same site, not previously
> recorded: `composer.py:67` sets the prescale to
> `w, h = int(spec.width / 1.5), int(spec.height / 1.5)` — *below* output
> resolution — and line 145 then ends the chain with
> `scale={spec.width}:{spec.height}`, upsampling back. It renders motion at 2/3
> resolution and stretches the result. That is the opposite of §1's finding.

**Adopt form 1.** It cannot hang, because the frame count is structural rather
than a flag someone can forget.

→ **D-030.**

---

## 3. `zoompan` vs time-based `scale`+`crop`

**Claim under test.** Reconciliation §4.4: *"Also worth benchmarking:
`scale`+`crop` with `t`-based expressions, which are evaluated in float and may
beat `zoompan`'s quantization outright."* `refergit.md` §4.5 asks for the same
comparison. `Automated-Video-Generator` shipped `scale`+`crop` on its production
path.

**Method.** Same still, same output, same encoder settings, 4 s at 1080p30,
3× prescale on both sides.

```bash
# A — zoompan
ffmpeg -y -i land.png -vf "scale=-2:3240:flags=lanczos,\
zoompan=z='min(1+0.12*on/120,1.12)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=120:s=1920x1080:fps=30,\
setsar=1,format=yuv420p" -frames:v 120 -c:v libx264 -preset medium -crf 18 A.mp4

# B — time-based scale+crop
ffmpeg -y -loop 1 -framerate 30 -t 4 -i land.png -vf "scale=-2:3240:flags=lanczos,\
scale=w='iw/(1+0.12*t/4)':h='ih/(1+0.12*t/4)':eval=frame,\
crop=w='min(iw,ih*1920/1080)':h='min(ih,iw*1080/1920)',\
scale=1920:1080:flags=lanczos,setsar=1,format=yuv420p" \
  -c:v libx264 -preset medium -crf 18 B.mp4
```

**Result.** Both: 120 frames, 4.000000 s, 1920×1080, SAR 1:1.

| | Wall | Peak RSS | Bytes |
|---|---:|---:|---:|
| A `zoompan` | **1.08 s** | **757 MB** | 1,712,262 |
| B `scale`+`crop` | 8.32 s | 1,501 MB | 1,827,977 |

**`zoompan` wins by 7.7× on time and 2× on memory.** The hypothesis was
backwards. `scale` with `eval=frame` re-runs a lanczos resample over the entire
3240px-tall prescaled image once per frame; `zoompan` resamples only the crop
window it actually emits.

And per §1, `zoompan`'s quantization — the thing `scale`+`crop` was supposed to
fix — is already fully resolved by 3× prescale, at 1.96 s per 300 frames.

**Why `Automated-Video-Generator` still uses `scale`+`crop`.** Its own comment
at `render.ts:920-924` says it: *"replaced zoompan with a streaming scale+crop
pan. zoompan buffered all `d` frames … and with d=1 on a still image it looped
forever (5h encode)."* They were escaping form 3/4 of §2, not losing a
benchmark. Their workaround also caps motion at 4 % (`zp = max(1.04, 1+0.04)`)
and applies it *after* already downscaling to output size, so it upsamples —
same defect as `ffmpeg-ai`.

Keep `scale`+`crop` documented as the fallback if a future FFmpeg regresses
`zoompan`. Re-run this benchmark before switching.

→ **D-031.**

---

## 4. The SAR trap — reproduced

**Claim under test.** `Automated-Video-Generator/src/agentic/orchestrator/render.ts:1049`:
*"motion FX filters (scale/crop/zoompan) reset the sample aspect ratio after the
early setsar=1 in the base chain, which produced SAR 12160:12159 and broke
downstream concat."*

```bash
# odd.png is 1999x1001 — odd width and height
ffmpeg -y -i odd.png \
  -vf "scale=3240:-2:flags=lanczos,zoompan=z='min(1+0.1*on/60,1.1)':d=60:s=1920x1080:fps=30,format=yuv420p" \
  -frames:v 60 -c:v libx264 -crf 20 sar_odd.mp4

ffprobe -v error -select_streams v:0 \
  -show_entries stream=sample_aspect_ratio,display_aspect_ratio -of csv=p=0 sar_odd.mp4
```

| Chain | SAR | DAR |
|---|---|---|
| `setsar=1` **before** `zoompan`, even-dimension source | 1:1 | 16:9 |
| `setsar=1` **after** `zoompan`, even-dimension source | 1:1 | 16:9 |
| **no trailing `setsar`, odd-dimension source** | **30007:30000** | **30007:16875** |

Reproduced, same class as the reported bug. `scale` computes a corrective SAR
when the rescale is not an exact ratio, and `zoompan` carries it through. A
`setsar=1` earlier in the chain does not survive.

Odd dimensions are not exotic — cropped photos and phone exports hit this
constantly.

**Rule: `setsar=1` is the last filter before `format=yuv420p`, always, with no
condition attached.**

→ **D-033.**

---

## 5. Concat accepts a mismatched segment silently

**Claim under test.** Reconciliation §4.6: *"One mismatched scene corrupts the
join silently."* Stated everywhere, demonstrated nowhere.

```bash
printf "file 'sar_yes.mp4'\nfile 'sar_odd.mp4'\nfile 'sar_yes.mp4'\n" > list_bad.txt
ffmpeg -y -f concat -safe 0 -i list_bad.txt -c copy bad.mp4
echo "exit=$?"
ffprobe -v error -select_streams v:0 -show_entries stream=sample_aspect_ratio,nb_frames -of csv=p=0 bad.mp4
```

Two SAR 1:1 segments with a SAR 30007:30000 segment between them:

```
exit=0
(no error, no warning, no stderr output at all)
result: 1:1,180   duration 6.000000
```

**Exit 0. Silence.** The output declares SAR 1:1 for all 180 frames; the middle
60 will display with the wrong geometry, and nothing in the pipeline says so.

This is the strongest argument in this document for D-040's pinned segment
profile. **FFmpeg is not a validator.** Every segment gets `ffprobe`-checked
against the pinned profile before it is written into the concat list, and the
absence of an FFmpeg error is not evidence of a valid join.

→ **D-041.**

---

## 6. Aspect fit into the prescale canvas — no black edges

**Claim under test.** `plan/PROJECT_BRIEF.md` §8: *"a landscape source in a 9:16
frame needs cover-crop plus clamped pan so the window never leaves the image …
this is where these pipelines produce black edges."*

Worst case: landscape 4000×3000 → portrait 1080×1920.

```bash
ffmpeg -y -i land.png -vf "\
scale=3240:5760:force_original_aspect_ratio=increase,crop=3240:5760,\
zoompan=z='min(1+0.10*on/150,1.10)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=150:s=1080x1920:fps=30,\
setsar=1,format=yuv420p" -frames:v 150 -c:v libx264 -preset medium -crf 18 v916.mp4
```

Output: 1080×1920, SAR 1:1, 150 frames, 5.000000 s.

Black-edge probe — minimum luma across the top 4-pixel border of the first,
middle, and last frame (a black edge would read 0):

| Frame | min luma | max luma |
|---|---:|---:|
| 0 | 24 | 225 |
| 74 | 17 | 229 |
| 149 | 22 | 229 |

No black edge at any point.

**Why it holds structurally.** Cover-fitting into the prescale canvas *before*
`zoompan` means the canvas is already ≥ the output aspect in both axes, and
`zoompan`'s centred `x`/`y` expressions with `zoom ≥ 1` can only ever address a
sub-rectangle of it. The failure mode is removed rather than clamped, which is
why this ordering is mandatory rather than a suggestion.

→ **D-034.**

---

## 7. Audio-driven segment duration

**Claim under test.** D-021/D-022: audio duration is authoritative and video is
built to fit it. Verify that a narration length that is *not* a frame multiple
still produces a frame-exact segment with no A/V drift.

A deliberately awkward 3.717 s narration at 30 fps.
`ceil(3.717 × 30) = 112 frames = 3.733333 s`.

```bash
ffmpeg -y -i land.png -i n.mp3 -filter_complex "\
[0:v]scale=3240:-2:flags=lanczos,zoompan=z='min(1+0.1*on/112,1.1)':d=112:s=1920x1080:fps=30,setsar=1,format=yuv420p[v];\
[1:a]aresample=48000,apad,atrim=0:3.733333,asetpts=N/SR/TB[a]" \
  -map "[v]" -map "[a]" -frames:v 112 \
  -c:v libx264 -preset medium -crf 18 -c:a aac -b:a 192k -ar 48000 -ac 2 seg.mp4
```

| Stream | Value |
|---|---|
| video | h264, 112 frames, 3.733333 s, time_base 1/15360 |
| audio | aac, 48000 Hz, 2 ch, 3.733 s, time_base 1/48000 |
| container | 3.733333 s |

Correct. `apad` then `atrim` to the frame-aligned length is the recipe: **pad
narration up to the frame grid, never trim video down to the audio.** The
0.33 ms audio shortfall is one AAC frame boundary (1024 samples = 21.33 ms) and
does not accumulate, because the concat demuxer re-bases timestamps per segment.

→ **D-022.**

---

## 7b. The full-range JPEG trap — `yuv420p` is not what comes out

Measured 2026-08-26, while building M1. **Not predicted by any prior document.**

A JPEG is full-range. Running the documented D-030/D-034 chain on one — which
ends `setsar=1,format=yuv420p` — and then probing the result:

```bash
ffmpeg -y -i land.jpg -i n.wav -filter_complex "\
[0:v]scale=5760:3240:force_original_aspect_ratio=increase,crop=5760:3240,\
zoompan=z='1+0.1*min(on/111,1)':x='0.5*(iw-iw/zoom)':y='0.5*(ih-ih/zoom)':\
d=112:s=1920x1080:fps=30,setsar=1,format=yuv420p[v];\
[1:a]aresample=48000,apad,atrim=end_sample=179200,asetpts=N/SR/TB[a]" \
  -map "[v]" -map "[a]" -frames:v 112 -c:v libx264 -preset medium -crf 18 \
  -c:a aac -b:a 192k -ar 48000 -ac 2 proto.mp4

ffprobe -v error -select_streams v:0 -show_entries stream=pix_fmt -of csv=p=0 proto.mp4
```

| Expected | Measured |
|---|---|
| `yuv420p` | **`yuvj420p`** |

The `format=yuv420p` filter converts the *layout*; it does not clear the range
flag, which travels on the frame into libx264 and is signalled in the
bitstream. So the segment's declared pixel format depends on the colour range of
the **source image**.

**Why this is a §5 problem, not a cosmetic one.** Two scenes whose sources
differ in range — one JPEG, one PNG — produce segments with different pixel
formats and different rendered colour. Per §5, the concat demuxer joins them
with exit 0 and no warning. It surfaces as "some scenes look washed out" in a
finished 500-scene render, months later.

**The fix, and what did not work.** Setting `-color_range tv -colorspace bt709
-color_primaries bt709 -color_trc bt709` as *encoder* options was tried first:

| Field | With encoder options | With `setparams` in the chain |
|---|---|---|
| `pix_fmt` | `yuv420p` ✅ | `yuv420p` ✅ |
| `color_range` | `tv` ✅ | `tv` ✅ |
| `color_space` | `bt709` ✅ | `bt709` ✅ |
| `color_primaries` | **unset** ❌ | `bt709` ✅ |
| `color_transfer` | **unset** ❌ | `bt709` ✅ |

So the range conversion goes on the prescale (`:out_range=tv`, where the
full-range source first meets a scaler), and the tagging goes in the chain:

```
setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709,
setsar=1,format=yuv420p
```

Verified on `odd.jpg` (1999x1001): all four colour fields correct, SAR 1:1.
`setparams` sets metadata only and does not touch SAR, so §4's rule is intact —
`setsar=1` is still the last filter before `format`.

→ **D-037.**

---

## 7c. The border probe, and proving it can fail

Measured 2026-08-26. §6 checked for black edges by reading the top border of
three frames with a Python helper. M1 needed that in CI, for every case in the
matrix, cheaply — and needed it to check all four borders rather than one.

All four in a single decode, without letting a scaler blend a thin black edge
into its bright neighbour:

```bash
ffmpeg -v info -i seg.mp4 -filter_complex \
"[0:v]select='eq(n\,0)+eq(n\,55)+eq(n\,111)',split=4[a][b][c][d];\
[a]crop=W:4:0:0,scale=64:64:flags=neighbor[t];\
[b]crop=W:4:0:H-4,scale=64:64:flags=neighbor[bo];\
[c]crop=4:H:0:0,scale=64:64:flags=neighbor[l];\
[d]crop=4:H:W-4:0,scale=64:64:flags=neighbor[r];\
[t][bo][l][r]vstack=inputs=4,signalstats,metadata=print:key=lavfi.signalstats.YMIN" \
  -fps_mode passthrough -f null -
```

`flags=neighbor` matters: any interpolating scaler would average a 4-pixel black
edge with the content beside it and lift `YMIN` off zero.

**The control.** §8b's lesson is that a check encoding a hazard must assert it
still encodes it. A border probe that always returned a positive number would
make every black-edge assertion vacuous and nothing else would notice. So the
same probe was run against a deliberately letterboxed file
(`force_original_aspect_ratio=decrease` + `pad`, which is the failure D-034
removes):

| File | `YMIN` |
|---|---|
| cover-fit, landscape 4000x3000 → 360x640 | 37 / 31 / 32 |
| letterboxed, same source and size | **0** |

Both the probe and the control now live in
`crates/spoonstill-media/tests/motion_matrix.rs`.

---

## 8. What is still unmeasured

Do not present any of these as known.

- **Windows.** Every number here is arm64 macOS. Peak RSS, encoder behaviour,
  and path handling all need re-measuring on Windows.

  > D-071 is now **Accepted** — cross-platform from M1, by author decision on
  > 2026-08-26 — which changes the code but **not this entry**. The code is
  > written for both platforms and the Windows CI job is enabled; nothing has
  > yet been *run* there. "Compiles and is written correctly for Windows" and
  > "measured on Windows" are different claims, and only the first is true.
- **Real photographs.** `testsrc2` is a synthetic worst case for stepping.
  Confirm the 3× prescale finding on real JPEGs, including CMYK and
  EXIF-rotated ones.
- **Hardware encoders.** D-036 asserts VideoToolbox bands on slow gradient pans.
  That is reasoning from how these encoders behave, **not** a measurement taken
  here. Benchmark `h264_videotoolbox` against `libx264` on real content before
  the "fast draft" mode ships.
- **n=500 in aggregate.** Everything above is a single segment. Pool sizing
  (D-044), SQLite checkpoint throughput, and total wall time for a 500-scene
  project are M3 measurements.
- **Concurrent peak RSS.** ~1.5 GB per worker is extrapolated from the §1
  control, not measured with N workers in flight.
- **`xfade` cost curve.** Asserted to scale badly with clip count. Unmeasured.

## 8b. Generating an odd-dimension fixture takes three deliberate steps

Measured 2026-08-26, while building `scripts/gen-fixtures.sh` for M0.

§4 proves the SAR trap needs a source with **odd** dimensions. Producing one is
harder than it looks — FFmpeg rounds to even at three independent points, each
silently:

| Attempt | Result |
|---|---|
| `testsrc2=size=1999x1001` | **1998×1000** — source filters round to even |
| `... -vf crop=1999:1001` | **1998×1000** — `crop` aligns to chroma boundaries |
| `... crop=1999:1001:exact=1`, default jpeg pix_fmt | **1998×1000** — `yuvj420p` 4:2:0 cannot represent odd dimensions |
| `... crop=1999:1001:exact=1 -pix_fmt yuvj444p` | **1999×1001** ✅ |

```bash
ffmpeg -f lavfi -i "testsrc2=size=2000x1002:rate=1" \
       -vf "crop=1999:1001:exact=1" -pix_fmt yuvj444p -frames:v 1 odd.jpg
ffprobe -v error -select_streams v:0 -show_entries stream=width,height \
       -of csv=p=0 odd.jpg     # 1999,1001
```

**Why this is worth a section.** A fixture that quietly comes out even makes the
D-033 regression test pass *for the wrong reason* — it would assert `SAR 1:1` on
a source that could never have produced a bad SAR, and would never catch BUG
W2-1. The failure is invisible: the fixture generates without error, the test
goes green, and the guarantee is gone.

So `gen-fixtures.sh` re-probes `odd.jpg` and hard-fails if it is not exactly
`1999,1001`. **A fixture that encodes a hazard must assert that it still
encodes it.**

## 9. Re-running all of it

The commands above are self-contained and need only `ffmpeg`, `ffprobe`, and
`python3`. **Done, 2026-08-26.** These are now
`crates/spoonstill-media/tests/motion_matrix.rs` — the same assertions, run in
CI, across the matrix D-030 requires: three durations × two frame rates × every
V1 aspect ratio × landscape/portrait/square sources × ASCII/Unicode/spaced
paths. The test renders the 54-case cross product of duration × frame rate ×
aspect × source shape and cycles the three path styles across it rather than
multiplying by them; `SPOONSTILL_FULL_MATRIX=1` renders all 162. That sampling
is stated in the test's own header, because a bounded test that reads as
exhaustive is worse than an honestly bounded one.

The matrix renders at a 360-pixel short edge. Every property it asserts is
scale-invariant and the prescale rule under test is a *ratio*, so the finding
transfers; the production size is covered separately by
`the_production_recipe_at_1080p_is_frame_exact`, which reproduces §7's exact
case and checks it against the measured answer.

A measurement that only ever ran once, by hand, on one machine, is a fact with a
short shelf life. Move each of these into a test as soon as there is somewhere
to put it.


## 10. Parallel rendering — measured 2026-08-26 (M2 slice 3)

Everything above was measured on 2026-08-24 against single renders. This
section is the M2 slice 3 addition: what happens when several scenes render at
once. Same machine, same FFmpeg build (§0), **10 cores / 24 GB**.

The benchmark project: twelve 1080p scenes, each a `testsrc2` still paired with
the 3.717 s narration from §7, rendered with `still render --jobs N` from a
cold project (`.spoonstill/` removed before every run) so nothing is reused.

```bash
B=/tmp/bench; rm -rf $B; mkdir -p $B/img $B/audio
for i in $(seq -w 1 12); do cp fixtures/generated/land.jpg $B/img/$i.jpg; done
cp fixtures/generated/n.wav $B/audio/n.wav
printf 'output: film.mp4\naspect: 16:9\nshort_edge: 1080\nfps: 30\n' > $B/project.yaml
{ echo "image,audio_file,duration";
  for i in $(seq -w 1 12); do echo "img/$i.jpg,audio/n.wav,"; done; } > $B/scenes.csv

for j in 1 2 3 4 6 8; do
  rm -rf $B/.spoonstill
  /usr/bin/time -l ./target/release/still render $B --out /tmp/bench-$j.mp4 --jobs $j
done
```

### 10a. The speedup curve flattens at three

| `--jobs` | wall clock | speedup | user CPU |
|---|---|---|---|
| 1 | 13.23 s | 1.00x | 35.68 s |
| 2 | 8.56 s | 1.55x | 40.91 s |
| 3 | 7.24 s | 1.83x | 42.71 s |
| 4 | 7.17 s | 1.85x | 44.53 s |
| 6 | 6.54 s | 2.02x | 45.67 s |
| 8 | 6.90 s | 1.92x | 46.69 s |

Run-to-run noise is roughly ±0.4 s, which is why 6 and 8 are not distinguished
by this data and why the honest reading of the tail is "flat", not "6 is best".

**Why it flattens so early:** look at the user CPU column. At `--jobs 1` the
render spent 35.68 s of CPU in 13.23 s of wall clock — x264 at `medium` was
already using 2.7 cores by itself. So on ten cores the machine is near
saturation at three or four workers, and further workers mostly contend with
the threads of the ones already running. The 6 s floor is the point where
twelve scenes' worth of encoding fills the machine.

This is the measurement behind **D-076**: the default is
`available_parallelism() / 2` clamped to `[1, 4]`, and `--jobs` is uncapped for
an operator who knows their machine.

A smaller run at 540p (the six-scene `renderable` fixture) shows the same
shape: 2.99 s at `--jobs 1`, 1.31 s at `--jobs 6` — 2.3x, and 0.40 s on a
warm re-run where every narration and segment is reused.

### 10b. Memory is ~780 MB per concurrent segment, and it does not flatten

`/usr/bin/time -l` reported `maximum resident set size = 786432000` (750 MiB,
780 MB) for **every** `--jobs` value, which is the measurement's own caveat:
macOS `rusage` reports the maximum RSS of any single waited-for child, not the
sum across children. So this is a **per-worker** figure, and the aggregate at
`--jobs N` is inferred as `N × 780 MB` rather than measured directly.

That inference is the conservative direction, and it is enough for the
decision: four workers is about 3.1 GB, eight is about 6.2 GB, and the 4%
of wall clock between them (10a) does not buy that.

The 780 MB itself is the prescale (D-032): a 3× output-height canvas at 1080p
is 5760×3240 held in the zoompan pipeline, plus x264's lookahead buffers.

> D-044 budgeted "~1.5 GB per concurrent segment worker until measured
> otherwise on the target machine". This is that measurement, and it is
> comfortably under. The number to carry forward is 780 MB at 1080p —
> **on macOS arm64 only**, per D-071. Windows is unmeasured.

### 10c. The join adds one AAC frame to the container, not one per segment

Six segments joined with the concat demuxer and `-c copy`:

```
$ ffprobe -v error -show_entries stream=codec_type,duration,nb_frames,start_time \
    -show_entries format=duration -of default=nw=1 /tmp/film.mp4
video  start_time=0.021000  duration=18.033333  nb_frames=541
audio  start_time=0.000000  duration=18.054667
format duration=18.054667
```

The video stream is exactly `541 / 30 = 18.033333` — frame-perfect, and equal
to the sum of the six segments' asserted frame counts. The container reports
the audio track's `18.054667`, which is **1024 samples longer**: exactly one
AAC frame of encoder priming, appearing once for the whole film rather than
once per join. Two- and four-segment joins showed the same single offset.

Consequences, both recorded as **D-078**:

- the frame-exactness gate is asserted against the **video stream**, and
  against the video stream's declared `nb_frames`, which for MP4 comes from the
  sample table the stream copy just wrote;
- the container's duration is still checked, with one frame *plus one AAC
  frame* of tolerance — loose enough for the priming, far tighter than the
  seconds a dropped scene would move it by.

Unmeasured, and worth knowing: whether the offset stays at one AAC frame at
n=500. It did not accumulate across 2, 4 and 6 segments. If it ever does, the
container check fails first and says so, which is the right place for that
surprise to surface.

### 10d. The determinism claim, checked rather than asserted

```bash
shasum -a 256 /tmp/bench-1.mp4 /tmp/bench-2.mp4 /tmp/bench-4.mp4 /tmp/bench-8.mp4
# 81e991b592cbba84110dd9a590567ecda3065d54822d44c0db1ae6e5e1f56fdf  (all four)
```

One worker, two, four and eight produce **byte-identical films**, and so does a
run that reuses every cached artifact. That is D-077, and it is gate 3 of
`scripts/m2-gates.sh` rather than a claim in a comment — motion is seeded
before the pool starts, each worker writes its own content-addressed path, and
results are collected by input index.

---

## 11. A hundred scenes, measured 2026-08-30

Everything in §10 was measured on fixtures. This is the same machine running a
project the size a person actually makes, because §10's own lesson — and the
race and the 3x regression that fixture-sized runs hid before it — is that four
scenes prove very little.

**The project.** 100 photographs of seven different sizes between 1600x900 and
3520x1620, each paired with its own recorded narration of 2.0 to 4.8 seconds.
Supplied recordings rather than speech, so this measures the render pool and not
somebody else's network. Import and validation are included because at this size
they stop being free.

| | |
|---|---|
| `still new` with 200 files | **0.06 s** |
| `still validate` (probes all 200) | **5.7 s** |
| cold render, no captions, default `--jobs` | **59.2 s** at 730% CPU |
| the film | 10 200 frames, **340.021 s** against 340.000 s expected |
| output | 97 MB |

The duration lands **21 ms** over 340 s, which is inside one frame at 30fps —
the same accuracy §7 measured on a single scene, held across a hundred joins.

**With captions burned in** (`--subtitles punch`, a caption beside every
recording per D-106, lines from four words to twenty-one):

| | |
|---|---|
| `--jobs 1`, cold | **2 m 02.8 s** |
| `--jobs 8`, cold | **1 m 02.2 s** |
| both films | `1d4d16d8…3c94f` — **byte-identical** |

Two things worth keeping from that. The speedup from one worker to eight is
**1.97x**, not 8x, which is §10's flattening curve appearing again at a hundred
scenes rather than four. And **D-077 holds at this size**: gate 3 asserts
byte-identical films across `--jobs` on a four-scene fixture, and the property
survives a hundred captioned scenes, where the pool actually saturates and every
worker is rasterizing text.

**Caching, measured on the four-scene project in the same session:** a second
render of an unchanged project is **0.58 s** against 5.2 s cold. Editing one
narration re-speaks and re-renders **that scene only** — three reused, one not
(D-107). A pure reorder reuses **nothing**, and that is D-140.

## 12. What a worker costs at each size, measured 2026-09-03

The reason for this section is D-144: a Windows machine froze hard at 4K, and
the pool that started four workers had only ever been sized against 1080p.
§10b measured one resolution and inferred the aggregate; this measures four
resolutions and then measures the aggregate directly.

Same machine as every other number here — macOS arm64, 10 cores, 24 GB.

### 12a. Per worker, by output size

One scene, `still render-scene`, peak RSS of the FFmpeg child via
`/usr/bin/time -l`:

```bash
/usr/bin/time -l ./target/release/still render-scene \
  --image fixtures/generated/land.jpg --audio fixtures/generated/n.wav \
  --out /tmp/s.mp4 --resolution 4k
```

| output | prescale canvas | canvas pixels | peak RSS |
|---|---|---|---|
| 1280x720 | 3840x2160 | 8.3 M | 369 MB |
| 1920x1080 | 5760x3240 | 18.7 M | **768 MB** |
| 2560x1440 | 7680x4320 | 33.2 M | 1219 MB |
| 3840x2160 | 11520x6480 | 74.6 M | **2630 MB** |

Two things to take from it. The 768 MB at 1080p is §10b's 780 MB by a different
method, so the method agrees with the existing measurement — which is the reason
to trust the other three rows. And the cost tracks the **prescale canvas**
(D-032), not the output frame: 4K's canvas is 4x 1080p's, and 4K's memory is
3.4x 1080p's.

A least-squares fit over the four is `107 MB + 33.8 bytes/pixel`. What is in
the code is `128 MB + 36 bytes/pixel`, which is above every measurement by 4-16%
— deliberately, per D-144: over-estimating costs a worker, under-estimating
costs the machine.

### 12b. Aggregate, and the 4K speedup curve

Eight scenes, one still each, cold every time (`rm -rf .spoonstill/segments`).
Aggregate is the peak sum of all live `ffmpeg` RSS, sampled at 5 Hz — which is
the direct measurement §10b could not make:

```bash
( while :; do ps -Ao rss,comm | awk '/ffmpeg/{s+=$1} END{if(s>0) print s/1024}'; \
  sleep 0.2; done ) > rss.txt &
```

**4K (3840x2160):**

| `--jobs` | wall clock | speedup | peak aggregate |
|---|---|---|---|
| 1 | 31.7 s | 1.00x | 2.6 GB |
| 2 | 22.7 s | 1.39x | 4.8 GB |
| 3 | 20.2 s | 1.57x | 6.0 GB |
| 4 | 18.7 s | 1.70x | **6.6 GB** |

The same eight scenes at 1080p, `--jobs 4`: **2.9 GB** — which reproduces
D-076's inferred ~3.1 GB, again by a different method.

**The trade D-144 acts on:** at 4K, going from two workers to four buys 22% of
wall clock for 1.8 GB. On a 24 GB machine that is free. On 8 GB it is the
machine, and the failure is not a slow render — it is a freeze that survives
until the power button.

### 12c. The encoder is 14% of a 4K scene, so a GPU would not have helped

D-036 recorded "filter-bound, not encoder-bound" at 1080p. Re-checked at 4K on
the shipped filter chain, 174 frames:

| | wall |
|---|---|
| filter only, `-f null -` | 3.59 s |
| filter + `libx264 -preset medium` (shipped) | 4.19 s |
| filter + `libx264 -preset ultrafast` | 3.64 s |

Making the encoder nearly free saves **0.55 s of 4.19 s**. So the ceiling on
*any* encoder change — NVENC, VideoToolbox, QSV — is about **1.17x**, and it
reduces memory by **zero**, because the memory is the 11520x6480 prescale canvas
inside the CPU filter graph. D-036 holds at 4K and holds harder than at 1080p.

Recorded because "the machine has a GPU and nothing uses it" is the obvious
first suspicion when a 4K render fails, and it is the wrong one.

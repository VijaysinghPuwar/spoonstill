<div align="center">

<img src="assets/brand/icon.svg" width="96" alt="spoonstill">

# spoonstill

**Turn a folder of photos and narration into one finished MP4.**

Ken Burns motion on every still, cut on the narration's own boundaries.
Drop in your files, press Render, walk away.

[![ci](https://github.com/VijaysinghPuwar/spoonstill/actions/workflows/ci.yml/badge.svg)](https://github.com/VijaysinghPuwar/spoonstill/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/VijaysinghPuwar/spoonstill?include_prereleases&label=download)](https://github.com/VijaysinghPuwar/spoonstill/releases/latest)
[![platforms](https://img.shields.io/badge/macOS-%C2%B7%20Windows-black)](#download)

</div>

---

## What it is

You have 40 photos and something to say over each one. spoonstill pairs them up,
gives each still a slow zoom or pan, makes each scene exactly as long as its
narration, and joins the lot into one MP4.

It is **a batch renderer, not a video editor**. There is no timeline and no
scrubber, on purpose. The unit of work is a folder, and the design point is
500 scenes — not five.

Narration can be a recording you made, or a line of text spoken by a neural
voice, or nothing at all (the still just holds). You can mix all three in one
film.

---

## Download

Grab the build for your machine from the
**[latest release](https://github.com/VijaysinghPuwar/spoonstill/releases/latest)**:

| Your machine | Desktop app | Command line |
|---|---|---|
| **macOS** — Apple Silicon *or* Intel | `spoonstill-macOS.dmg` | `still-macOS-AppleSilicon.tar.gz`<br>`still-macOS-Intel.tar.gz` |
| **Windows 10/11, 64-bit** | `spoonstill-Windows-Installer.exe` | `still-Windows.zip` |

The Mac app is one universal build, so you do not have to know which processor
you have. The one-line installer below works that out for the command line too.

### Or install with one line

**macOS**

```bash
curl -fsSL https://raw.githubusercontent.com/VijaysinghPuwar/spoonstill/master/scripts/install.sh | bash
```

**Windows** (PowerShell)

```powershell
irm https://raw.githubusercontent.com/VijaysinghPuwar/spoonstill/master/scripts/install.ps1 | iex
```

Either script installs the `still` command, then checks for FFmpeg and installs
it through Homebrew or winget if it is missing. Nothing runs as administrator
and nothing is written outside your own user folder.

The desktop app can install the voice service for you from **Settings > Voice
service > Install it**; on the command line that is `still voices --install`.

---

## One thing to install first

spoonstill does the thinking; **FFmpeg** does the pixels. It is not bundled yet
(see [D-062](decisions.md#d-062--licensing-boundary) — the shipped binary needs
its own LGPL build first), so it has to be on your machine.

**You do not have to do this by hand.** Open the window, go to
**Settings**, and press **Install it for me** under Video engine — and under
Voice service too, if you want written lines read aloud. Both use the package
manager already on your machine, and both tell you when they are done.

Prefer a terminal? One command checks everything and offers to fetch what is
missing:

```bash
still doctor              # what is here, what is not
still doctor --install    # fetch whatever is missing
```

Or install them yourself, once:

```bash
brew install ffmpeg          # macOS
winget install Gyan.FFmpeg   # Windows

still voices --install       # a neural voice, or: pipx install edge-tts
```

Skip the voice if you are only using recordings you made yourself.

**A note on quality.** Edge TTS is Microsoft's free endpoint and it returns
24 kHz, mono, 48 kbps MP3 — the format is fixed in `edge-tts` itself, because
its word-timing arithmetic divides by exactly that bitrate. spoonstill does not
degrade it further (the working artifact is lossless PCM and the film's audio is
192 kbps AAC), but a free service's ceiling is a free service's ceiling. A
higher-fidelity provider is on the roadmap.

---

## Three minutes to your first film

```bash
# 1. Make a project out of whatever you have. Nothing gets renamed or moved.
still new ~/holiday ~/Pictures/trip/*.jpg ~/Voice\ Memos/*.m4a

# 2. See what it made of them, before anything is encoded.
still validate ~/holiday

# 3. Render.
still render ~/holiday --out ~/holiday.mp4
```

`still new` copies your photos in as `001`, `002`, … in natural order and pairs
each one with a recording — **by matching name first, then by position**. It
never touches your originals and never overwrites anything.

Prefer the window? Launch **spoonstill** from Applications or the Start menu,
drop the folder on it, and press Render. Every button there is one of the
commands above.

### Speaking text instead of recording it

Put a `.txt` file next to a photo — `002.jpg` and `002.txt` — and its contents
become the narration for that scene.

```bash
still voices en-GB                                  # what's available
still render ~/holiday --out ~/holiday.mp4 --voice en-GB-RyanNeural
```

Spoken lines and your own recordings are levelled to the same loudness, so a
phone recording and a synthetic voice sit together in one film without you
riding a fader.

### Subtitles

Off unless you ask — text burned into the picture cannot be taken out again
without re-rendering.

```bash
still subtitles                                     # six looks, and what each is for
still render ~/holiday --out ~/holiday.mp4 --subtitles boxed
still render ~/holiday --out ~/holiday.mp4 --subtitle-position top
still render ~/holiday --out ~/holiday.mp4 --no-subtitles
```

If your pictures already have words drawn into them — captions in the artwork,
a logo along the bottom — put the subtitles at the **top**. A theme with a
plate behind it (`boxed`, `band`, `card`) also covers that lettering, where
`classic` and `minimal` let it show through between the lines.

A scene gets a caption when it has words. That is the `.txt` it speaks — or,
**if the scene has a recording, a `.txt` next to it is the caption for that
recording**, so your own voiceover can be subtitled without typing anything
twice.

To make it the project's own setting, in `project.yaml`:

```yaml
subtitles:
  enabled: true
  theme: boxed      # classic · boxed · band · card · punch · minimal
  position: bottom  # or top
```

In the window there is a **Subtitles** screen: pick a look and see it drawn on
a real frame before you commit a whole film to it, or pick *No subtitles*.

The text is drawn by spoonstill itself, not by FFmpeg — so this works on the
plain `ffmpeg` you already installed, with no extra libraries and nothing else
to download.

### When it goes wrong

```bash
still doctor                    # is everything this needs actually installed?
still validate ~/holiday        # every problem in the folder, all at once
still diagnostics export --project ~/holiday --out ~/bundle.txt
```

Start with `doctor`. A folder of good photographs opening as **no scenes**, or a
Voice screen with no voices on it, is almost always one missing program rather
than anything wrong with your files — and `doctor --install` fetches it.

`validate` reports *everything* it can find in one pass rather than stopping at
the first bad row. The diagnostics bundle is one text file, with credentials
redacted, that you can attach to an issue.

**In the window**, you never need any of this: wherever spoonstill notices that
something it needs is missing — the Voice screen, the Render screen, Settings —
it says so in a sentence and puts an **Install it for me** button next to it.
Press it and the screen you are on repairs itself.

---

## "Apple could not verify spoonstill is free of malware"

These builds are **not code-signed yet** — signing certificates are M5 work.
The operating system is telling you it cannot identify the publisher, which is
true, and not that anything is wrong with the file.

**The one-line installer above avoids this entirely.** It verifies the
checksum, installs the app, and clears the quarantine attribute itself, so the
first launch is just the app opening. If you downloaded the `.dmg` through a
browser instead:

- **macOS** — press **Done** on that dialog, never *Move to Trash*. Then either
  **System Settings > Privacy & Security**, scroll down, **Open Anyway** — or,
  once:
  ```bash
  xattr -dr com.apple.quarantine /Applications/spoonstill.app
  ```
  Right-click > Open is the advice everywhere on the internet and **Apple
  removed it in macOS 15**; on 15 or later it does nothing.
- **Windows** — on the SmartScreen dialog, click **More info** > **Run anyway**.

If that trade is not one you want to make, [build from source](#build-from-source)
instead: it is two commands.

---

## Build from source

You need [Rust](https://rustup.rs) (1.94+, pinned by `rust-toolchain.toml`) and
FFmpeg on `PATH`.

```bash
git clone https://github.com/VijaysinghPuwar/spoonstill.git
cd spoonstill

cargo build --release -p spoonstill-cli     # -> target/release/still
cargo run   --release -p spoonstill-desktop # the window
```

Working on it:

```bash
make help       # every entry point
make test       # the whole workspace
make lint       # clippy, warnings denied, plus a format check
make fixtures   # synthesize the test media
make gates      # every milestone's exit gates — the real state of the build
```

`make gates` is the honest answer to "does this work?". It runs 25 checks
across M0, M1 and M2 and prints pass/fail for each.

---

## How a project is laid out

A project is just a folder. Everything in it is optional, including
`project.yaml` — a folder of images is already a valid project.

```
holiday/
├── project.yaml     ← settings. Never scenes. spoonstill never writes to it.
├── scenes.csv       ← optional. When present, it is the complete scene list.
├── img/
│   ├── 001.jpg
│   └── 002.jpg
└── audio/
    ├── 001.m4a      ← paired to 001.jpg by name
    └── 002.txt      ← a line to be spoken over 002.jpg
```

```yaml
# project.yaml
output: film.mp4
aspect: 16:9        # or 9:16, or 1:1
short_edge: 1080
fps: 30
defaults:
  duration: 4.0     # how long an unpaired still holds
subtitles:
  enabled: false    # burned into the picture when true
  theme: classic
```

Full rules: [D-056](decisions.md) for what a project folder is and which file
wins, [D-080](decisions.md) for how media gets in, [D-106](decisions.md) for
subtitles.

---

## Where things stand

| | |
|---|---|
| **M0** scaffolding, architecture boundary | complete — 8/8 gates |
| **M1** one scene, end to end | complete — 8/8 gates |
| **M2** whole projects: import, validation, speech, parallel render | complete — 9/9 gates |
| **M3** state database, resumable queue | next |
| **M4** the Tauri window | shell exists, ahead of schedule |
| **M5** signing, notarization, bundled FFmpeg, auto-update | not started |

So: it renders real films today, and the releases here are **unsigned preview
builds** that use the FFmpeg already on your machine. Treat them accordingly.

---

## The documents

This project keeps its reasoning in the repository rather than in anyone's head.

| File | What it is |
|---|---|
| [`decisions.md`](decisions.md) | **Single source of truth.** Every design decision, numbered, with the reasoning that produced it. |
| [`plan.md`](plan.md) | Milestones M0–M5, each with runnable exit gates. |
| [`ffmpeg-findings.md`](ffmpeg-findings.md) | Benchmarks measured on real hardware, with reproduction commands. |
| [`CLAUDE.md`](CLAUDE.md) | Orientation for anyone — or anything — picking the work up cold. |

If you are about to propose a design, read `decisions.md` first. Most of it is
already settled, and the reasoning is written down.

---

## Licence

Not yet chosen — see [D-062](decisions.md) for the licence boundaries that
already constrain it. Until then, all rights reserved by the author.
The reference checkouts under `plan/` are **not** part of this repository.

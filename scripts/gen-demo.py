#!/usr/bin/env python3
"""Generate the stills and scripts behind README.md's demo GIF.

The GIF at the top of the README has to be a real render — the same filter
chain, the same subtitle rasterizer, the same join every operator gets. What it
must *not* be is the author's own photographs, which are their work rather than
this repository's. So the demo has its own stills, described here and generated
rather than committed as pixels, exactly like the logo (D-079).

    make demo

Deliberately abstract. A demo made of stock photography would be showing off
someone else's picture; these show the thing this program does to a picture —
the slow move across it, the cut on the narration's own length, the caption
burned into the frame.

Stdlib plus Pillow, which is the one thing here that is not already a
dependency of the build.
"""

import math
import subprocess
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
except ImportError:
    sys.exit("This needs Pillow: python3 -m pip install --user pillow")

# 2560x1440, not 1920x1080: the renderer prescales down (D-030), and a still
# that is already the output size has nothing to move *within*. A demo that
# fed it an exactly-sized image would be demonstrating the one case Ken Burns
# cannot show.
W, H = 2560, 1440

# The window's ink series (apps/desktop/ui/styles.css), so the film and the
# application it came out of look like one product. Each scene is one ink over
# one ground, because two-colour is a composition and five is a swatch.
SCENES = [
    {
        "ground": (14, 16, 20),
        "ink": (232, 168, 92),
        "line": "A folder of photographs, and something to say over each one.",
    },
    {
        "ground": (18, 20, 26),
        "ink": (110, 168, 214),
        "line": "Every still gets a slow move. No two are quite the same.",
    },
    {
        "ground": (16, 14, 20),
        "ink": (196, 132, 188),
        "line": "Each scene lasts exactly as long as the words spoken over it.",
    },
    {
        "ground": (14, 18, 18),
        "ink": (126, 194, 158),
        "line": "Then it joins them, and hands you one finished film.",
    },
]


def still(index: int, ground, ink) -> Image.Image:
    """One frame: a graded ground, a ring off centre, and a few crisp rules.

    Drawn at 2x and reduced, which is the cheapest antialiasing there is and
    the reason the curves have no stair-stepping once this is a GIF. The blur
    is deliberately small — an earlier version blurred hard enough to erase its
    own composition, which reads as an empty frame rather than a calm one.
    """
    scale = 2
    w, h = W * scale, H * scale
    img = Image.new("RGB", (w, h), ground)
    px = img.load()

    # A diagonal grade from the ground into a tint of the ink. Computed on a
    # small image and scaled up: per-pixel Python over 5120x2880 is slow enough
    # to notice, and a gradient has no detail to lose.
    small = Image.new("RGB", (64, 36))
    sp = small.load()
    for y in range(36):
        for x in range(64):
            t = (x / 63 * 0.65) + (y / 35 * 0.35)
            t = t ** 1.3
            sp[x, y] = tuple(
                int(g + (i * 0.55 - g) * t) for g, i in zip(ground, ink)
            )
    img = small.resize((w, h), Image.BICUBIC)
    draw = ImageDraw.Draw(img, "RGBA")

    # One ring, placed off centre so the pan has somewhere to travel to. Two
    # weights of it: a broad soft halo, then a bright edge that survives the
    # blur and gives the motion something to be measured against.
    cx = w * (0.30 + 0.15 * index)
    cy = h * (0.58 - 0.12 * (index % 2))
    r = h * 0.30
    for ring in range(90):
        rr = r + ring * 4.5
        draw.ellipse(
            [cx - rr, cy - rr, cx + rr, cy + rr],
            outline=(*ink, max(0, 70 - ring)),
            width=5,
        )
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], outline=(*ink, 190), width=6)

    # A few crisp rules across the frame. Sparse and bright rather than many
    # and faint: the faint version was invisible after the blur.
    angle = math.radians(20 + index * 14)
    dy = math.sin(angle)
    for step in range(6):
        x0 = w * (0.06 + step * 0.17)
        draw.line(
            [(x0, -h), (x0 + dy * h * 2.4, h * 2)],
            fill=(*ink, 40 if step % 2 else 68),
            width=scale * 2,
        )

    img = img.filter(ImageFilter.GaussianBlur(radius=scale * 0.6))
    return img.resize((W, H), Image.LANCZOS)


def main() -> None:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "fixtures/demo")
    out.mkdir(parents=True, exist_ok=True)

    for i, scene in enumerate(SCENES, start=1):
        # D-050's convention: a scene is every file sharing a numeric stem, and
        # the .txt beside the photograph is the line to be spoken (D-020) and
        # therefore also the caption (D-106).
        name = f"{i:03d}"
        still(i - 1, scene["ground"], scene["ink"]).save(
            out / f"{name}.jpg", quality=92, subsampling=0
        )
        (out / f"{name}.txt").write_text(scene["line"] + "\n", encoding="utf-8")
        print(f"  {name}.jpg  {name}.txt  {scene['line'][:44]}…")

    print(f"\n{len(SCENES)} scenes in {out}")
    print("Render it:  still render", out, "--out demo.mp4 --subtitles boxed")


if __name__ == "__main__":
    main()

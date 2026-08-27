#!/usr/bin/env python3
"""Generate every spoonstill brand asset from one description of the mark.

The mark is three stacked stills (D-079). Its geometry was measured off the
master render rather than eyeballed, and those measurements live here, once, so
that the SVGs and the twenty-odd raster sizes cannot drift apart. Regenerate
with `make brand`; the outputs are committed, so a build never needs Python.

Stdlib only, deliberately: no Pillow, no ImageMagick, no librsvg. The artwork is
axis-aligned rectangles, so coverage is computed analytically — which also means
the 16 px icon is exactly antialiased rather than downsampled from a big one.
"""

import os
import struct
import subprocess
import sys
import tempfile
import zlib

# --- the mark, measured ------------------------------------------------------
# Tight bounding box of the artwork. Every number below is in this space.
MARK_W, MARK_H = 1024, 1126

INK = (0xEF, 0xE9, 0xE0)  # one ink, three opacities — never a second colour
S = 51                    # stroke width
FW, FH = 721, 819         # one still's outer size

# Where each still sits, measured off the master rather than derived from a
# uniform step: the real steps are (151,154) then (152,153). That one-pixel
# irregularity is under 0.1% and invisible, but forcing a uniform step would
# move the artwork off the delivered original, so the measurements win.
POS = [(0, 0), (151, 154), (303, 307)]

# Depth is carried by opacity alone. These are the measured values: the two rear
# strokes are translucent, so where they cross they composite to a brighter
# value (0.72 over 0.862 -> 0.961), and that accidental highlight is part of the
# mark. Compositing order therefore matters, and each still's black plate covers
# only its cavity — never its own stroke.
STILLS = [(*POS[0], 0.72), (*POS[1], 0.862), (*POS[2], 1.0)]

# The image inside the front still: flush to the cavity's left and bottom, inset
# by two stroke widths from its top and right.
FRONT_X, FRONT_Y = POS[2]
PANEL = (FRONT_X + S, FRONT_Y + S + 2 * S, FW - 2 * S - 2 * S, FH - 2 * S - 2 * S)


def ops():
    """The mark as an ordered list of (x, y, w, h, rgb, alpha) rectangles."""
    out = []
    for x, y, a in STILLS:
        # cavity plate: opaque black, so a still occludes the ones behind it
        out.append((x + S, y + S, FW - 2 * S, FH - 2 * S, (0, 0, 0), 1.0))
        # stroke, as four non-overlapping bars so a translucent stroke does not
        # double-composite at its own corners
        out.append((x, y, FW, S, INK, a))
        out.append((x, y + FH - S, FW, S, INK, a))
        out.append((x, y + S, S, FH - 2 * S, INK, a))
        out.append((x + FW - S, y + S, S, FH - 2 * S, INK, a))
    out.append((*PANEL, INK, 1.0))
    return out


# --- raster ------------------------------------------------------------------

ICON_FILL = 0.78  # the mark's height as a fraction of a square icon's edge


def render(size, fill=ICON_FILL):
    """Render a square opaque-black icon of `size` px. Returns RGB bytes."""
    k = size * fill / MARK_H
    ox = (size - MARK_W * k) / 2.0
    oy = (size - MARK_H * k) / 2.0

    buf = [0.0] * (size * size * 3)
    for mx, my, mw, mh, rgb, alpha in ops():
        x0, y0 = ox + mx * k, oy + my * k
        x1, y1 = x0 + mw * k, y0 + mh * k
        for py in range(max(0, int(y0)), min(size, int(y1) + 1)):
            cov_y = min(py + 1, y1) - max(py, y0)
            if cov_y <= 0:
                continue
            for px in range(max(0, int(x0)), min(size, int(x1) + 1)):
                cov_x = min(px + 1, x1) - max(px, x0)
                if cov_x <= 0:
                    continue
                a = alpha * cov_x * cov_y  # exact area coverage, not a sample
                i = (py * size + px) * 3
                for c in range(3):
                    buf[i + c] += (rgb[c] - buf[i + c]) * a
    return bytes(min(255, int(v + 0.5)) for v in buf)


def png(size, rgb):
    """A PNG of one square, always **RGBA**.

    The mark is opaque everywhere — it is defined on black — so every alpha
    byte is 0xFF and the file is a third larger than it needs to be. It is
    written this way regardless because Tauri's icon pipeline rejects a
    truecolor PNG outright ("icon ... is not RGBA"), and a build that fails on
    the icon is a worse trade than 1 KB.
    """
    rows = []
    for y in range(size):
        line = rgb[y * size * 3:(y + 1) * size * 3]
        rows.append(b"\x00" + b"".join(
            line[x * 3:x * 3 + 3] + b"\xff" for x in range(size)
        ))
    raw = b"".join(rows)

    def chunk(tag, data):
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9))
            + chunk(b"IEND", b""))


def ico(pngs):
    """ICO with PNG-compressed entries (Vista+), which is what Tauri wants."""
    head = struct.pack("<HHH", 0, 1, len(pngs))
    offset = len(head) + 16 * len(pngs)
    entries, blobs = b"", b""
    for size, data in pngs:
        entries += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32,
                               len(data), offset)
        blobs += data
        offset += len(data)
    return head + entries + blobs


# --- svg ---------------------------------------------------------------------

HEADER = """<!-- spoonstill mark (D-079). GENERATED by scripts/gen-brand.py — edit the
     geometry there, not here, then run `make brand`.

     Defined ON BLACK and not background-agnostic: each still is an opaque black
     plate plus a translucent stroke, which is what makes the rear stills read
     as depth and what makes their overlap brighten. On any surface that is not
     near-black the plates show. That is the design (D-079). -->
"""


def svg_body(indent=""):
    lines = []
    for n, (x, y, a) in enumerate(STILLS):
        lines.append(f'{indent}<!-- still {n + 1} of 3, ink at {a:g} -->')
        lines.append(f'{indent}<rect x="{x + S}" y="{y + S}" '
                     f'width="{FW - 2 * S}" height="{FH - 2 * S}" fill="#000"/>')
        op = "" if a == 1.0 else f' stroke-opacity="{a:g}"'
        lines.append(f'{indent}<rect x="{x + S / 2}" y="{y + S / 2}" '
                     f'width="{FW - S}" height="{FH - S}" fill="none" '
                     f'stroke="#{"%02X%02X%02X" % INK}" stroke-width="{S}"{op}/>')
    lines.append(f'{indent}<!-- the image itself, flush bottom-left in its mat -->')
    lines.append(f'{indent}<rect x="{PANEL[0]}" y="{PANEL[1]}" width="{PANEL[2]}" '
                 f'height="{PANEL[3]}" fill="#{"%02X%02X%02X" % INK}"/>')
    return "\n".join(lines)


def svg_mark():
    return (f'{HEADER}<svg xmlns="http://www.w3.org/2000/svg" '
            f'viewBox="0 0 {MARK_W} {MARK_H}" width="{MARK_W}" height="{MARK_H}" '
            f'role="img" aria-label="spoonstill">\n'
            f'  <title>spoonstill</title>\n{svg_body("  ")}\n</svg>\n')


def svg_icon(size=1024):
    k = size * ICON_FILL / MARK_H
    ox = round((size - MARK_W * k) / 2.0, 2)
    oy = round((size - MARK_H * k) / 2.0, 2)
    return (f'{HEADER}<svg xmlns="http://www.w3.org/2000/svg" '
            f'viewBox="0 0 {size} {size}" width="{size}" height="{size}" '
            f'role="img" aria-label="spoonstill">\n'
            f'  <title>spoonstill</title>\n'
            f'  <rect width="{size}" height="{size}" fill="#000"/>\n'
            f'  <g transform="translate({ox} {oy}) scale({k:.6f})">\n'
            f'{svg_body("    ")}\n  </g>\n</svg>\n')


# --- outputs -----------------------------------------------------------------

def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    brand = os.path.join(root, "assets", "brand")
    icons = os.path.join(root, "apps", "desktop", "icons")
    os.makedirs(brand, exist_ok=True)
    os.makedirs(icons, exist_ok=True)

    def write(path, data):
        mode = "w" if isinstance(data, str) else "wb"
        with open(path, mode) as f:
            f.write(data)
        print(f"  {os.path.relpath(path, root):<44} {os.path.getsize(path):>7} B")

    write(os.path.join(brand, "mark.svg"), svg_mark())
    write(os.path.join(brand, "icon.svg"), svg_icon())

    cache = {}

    def at(size):
        if size not in cache:
            cache[size] = png(size, render(size))
        return cache[size]

    # Tauri's default bundle icon set, plus the plain icon.png already in tree.
    for name, size in [("32x32.png", 32), ("128x128.png", 128),
                       ("128x128@2x.png", 256), ("icon.png", 512)]:
        write(os.path.join(icons, name), at(size))
    write(os.path.join(brand, "icon-1024.png"), at(1024))

    write(os.path.join(icons, "icon.ico"),
          ico([(s, at(s)) for s in (16, 32, 48, 64, 128, 256)]))

    # .icns needs iconutil, which is macOS-only. Skip it elsewhere rather than
    # fail: the committed file stays valid until a mac regenerates it.
    if sys.platform == "darwin":
        with tempfile.TemporaryDirectory() as tmp:
            iconset = os.path.join(tmp, "icon.iconset")
            os.makedirs(iconset)
            for base in (16, 32, 128, 256, 512):
                for suffix, size in ((f"{base}x{base}", base),
                                     (f"{base}x{base}@2x", base * 2)):
                    with open(os.path.join(iconset, f"icon_{suffix}.png"), "wb") as f:
                        f.write(at(size))
            out = os.path.join(icons, "icon.icns")
            subprocess.run(["iconutil", "-c", "icns", iconset, "-o", out], check=True)
            print(f"  {os.path.relpath(out, root):<44} {os.path.getsize(out):>7} B")
    else:
        print("  icon.icns                                    skipped (needs macOS)")

    # Tauri ships only what is under `frontendDist`, so the shell needs its own
    # copy. Regenerate it here rather than asking anyone to remember. It gets
    # the square black-tiled form: the mark needs its black, and a tile renders
    # correctly on a panel of any colour instead of only on a black one.
    write(os.path.join(root, "apps", "desktop", "ui", "icon.svg"), svg_icon())


if __name__ == "__main__":
    main()

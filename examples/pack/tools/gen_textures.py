#!/usr/bin/env python3
"""Generate the example pack's PNG textures as real, decodable 16x16 PNGs.

Pillow is not assumed to be installed: this writes valid PNG bytes directly
using only the Python standard library (``zlib`` + ``struct``). Each texture is
a small, intentional pixel-art pattern so the example looks hand-made.

Run from the repo with::

    python3 examples/pack/tools/gen_textures.py

It (re)writes the PNG files under ``examples/pack/src/assets/minecraft/textures``.
The output is deterministic, so re-running it produces byte-identical files.
"""

import os
import struct
import zlib

SIZE = 16

# A small palette (RGBA tuples).
TRANSPARENT = (0, 0, 0, 0)
BLACK = (30, 30, 36, 255)
GRAY = (110, 110, 120, 255)
LIGHT = (200, 200, 210, 255)
WHITE = (245, 245, 250, 255)
BLADE = (210, 220, 235, 255)
BLADE_EDGE = (160, 170, 190, 255)
HILT = (120, 80, 40, 255)
GUARD = (190, 160, 70, 255)
GEM = (80, 200, 220, 255)
GEM_HI = (170, 240, 250, 255)
GEM_LO = (40, 140, 170, 255)
BRICK = (150, 70, 60, 255)
BRICK_DK = (110, 50, 45, 255)
MORTAR = (200, 195, 185, 255)
EMBER = (255, 170, 40, 255)
EMBER_HOT = (255, 230, 120, 255)
EMBER_DK = (180, 70, 20, 255)


def write_png(path, pixels, width=SIZE, height=SIZE):
    """Write ``pixels`` (rows of RGBA tuples) as an 8-bit RGBA PNG."""
    raw = bytearray()
    for row in pixels:
        raw.append(0)  # filter type 0 (None) for each scanline
        for (r, g, b, a) in row:
            raw += bytes((r, g, b, a))

    def chunk(tag, data):
        out = struct.pack(">I", len(data)) + tag + data
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return out + struct.pack(">I", crc)

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    idat = zlib.compress(bytes(raw), 9)

    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(sig)
        f.write(chunk(b"IHDR", ihdr))
        f.write(chunk(b"IDAT", idat))
        f.write(chunk(b"IEND", b""))


def grid(fn):
    """Build a 16x16 pixel grid from a function ``fn(x, y) -> rgba``."""
    return [[fn(x, y) for x in range(SIZE)] for y in range(SIZE)]


def diamond_sword(x, y):
    # Diagonal blade from bottom-left to top-right, with a hilt.
    if x - y == 0 and 2 <= x <= 11:
        return BLADE
    if x - y == 1 and 2 <= x <= 12:
        return BLADE_EDGE
    if x - y == -1 and 1 <= x <= 10:
        return WHITE
    # Cross-guard near bottom-left.
    if 11 <= x + y <= 13 and 9 <= y <= 13 and x <= 6:
        return GUARD
    # Hilt grip.
    if 13 <= y <= 15 and 1 <= x <= 3:
        return HILT
    return TRANSPARENT


def gem(x, y):
    cx, cy = 7.5, 7.5
    d = abs(x - cx) + abs(y - cy)  # diamond distance
    if d <= 5.5:
        if x < cx and y < cy:
            return GEM_HI
        if x > cx and y > cy:
            return GEM_LO
        return GEM
    if d <= 6.5:
        return GEM_LO
    return TRANSPARENT


def bricks(x, y):
    # Two staggered rows of bricks with mortar lines.
    row = y // 4
    offset = 0 if row % 2 == 0 else 4
    bx = (x + offset) % 8
    if y % 4 == 0:
        return MORTAR  # horizontal mortar
    if bx == 0:
        return MORTAR  # vertical mortar
    return BRICK if (x + y) % 3 else BRICK_DK


def ember_frame(level):
    """A flame texture; ``level`` 0..3 controls how high the flame reaches."""
    top = 12 - level * 3

    def fn(x, y):
        cx = 7.5
        if y < top:
            return TRANSPARENT
        width = (y - top) / 2.0 + 1
        if abs(x - cx) <= width - 2:
            return EMBER_HOT
        if abs(x - cx) <= width:
            return EMBER
        if abs(x - cx) <= width + 1 and y > top + 1:
            return EMBER_DK
        return TRANSPARENT

    return grid(fn)


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    tex = os.path.normpath(os.path.join(here, "..", "src", "assets", "minecraft", "textures"))

    write_png(os.path.join(tex, "item", "diamond_sword.png"), grid(diamond_sword))
    write_png(os.path.join(tex, "custom", "gem.png"), grid(gem))
    write_png(os.path.join(tex, "block", "rpp_bricks.png"), grid(bricks))

    # An animated texture: 4 stacked frames (16x64) read top-to-bottom by the
    # accompanying .mcmeta. Build it as one tall image.
    frames = [ember_frame(lvl) for lvl in (0, 1, 2, 3)]
    tall = []
    for frame in frames:
        tall.extend(frame)
    write_png(os.path.join(tex, "block", "ember.png"), tall, SIZE, SIZE * 4)
    print("textures written under", tex)



if __name__ == "__main__":
    main()

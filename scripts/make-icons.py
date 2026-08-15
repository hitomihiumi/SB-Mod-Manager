#!/usr/bin/env python3
"""Generate the application icon set.

Kept in-repo and dependency-free so the icons can be regenerated without
pulling in an image toolchain. Draws at 4x and box-filters down, which is
enough anti-aliasing for a mark this simple.
"""

import struct
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"

BG_TOP = (23, 26, 33)
BG_BOTTOM = (13, 15, 19)
ACCENT_TOP = (236, 72, 153)
ACCENT_BOTTOM = (99, 102, 241)
EDGE = (244, 244, 245)

SS = 4  # supersampling factor


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def rounded_rect(x, y, w, h, r):
    """Signed coverage test for a rounded rectangle."""
    def inside(px, py):
        cx = min(max(px, x + r), x + w - r)
        cy = min(max(py, y + r), y + h - r)
        dx, dy = px - cx, py - cy
        if px < x or px > x + w or py < y or py > y + h:
            return False
        if (x + r <= px <= x + w - r) or (y + r <= py <= y + h - r):
            return True
        return dx * dx + dy * dy <= r * r
    return inside


def blade(size):
    """A stylised blade: a slanted stroke tapering to a point at the top."""
    def inside(px, py):
        u, v = px / size, py / size
        if not (0.15 < v < 0.85):
            return False
        centre = 0.5 + (v - 0.5) * 0.34
        # Taper across the upper third so the tip reads as an edge, not a bar.
        taper = min(1.0, (v - 0.15) / 0.34)
        return abs(u - centre) < 0.108 * taper
    return inside


def render(size):
    ss = size * SS
    panel = rounded_rect(0, 0, ss, ss, ss * 0.22)
    mark = blade(ss)

    # Supersampled RGBA buffer.
    hi = bytearray(ss * ss * 4)
    for py in range(ss):
        t = py / max(ss - 1, 1)
        bg = lerp(BG_TOP, BG_BOTTOM, t)
        fg = lerp(ACCENT_TOP, ACCENT_BOTTOM, t)
        for px in range(ss):
            i = (py * ss + px) * 4
            if not panel(px, py):
                continue
            if mark(px, py):
                # Brighten the leading edge so the shape reads at 32px.
                edge = mark(px + ss * 0.02, py) and not mark(px + ss * 0.05, py)
                colour = EDGE if edge else fg
            else:
                colour = bg
            hi[i], hi[i + 1], hi[i + 2], hi[i + 3] = colour[0], colour[1], colour[2], 255

    # Box filter down to the target size.
    out = bytearray(size * size * 4)
    for y in range(size):
        for x in range(size):
            r = g = b = a = 0
            for dy in range(SS):
                for dx in range(SS):
                    i = ((y * SS + dy) * ss + (x * SS + dx)) * 4
                    r += hi[i]
                    g += hi[i + 1]
                    b += hi[i + 2]
                    a += hi[i + 3]
            n = SS * SS
            o = (y * size + x) * 4
            out[o] = r // n
            out[o + 1] = g // n
            out[o + 2] = b // n
            out[o + 3] = a // n
    return bytes(out)


def write_png(path, size, rgba):
    raw = b"".join(
        b"\x00" + rgba[y * size * 4:(y + 1) * size * 4] for y in range(size)
    )

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    path.write_bytes(png)
    return png


def write_ico(path, entries):
    """ICO with embedded PNG frames."""
    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    dir_entries, blobs = b"", b""
    for size, png in entries:
        dim = 0 if size >= 256 else size
        dir_entries += struct.pack(
            "<BBBBHHII", dim, dim, 0, 0, 1, 32, len(png), offset
        )
        blobs += png
        offset += len(png)
    path.write_bytes(header + dir_entries + blobs)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    cache = {}

    for size, name in [
        (32, "32x32.png"),
        (128, "128x128.png"),
        (256, "128x128@2x.png"),
        (512, "icon.png"),
    ]:
        rgba = cache.setdefault(size, render(size))
        write_png(OUT / name, size, rgba)
        print(f"wrote {name}")

    ico = []
    for size in (16, 32, 48, 64, 256):
        rgba = cache.setdefault(size, render(size))
        ico.append((size, write_png(OUT / f".tmp{size}.png", size, rgba)))
    write_ico(OUT / "icon.ico", ico)
    for size in (16, 32, 48, 64, 256):
        (OUT / f".tmp{size}.png").unlink()
    print("wrote icon.ico")


if __name__ == "__main__":
    main()

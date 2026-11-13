#!/usr/bin/env python3
"""Generate the application icon set from docs/logo.png.

Needs Pillow (`pip install pillow`). Replace docs/logo.png with square art of
at least 512 px, run this, and commit the result.
"""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "docs" / "logo.png"
OUT = ROOT / "src-tauri" / "icons"

PNGS = {
    "icon.png": 512,
    "128x128@2x.png": 256,
    "128x128.png": 128,
    "32x32.png": 32,
}
ICO_SIZES = [(s, s) for s in (16, 24, 32, 48, 64, 128, 256)]


def main():
    art = Image.open(SOURCE).convert("RGBA")
    if art.width != art.height or art.width < 512:
        raise SystemExit(f"{SOURCE} must be square and at least 512 px")
    for name, size in PNGS.items():
        art.resize((size, size), Image.LANCZOS).save(OUT / name)
    art.save(OUT / "icon.ico", sizes=ICO_SIZES)


if __name__ == "__main__":
    main()

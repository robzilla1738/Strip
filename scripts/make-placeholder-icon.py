#!/usr/bin/env python3
"""Write a flat 1024 placeholder icon until APP_ICON_1024_PNG is a real file."""

import os
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / "app" / "Resources" / "Assets.xcassets" / "AppIcon.appiconset" / "icon_1024.png"


def chunk(tag: bytes, data: bytes) -> bytes:
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)


def write_png(path: Path) -> None:
    width = height = 1024
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        for x in range(width):
            dx = x - 512
            dy = y - 512
            inside = dx * dx + dy * dy <= 430 * 430
            bar = inside and 470 <= y <= 554 and abs(dx) <= 280
            if bar:
                raw.extend((244, 247, 252, 255))
            elif inside:
                raw.extend((28, 40, 56, 255))
            else:
                raw.extend((0, 0, 0, 0))
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)


def main() -> None:
    supplied = os.environ.get("APP_ICON_1024_PNG", "")
    if supplied and "{{" not in supplied and Path(supplied).is_file():
        DEST.parent.mkdir(parents=True, exist_ok=True)
        DEST.write_bytes(Path(supplied).read_bytes())
        return
    print("MISSING: {{APP_ICON_1024_PNG}}", flush=True)
    if not DEST.exists():
        write_png(DEST)


if __name__ == "__main__":
    main()

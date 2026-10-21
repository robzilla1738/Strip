#!/usr/bin/env bash
# Build the drag-install disk image. Dry-run prints the create-dmg command
# and does not write a disk image.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
strip_load_brand

name="$(strip_show APP_NAME "${APP_NAME:-}")"
app="${1:-$root/build/${name}.app}"
dmg="$root/build/${name}.dmg"
background="$root/app/Resources/dmg-background.png"
identity="$(strip_show DEVELOPER_ID_APPLICATION "${DEVELOPER_ID_APPLICATION:-}")"

if [[ ! -f "$background" && "${DRY_RUN:-}" != "1" ]]; then
  python3 - "$background" <<'PY'
import struct, sys, zlib
from pathlib import Path
path, width, height = Path(sys.argv[1]), 540, 380
pixel = bytes((18, 22, 28, 255))
raw = bytearray()
for _ in range(height):
    raw.append(0)
    raw.extend(pixel * width)
def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
png = b"\x89PNG\r\n\x1a\n"
png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
png += chunk(b"IEND", b"")
path.parent.mkdir(parents=True, exist_ok=True)
path.write_bytes(png)
PY
fi

echo "would run: create-dmg --volname \"$name\" --window-size 540 380 --icon-size 128 --icon \"$name.app\" 140 190 --app-drop-link 400 190 --background \"$background\" \"$dmg\" \"$app\""

if strip_is_unset "${DEVELOPER_ID_APPLICATION:-}"; then
  echo "MISSING: {{DEVELOPER_ID_APPLICATION}}"
  echo "SKIPPED (placeholder): dmg signature"
  echo "would run: codesign --force --timestamp --sign \"$identity\" \"$dmg\""
else
  echo "would run: codesign --force --timestamp --sign \"$identity\" \"$dmg\""
fi

if strip_dry_run; then
  exit 0
fi
if [[ ! -d "$app" ]]; then
  echo "SKIPPED (placeholder): dmg" >&2
  exit 0
fi
rm -f "$dmg"
create-dmg --volname "$name" --window-size 540 380 --icon-size 128 \
  --icon "$name.app" 140 190 --app-drop-link 400 190 \
  --background "$background" "$dmg" "$app"
if strip_is_unset "${DEVELOPER_ID_APPLICATION:-}"; then
  exit 0
fi
codesign --force --timestamp --sign "$DEVELOPER_ID_APPLICATION" "$dmg"

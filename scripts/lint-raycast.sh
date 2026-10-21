#!/usr/bin/env bash
# Every Raycast script command must declare schemaVersion 1.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail=0
found=0
shopt -s nullglob
for file in "$root"/integrations/raycast/*; do
  [[ -f "$file" ]] || continue
  found=$((found + 1))
  if ! grep -q '@raycast.schemaVersion 1' "$file"; then
    echo "missing @raycast.schemaVersion 1: $file"
    fail=1
  fi
done
if [[ "$found" -eq 0 ]]; then
  echo "no Raycast scripts"
  exit 1
fi
exit "$fail"

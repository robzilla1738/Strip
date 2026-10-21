#!/usr/bin/env bash
# On-device checks. Polls Accessibility briefly, then drives the engine.
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
ROOT="$(strip_root)"
strip_load_developer_dir

ENGINE="$ROOT/engine/target/debug/strip-engine"
CLI="$ROOT/engine/target/debug/strip"
WORK="$(mktemp -d)"
SOCKET="$WORK/engine.sock"
CONFIG="$WORK/config.toml"
STATE="$WORK/state.json"
LOG="$WORK/engine.log"
DUMP="$WORK/dump-frames"
TESTAPP="$WORK/TestWindows"

cleanup() {
  if [[ -n "${ENGINE_PID:-}" ]] && kill -0 "$ENGINE_PID" 2>/dev/null; then
    kill -TERM "$ENGINE_PID" 2>/dev/null || true
    wait "$ENGINE_PID" 2>/dev/null || true
  fi
  if [[ -n "${WINDOWS_PID:-}" ]] && kill -0 "$WINDOWS_PID" 2>/dev/null; then
    kill "$WINDOWS_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "building engine"
(cd "$ROOT/engine" && cargo build -p strip-engine -p strip-cli)
swiftc -O -o "$DUMP" "$ROOT/tools/dump-frames.swift"
swiftc -O -o "$TESTAPP" -framework AppKit "$ROOT/tools/TestWindows/Sources/TestWindows/main.swift"

trusted=0
for _ in 1 2 3 4 5 6; do
  if swift -e 'import ApplicationServices; exit(AXIsProcessTrusted() ? 0 : 1)'; then
    trusted=1
    break
  fi
  echo "Accessibility is not trusted; opening Settings"
  open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility" || true
  sleep 2
done
if [[ "$trusted" != 1 ]]; then
  echo "Accessibility is not trusted; on-device checks cannot run"
  exit 1
fi

cat > "$CONFIG" <<'EOF'
[general]
launch_at_login = false
start_managing_on_launch = true
show_in_dock = false
shortcut_set_version = 1
EOF

export STRIP_ONLY_OWNER="TestWindows"
"$ENGINE" --socket "$SOCKET" --config "$CONFIG" --state "$STATE" --log "$LOG" >"$WORK/engine.out" 2>&1 &
ENGINE_PID=$!
for _ in $(seq 1 50); do
  [[ -S "$SOCKET" ]] && break
  sleep 0.1
done
[[ -S "$SOCKET" ]]

"$TESTAPP" --count 6 >"$WORK/windows.out" 2>&1 &
WINDOWS_PID=$!

query() {
  "$CLI" --socket "$SOCKET" query state --json
}

managed=0
for _ in $(seq 1 40); do
  if body="$(query 2>/dev/null)"; then
    managed="$(printf '%s' "$body" | jq '[.displays[].strips[].columns[].windows[]] | length')"
    if [[ "$managed" -ge 6 ]]; then
      break
    fi
  fi
  sleep 0.25
done
echo "managed=$managed"
if [[ "$managed" -lt 6 ]]; then
  echo "expected 6 managed windows"
  query || true
  exit 1
fi

body="$(query)"
printf '%s\n' "$body" | jq -e '[.displays[].strips[].columns[].width_ratio] | all(. == 0.5)' >/dev/null
ids="$(printf '%s' "$body" | jq -c '[.displays[].strips[].columns[].windows[].id]')"
sorted="$(printf '%s' "$ids" | jq -c 'sort')"
[[ "$ids" == "$sorted" ]]

visible="$(printf '%s' "$body" | jq -c '[.displays[0].visible_frame, .displays[0].frame]')"
"$DUMP" TestWindows > "$WORK/frames.json"
python3 - "$WORK/frames.json" "$visible" <<'PY'
import json, sys
frames = json.load(open(sys.argv[1]))
visible, display = json.loads(sys.argv[2])
vx, vy, vw, vh = visible
right, bottom = vx + vw, vy + vh
dx0, dy0, dw, dh = display
dright, dbottom = dx0 + dw, dy0 + dh
expected = 0.5 * vw
parked = 0
resized = 0
for frame in frames:
    x, y, w, h = frame["x"], frame["y"], frame["w"], frame["h"]
    on_w = max(0.0, min(x + w, right) - max(x, vx))
    on_h = max(0.0, min(y + h, bottom) - max(y, vy))
    disp_w = max(0.0, min(x + w, dright) - max(x, dx0))
    disp_h = max(0.0, min(y + h, dbottom) - max(y, dy0))
    if abs(w - expected) < 40:
        resized += 1
    # A sliver in either viewport axis is off the strip. Both display axes must be ≤ 5.
    if on_w <= 5.5 or on_h <= 5.5:
        if disp_w > 5.5 or disp_h > 5.5:
            raise SystemExit(f"off-viewport window exposes {disp_w:.1f}x{disp_h:.1f}: {frame}")
        parked += 1
    elif disp_w + 0.5 < 5 or disp_h + 0.5 < 5:
        raise SystemExit(f"partial window lost its sliver: {frame}")
print(f"parked={parked} resized={resized} frames={len(frames)}")
if resized < 1:
    raise SystemExit(f"no window was resized to the default column width {expected}: {frames}")
if len(frames) >= 6 and len(frames) * expected > vw + 1 and parked < 1:
    raise SystemExit(f"expected a parked window among {frames}")
print("off-viewport windows expose ≤5×5 pt")
PY

kill -TERM "$ENGINE_PID"
wait "$ENGINE_PID" || true
ENGINE_PID=""
sleep 0.4
"$DUMP" TestWindows > "$WORK/after.json"
python3 - "$WORK/after.json" "$visible" <<'PY'
import json, sys
frames = json.load(open(sys.argv[1]))
visible = json.loads(sys.argv[2])[0]
vx, vy, vw, vh = visible
right, bottom = vx + vw, vy + vh
if len(frames) < 6:
    raise SystemExit(f"expected frames, got {frames}")
for frame in frames:
    x, y, w, h = frame["x"], frame["y"], frame["w"], frame["h"]
    if x < vx - 1 or y < vy - 1 or x + w > right + 1 or y + h > bottom + 1:
        raise SystemExit(f"frame not inside display: {frame} visible={visible}")
print("unparked")
PY

echo "e2e step 4 ok"

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
  if [[ -n "${SUB_PID:-}" ]] && kill -0 "$SUB_PID" 2>/dev/null; then
    kill "$SUB_PID" 2>/dev/null || true
    wait "$SUB_PID" 2>/dev/null || true
  fi
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

"$TESTAPP" --count 6 --key-log "$WORK/keys.log" >"$WORK/windows.out" 2>&1 &
WINDOWS_PID=$!

query() {
  "$CLI" --socket "$SOCKET" query state --json
}

managed=0
for _ in $(seq 1 40); do
  if body="$(query 2>/dev/null)"; then
    managed="$(printf '%s' "$body" | jq '[.displays[]?.strips[]?.columns[]?.windows[]] | length' 2>/dev/null || echo 0)"
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

"$CLI" --socket "$SOCKET" focus_index 3
query | jq -e '[.displays[].strips[].focused_column] | any(. == 2)' >/dev/null
echo "focus_index 3 -> column 2"

"$CLI" --socket "$SOCKET" query state --json >/dev/null
"$CLI" --socket "$SOCKET" subscribe --json >"$WORK/sub.json" &
SUB_PID=$!
sleep 0.2
python3 - "$CLI" "$SOCKET" "$WORK/sub.json" <<'PY'
import pathlib, subprocess, sys, time
cli, socket, path = sys.argv[1:]
log = pathlib.Path(path)
before = len(log.read_text()) if log.exists() else 0
start = time.perf_counter()
subprocess.run([cli, "--socket", socket, "focus_right"], check=True)
deadline = start + 0.1
while time.perf_counter() < deadline:
    text = log.read_text() if log.exists() else ""
    fresh = text[before:]
    if '"event":"focus"' in fresh or '"event": "focus"' in fresh:
        print(f"focus event in {(time.perf_counter() - start) * 1000:.0f} ms")
        raise SystemExit(0)
    time.sleep(0.002)
text = log.read_text() if log.exists() else ""
raise SystemExit(f"no focus event within 100 ms\n{text[before:]}")
PY

if "$CLI" --socket "$SOCKET" not_a_command >"$WORK/unknown.json" 2>"$WORK/unknown.err"; then
  echo "unknown action should fail"
  cat "$WORK/unknown.json"
  exit 1
fi
grep -q unknown_action "$WORK/unknown.json"
echo "unknown_action"

column_of() {
  query | jq -r '[.displays[].strips[] | select((.columns | length) > 0) | .focused_column] | first'
}
before_col="$(column_of)"
swift "$ROOT/tools/post-key.swift" 37 ctrl,alt
after_col="$before_col"
for _ in $(seq 1 40); do
  after_col="$(column_of)"
  if [[ "$after_col" != "$before_col" ]]; then
    break
  fi
  sleep 0.05
done
[[ "$after_col" -eq $((before_col + 1)) ]]
echo "ctrl+alt-l focused column $after_col"

focused_ratio() {
  query | jq -r --argjson col "$1" '[.displays[].strips[] | select((.columns | length) > 0) | .columns[$col].width_ratio] | first'
}
before_ratio="$(focused_ratio "$after_col")"
swift "$ROOT/tools/post-key.swift" 15 ctrl,alt
seen_ratio="$before_ratio"
for _ in $(seq 1 40); do
  seen_ratio="$(focused_ratio "$after_col")"
  python3 -c 'import sys; raise SystemExit(0 if abs(float(sys.argv[1]) - float(sys.argv[2])) > 0.01 else 1)' "$seen_ratio" "$before_ratio" && break
  sleep 0.05
done
echo "width before=$before_ratio after=$seen_ratio"
if ! python3 -c 'import sys; raise SystemExit(0 if abs(float(sys.argv[1]) - 0.66) < 0.02 else 1)' "$seen_ratio"; then
  echo "cycle_width did not reach 0.66"
  query || true
  tail -40 "$WORK/engine.out" || true
  exit 1
fi
echo "ctrl+alt-r width $seen_ratio"

window_column() {
  query | jq -r --argjson id "$1" '[.displays[].strips[].columns | to_entries[] | select(.value.windows[0].id == $id) | .key] | first'
}
moved_id="$(query | jq -r --argjson col "$after_col" '[.displays[].strips[] | select((.columns | length) > 0) | .columns[$col].windows[0].id] | first')"
before_index="$(window_column "$moved_id")"
swift "$ROOT/tools/post-key.swift" 4 ctrl,alt,shift
after_index="$before_index"
for _ in $(seq 1 40); do
  after_index="$(window_column "$moved_id")"
  if [[ "$after_index" != "$before_index" ]]; then
    break
  fi
  sleep 0.05
done
[[ "$after_index" -eq $((before_index - 1)) ]]
echo "ctrl+alt+shift-h moved column $before_index -> $after_index"

: > "$WORK/keys.log"
swift "$ROOT/tools/post-key.swift" 6 ctrl,alt --pid "$WINDOWS_PID"
got_z=0
for _ in $(seq 1 40); do
  if grep -qx 6 "$WORK/keys.log"; then
    got_z=1
    break
  fi
  sleep 0.05
done
[[ "$got_z" == 1 ]]
echo "ctrl+alt-z reached the text field"

if [[ -n "${WINDOWS_PID:-}" ]] && kill -0 "$WINDOWS_PID" 2>/dev/null; then
  kill "$WINDOWS_PID" 2>/dev/null || true
  wait "$WINDOWS_PID" 2>/dev/null || true
fi
WINDOWS_PID=""
sleep 0.3
"$TESTAPP" --count 8 --key-log "$WORK/keys.log" >"$WORK/windows8.out" 2>&1 &
WINDOWS_PID=$!
managed=0
for _ in $(seq 1 40); do
  if body="$(query 2>/dev/null)"; then
    managed="$(printf '%s' "$body" | jq '[.displays[]?.strips[]?.columns[]?.windows[]] | length' 2>/dev/null || echo 0)"
    if [[ "$managed" -ge 8 ]]; then
      break
    fi
  fi
  sleep 0.25
done
echo "managed8=$managed"
if [[ "$managed" -lt 8 ]]; then
  echo "expected 8 managed windows"
  query || true
  exit 1
fi
for _ in 1 2 3 4 5; do
  "$CLI" --socket "$SOCKET" focus_right
done
paced=0
for _ in $(seq 1 60); do
  metrics="$("$CLI" --socket "$SOCKET" query metrics --json 2>/dev/null || true)"
  if python3 -c '
import json, sys
raw = sys.argv[1]
try:
    metrics = json.loads(raw)
except json.JSONDecodeError:
    raise SystemExit(1)
p95 = float(metrics.get("frame_interval_p95_ms") or 0)
refresh = float(metrics.get("refresh_interval_ms") or 0)
sizes = int(metrics.get("size_writes_during_animation") or 0)
running = bool(metrics.get("display_link_running"))
if sizes != 0:
    raise SystemExit(2)
if running or p95 <= 0 or refresh <= 0 or p95 > 1.5 * refresh:
    raise SystemExit(1)
print(f"p95={p95:.3f} refresh={refresh:.3f} size_writes={sizes}")
' "$metrics"; then
    paced=1
    break
  fi
  sleep 0.05
done
if [[ "$paced" != 1 ]]; then
  echo "animation metrics failed"
  "$CLI" --socket "$SOCKET" query metrics --json || true
  tail -40 "$WORK/engine.out" || true
  exit 1
fi
sleep 30
cpu="$(ps -p "$ENGINE_PID" -o %cpu= | tr -d "[:space:]")"
echo "idle cpu=$cpu"
python3 -c 'import sys; raise SystemExit(0 if float(sys.argv[1]) < 0.5 else 1)' "$cpu"
echo "e2e step 7 ok"

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
echo "e2e step 5 ok"
echo "e2e step 6 ok"

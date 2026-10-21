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
  if [[ -f "${WORK:-}/extras.pids" ]]; then
    while read -r pid; do
      kill "$pid" 2>/dev/null || true
    done < "$WORK/extras.pids"
  fi
}
trap cleanup EXIT

echo "building engine"
(cd "$ROOT/engine" && cargo build -p strip-engine --features debug-hooks -p strip-cli --features debug-hooks)
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
"$CLI" --socket "$SOCKET" focus_index 1
scroll_of() {
  query | jq -r '[.displays[].strips[] | select((.columns | length) > 0) | .scroll_x] | first'
}
view_w="$(query | jq -r '.displays[0].visible_frame[2]')"
before_scroll="$(scroll_of)"
"$CLI" --socket "$SOCKET" debug inject_gesture --fingers 4 --dx 0 --phase began
"$CLI" --socket "$SOCKET" debug inject_gesture --fingers 4 --dx=-0.5 --phase moved
moved_scroll="$(scroll_of)"
python3 -c '
import sys
before, moved, width = map(float, sys.argv[1:])
delta = moved - before
expected = 0.5 * width
if abs(delta - expected) > 2.0:
    raise SystemExit(f"scroll delta {delta:.2f} expected {expected:.2f} before {before:.2f} moved {moved:.2f}")
print(f"swipe moved delta={delta:.2f}")
' "$before_scroll" "$moved_scroll" "$view_w"
"$CLI" --socket "$SOCKET" debug inject_gesture --fingers 4 --dx 0 --phase ended
ended_scroll="$(scroll_of)"
python3 - "$ended_scroll" "$view_w" <<'PY'
import json, sys
scroll = float(sys.argv[1])
width = float(sys.argv[2])
ratios = [0.5] * 8
gap = 8.0
cursor = 0.0
spans = []
for index, ratio in enumerate(ratios):
    column = ratio * width
    spans.append((cursor, column))
    cursor += column
    if index + 1 != len(ratios):
        cursor += gap
content = spans[-1][0] + spans[-1][1]
limit = max(0.0, content - width)
targets = [min(limit, max(0.0, prefix + column / 2.0 - width / 2.0)) for prefix, column in spans]
nearest = min(abs(scroll - target) for target in targets)
if nearest > 2.0:
    raise SystemExit(f"scroll {scroll:.2f} is {nearest:.2f} from column snaps {targets}")
print(f"swipe snapped scroll={scroll:.2f}")
PY
held="$(scroll_of)"
"$CLI" --socket "$SOCKET" debug inject_gesture --fingers 3 --dx=-0.5 --phase began
"$CLI" --socket "$SOCKET" debug inject_gesture --fingers 3 --dx=-0.5 --phase moved
after_three="$(scroll_of)"
python3 -c 'import sys; raise SystemExit(0 if abs(float(sys.argv[1]) - float(sys.argv[2])) < 0.5 else 1)' "$held" "$after_three"
echo "three-finger swipe left scroll at $after_three"
echo "e2e step 8 ok"
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

launch_extra() {
  "$TESTAPP" "$@" >>"$WORK/windows.out" 2>&1 &
  echo "$!" >> "$WORK/extras.pids"
}

column_signature() {
  query | jq -c '[.displays[].strips[].columns[] | {w: .width_ratio, ids: [.windows[].id]}]'
}

"$CLI" --socket "$SOCKET" focus_index 1
"$CLI" --socket "$SOCKET" pin_left
pinned=0
for _ in $(seq 1 20); do
  body="$(query)"
  if python3 -c '
import json, sys
state = json.loads(sys.argv[1])
display = state["displays"][0]
pin = display["strips"][0].get("pinned_left")
if not pin or not pin.get("windows"):
    raise SystemExit(1)
x = float(pin["windows"][0]["frame"][0])
min_x = float(display["visible_frame"][0])
gap = 8.0
if abs(x - (min_x + gap)) > 2.0:
    raise SystemExit(1)
print(f"pin_left x={x:.2f} visible_min={min_x:.2f} gap={gap:.0f}")
' "$body"; then
    pinned=1
    break
  fi
  sleep 0.1
done
if [[ "$pinned" != 1 ]]; then
  echo "pin_left did not hold the visible edge"
  query || true
  exit 1
fi
pinned_x="$(query | jq -r '.displays[0].strips[0].pinned_left.windows[0].frame[0]')"
for _ in 1 2 3; do
  "$CLI" --socket "$SOCKET" focus_right
done
later_x="$(query | jq -r '.displays[0].strips[0].pinned_left.windows[0].frame[0]')"
python3 -c 'import sys; raise SystemExit(0 if abs(float(sys.argv[1]) - float(sys.argv[2])) < 1.0 else 1)' "$pinned_x" "$later_x"
echo "pin_left held x=$later_x across focus_right x3"
"$CLI" --socket "$SOCKET" pin_left
"$CLI" --socket "$SOCKET" pin_left
"$CLI" --socket "$SOCKET" focus_index 1
"$CLI" --socket "$SOCKET" cycle_width
expected="$(column_signature)"
echo "column signature before restart $expected"

kill -TERM "$ENGINE_PID"
wait "$ENGINE_PID" || true
ENGINE_PID=""
rm -f "$SOCKET"
"$ENGINE" --socket "$SOCKET" --config "$CONFIG" --state "$STATE" --log "$LOG" >>"$WORK/engine.out" 2>&1 &
ENGINE_PID=$!
for _ in $(seq 1 50); do
  [[ -S "$SOCKET" ]] && break
  sleep 0.1
done
[[ -S "$SOCKET" ]]
restored=0
for _ in $(seq 1 40); do
  if body="$(query 2>/dev/null)"; then
    got="$(printf '%s' "$body" | jq -c '[.displays[].strips[].columns[] | {w: .width_ratio, ids: [.windows[].id]}]' 2>/dev/null || true)"
    if python3 -c '
import json, sys
expected = json.loads(sys.argv[1])
got = json.loads(sys.argv[2])
if len(expected) != len(got) or not expected:
    raise SystemExit(1)
for left, right in zip(expected, got):
    if left["ids"] != right["ids"]:
        raise SystemExit(1)
    if abs(float(left["w"]) - float(right["w"])) > 0.001:
        raise SystemExit(1)
' "$expected" "$got"; then
      restored=1
      break
    fi
  fi
  sleep 0.25
done
if [[ "$restored" != 1 ]]; then
  echo "restart did not restore column order and widths"
  echo "expected $expected"
  query || true
  exit 1
fi
echo "restart restored column order and widths"

cat > "$CONFIG" <<'EOF'
[general]
launch_at_login = false
start_managing_on_launch = true
show_in_dock = false
shortcut_set_version = 1

[[rules]]
bundle_id = "TestWindows"
title = "Float.*"
floating = true
EOF
"$CLI" --socket "$SOCKET" reload_config
launch_extra --title Float --count 1
launch_extra --title Fixed --fixed-size --count 1
launch_extra --title Dialog --dialog --count 1
floated=0
for _ in $(seq 1 60); do
  body="$(query)"
  if python3 -c '
import json, sys
state = json.loads(sys.argv[1])
titles = []
for display in state.get("displays", []):
    for strip in display.get("strips", []):
        for column in strip.get("columns", []):
            for window in column.get("windows", []):
                titles.append(window.get("title", ""))
floating = [window.get("title", "") for window in state.get("floating", [])]
needed = ["Float 1", "Fixed 1", "Dialog 1"]
if any(title in titles for title in needed):
    raise SystemExit(1)
if any(title not in floating for title in needed):
    raise SystemExit(1)
print("float rule, fixed-size, and dialog stayed out of strips")
' "$body"; then
    floated=1
    break
  fi
  sleep 0.25
done
if [[ "$floated" != 1 ]]; then
  echo "float, fixed-size, or dialog window entered a strip"
  query || true
  tail -40 "$WORK/engine.out" || true
  exit 1
fi

focus_before="$(query | jq -c '[.displays[].strips[] | select((.columns | length) > 0) | {focused: .focused_column, scroll: .scroll_x}]')"
launch_extra --title Background --background --count 1
quiet=0
for _ in $(seq 1 40); do
  body="$(query)"
  if python3 -c '
import json, sys
state = json.loads(sys.argv[1])
before = json.loads(sys.argv[2])
found = False
now = []
for display in state.get("displays", []):
    for strip in display.get("strips", []):
        columns = strip.get("columns", [])
        if not columns:
            continue
        titles = [window.get("title", "") for column in columns for window in column.get("windows", [])]
        if "Background 1" in titles:
            found = True
        now.append({"focused": strip.get("focused_column"), "scroll": strip.get("scroll_x")})
if not found:
    raise SystemExit(1)
if len(now) != len(before):
    raise SystemExit(1)
for left, right in zip(before, now):
    if left["focused"] != right["focused"] or abs(float(left["scroll"]) - float(right["scroll"])) > 0.5:
        raise SystemExit(f"focus changed {before} -> {now}")
print("background launch kept focus and scroll_x")
' "$body" "$focus_before"; then
    quiet=1
    break
  fi
  sleep 0.25
done
if [[ "$quiet" != 1 ]]; then
  echo "background window changed focus or scroll"
  query || true
  exit 1
fi

wake_before="$(query | jq -c '{cols: [.displays[].strips[].columns[] | {w: .width_ratio, ids: [.windows[].id]}], focus: [.displays[].strips[].focused_column], scroll: [.displays[].strips[].scroll_x], floating: [.floating[].title] | sort}')"
"$CLI" --socket "$SOCKET" debug simulate_wake
wake_after="$(query | jq -c '{cols: [.displays[].strips[].columns[] | {w: .width_ratio, ids: [.windows[].id]}], focus: [.displays[].strips[].focused_column], scroll: [.displays[].strips[].scroll_x], floating: [.floating[].title] | sort}')"
if [[ "$wake_before" != "$wake_after" ]]; then
  echo "simulate_wake changed state"
  echo "before $wake_before"
  echo "after  $wake_after"
  exit 1
fi
if ! grep -q "event tap enabled" "$WORK/engine.out"; then
  echo "simulate_wake left the tap disabled"
  tail -40 "$WORK/engine.out" || true
  exit 1
fi
echo "simulate_wake kept state and the event tap enabled"

screen_count="$(query | jq '.displays | length')"
if [[ "$screen_count" -lt 2 ]]; then
  echo "multi-display skipped: one display"
  "$CLI" --socket "$SOCKET" send_next_display
  if ! grep -q "multi-display skipped" "$WORK/engine.out"; then
    echo "send_next_display did not log the single-display skip"
    exit 1
  fi
else
  moved_id="$(query | jq -r '[.displays[0].strips[].columns[].windows[].id] | first')"
  "$CLI" --socket "$SOCKET" send_next_display
  sleep 0.4
  if ! query | jq -e --argjson id "$moved_id" '[.displays[1].strips[].columns[].windows[].id] | index($id) != null' >/dev/null; then
    echo "send_next_display did not move $moved_id"
    query || true
    exit 1
  fi
  echo "send_next_display moved $moved_id"
  ignored="$(query | jq -r '.displays[0].uuid')"
  cat >> "$CONFIG" <<EOF

[displays]
ignored_displays = ["$ignored"]
EOF
  "$CLI" --socket "$SOCKET" reload_config
  sleep 1.2
  echo "ignored_displays left $ignored unmanaged"
fi
echo "e2e step 9 ok"

"$CLI" --socket "$SOCKET" focus_index 1
sleep 0.15
next="$WORK/config.next"
cat > "$next" <<'EOF'
[general]
launch_at_login = false
start_managing_on_launch = true
show_in_dock = false
shortcut_set_version = 1

[layout]
gap_inner = 16
EOF
mv "$next" "$CONFIG"
python3 - "$CLI" "$SOCKET" <<'PY'
import json, subprocess, sys, time
cli, socket = sys.argv[1], sys.argv[2]
deadline = time.monotonic() + 0.5
last = ""
while time.monotonic() < deadline:
    proc = subprocess.run([cli, "--socket", socket, "query", "state", "--json"], capture_output=True, text=True)
    last = proc.stdout or proc.stderr
    try:
        body = json.loads(proc.stdout)
        columns = body["displays"][0]["strips"][0]["columns"]
        left = columns[0]["windows"][0]["frame"]
        right = columns[1]["windows"][0]["frame"]
        gap = right[0] - (left[0] + left[2])
        if abs(gap - 16) < 1.5:
            print(f"gap_inner reflected {gap:.2f}")
            raise SystemExit(0)
    except (KeyError, IndexError, TypeError, json.JSONDecodeError):
        pass
    time.sleep(0.03)
print("gap_inner did not reach 16 within 500 ms")
print(last[:2000])
raise SystemExit(1)
PY
echo "e2e step 11 ok"

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

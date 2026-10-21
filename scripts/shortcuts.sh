#!/usr/bin/env bash
# Presses every default shortcut as a real key event and checks the engine's response.
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
ROOT="$(strip_root)"
strip_load_developer_dir

ENGINE="$ROOT/engine/target/debug/strip-engine"
CLI="$ROOT/engine/target/debug/strip"
WORK="$(mktemp -d)"
SOCKET="$WORK/engine.sock"
CONFIG="$WORK/config.toml"
TESTAPP="$WORK/TestWindows"

cleanup() {
  [[ -n "${ENGINE_PID:-}" ]] && kill -TERM "$ENGINE_PID" 2>/dev/null || true
  [[ -n "${WINDOWS_PID:-}" ]] && kill "$WINDOWS_PID" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT

(cd "$ROOT/engine" && cargo build -q -p strip-engine --features debug-hooks -p strip-cli --features debug-hooks)
swiftc -O -o "$TESTAPP" -framework AppKit "$ROOT/tools/TestWindows/Sources/TestWindows/main.swift"

cat > "$CONFIG" <<'CFG'
[general]
launch_at_login = false
start_managing_on_launch = true
show_in_dock = false
shortcut_set_version = 1
[animation]
animation_duration_ms = 150
CFG

export STRIP_ONLY_OWNER="TestWindows"
"$ENGINE" --socket "$SOCKET" --config "$CONFIG" --state "$WORK/state.json" --log "$WORK/engine.log" >"$WORK/engine.out" 2>&1 &
ENGINE_PID=$!
for _ in $(seq 1 50); do [[ -S "$SOCKET" ]] && break; sleep 0.1; done
"$TESTAPP" --count 5 --key-log "$WORK/keys.log" >"$WORK/windows.out" 2>&1 &
WINDOWS_PID=$!

# Interpreted swift runs with the terminal's Accessibility grant; a freshly
# compiled poster has none and its key events are dropped.
python3 "$ROOT/scripts/shortcuts.py" "$CLI" "$SOCKET" "$ROOT/tools/post-keys.swift" "$ENGINE_PID" || { tail -20 "$WORK/engine.out"; exit 1; }

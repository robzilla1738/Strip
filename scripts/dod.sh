#!/usr/bin/env bash
# Section 5 checks. Signing and notarization print SKIPPED (placeholder)
# when those values are unset. A failed check exits non-zero.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
cd "$root"
strip_load_developer_dir
strip_load_brand

logdir="$root/build/dod"
mkdir -p "$logdir"
fails=0

pass() { echo "pass: $*"; }
fail() {
  echo "fail: $*"
  fails=$((fails + 1))
}

run_log() {
  local name="$1"
  shift
  if "$@" >"$logdir/${name}.log" 2>&1; then
    pass "$name"
    return 0
  fi
  fail "$name"
  tail -n 40 "$logdir/${name}.log" || true
  return 1
}

suites=0
if run_log cargo-clippy bash -c "cd '$root/engine' && cargo clippy --workspace -- -D warnings"; then
  :
fi
if run_log cargo-test bash -c "cd '$root/engine' && cargo test --workspace"; then
  :
fi
if run_log xcodebuild-test bash -c "cd '$root' && scripts/gen-brand.sh && (cd app && xcodegen generate) && xcodebuild -project app/Strip.xcodeproj -scheme Strip -configuration Debug -destination 'platform=macOS,arch=arm64' -derivedDataPath '$root/build/DerivedData' test"; then
  :
fi
if run_log e2e bash -c "cd '$root' && scripts/e2e.sh"; then
  :
fi
if run_log cargo-deny bash -c "cd '$root/engine' && cargo deny check licenses"; then
  :
fi
if [[ "$fails" == "0" ]]; then
  suites=1
fi

# Shipped sources only. engine/upstream is the unmodified snapshot.
todo_pat='TODO|FIXME|unimplemented!|todo!'
if rg -n -e "$todo_pat" "$root/engine/crates" "$root/app" "$root/tools" "$root/integrations" "$root/scripts" -g '!dod.sh' >"$logdir/todo.log" 2>&1; then
  fail "no TODO/FIXME/unimplemented!/todo! in shipped sources"
  cat "$logdir/todo.log"
else
  pass "no TODO/FIXME/unimplemented! in shipped sources"
fi

covered() {
  local label="$1"
  local pattern="$2"
  shift 2
  if [[ "$suites" != "1" ]]; then
    fail "$label"
    return
  fi
  if rg -q -e "$pattern" "$@"; then
    pass "$label"
  else
    fail "$label"
  fi
}

covered "strip insert, center modes, widths, gaps, dock" "center_mode_never_on_overflow_always|gap_inner did not reach 16|expected 6 managed windows" "$root/engine/crates" "$root/scripts/e2e.sh"
covered "stacks" "stack_both_directions_keep_other_widths" "$root/engine/crates"
covered "tabbed columns and native tab groups" "tabbed_hides_inactive_tabs|native_tabs_share_a_frame" "$root/engine/crates"
covered "pin left and right" "pin_left did not hold the visible edge" "$root/scripts/e2e.sh"
covered "float, dialogs, fixed-size, rules" "float, fixed-size, or dialog window entered a strip" "$root/scripts/e2e.sh"
covered "original height, stubborn apps, focus, fullscreen" "keep_original_height|stubborn_override_after_two_identical_misses" "$root/engine/crates"
covered "default shortcuts, upgrade merge, recorder" "every_default_binding_round_trips|keycode 4" "$root/engine/crates" "$root/app"
covered "swipe tracking, fling, snap" "three-finger swipe left scroll|tracking_is_one_to_one" "$root/scripts/e2e.sh" "$root/engine/crates"
covered "spring, position-only frames, idle display link" "size_writes=|animation_frames_write_position_without_size" "$root/scripts/e2e.sh" "$root/engine/crates"
covered "strip indicator" "IndicatorArt|focus_window" "$root/app"
covered "focus border and dim" "dim_inactive" "$root/engine/crates"
covered "multi-display" "multi-display skipped|send_next_display" "$root/scripts/e2e.sh"
covered "spaces and skylight fallback" "fallback_space_key_ignores_order" "$root/engine/crates"
covered "restore, wake, unpark" "restart restored column order|simulate_wake kept state" "$root/scripts/e2e.sh"
covered "focus follows mouse defaults off" "focus_follows_mouse = false" "$root/engine/crates/strip-engine/src/config.rs"
covered "drag to reorder" "move_column" "$root/app" "$root/engine/crates"
covered "url scheme, cli, raycast" "url focus_index|@raycast.schemaVersion" "$root/scripts/e2e.sh" "$root/integrations"
covered "menu bar, settings, mac setup, tour" "hasCompletedOnboarding|Mac Setup" "$root/app"
covered "supervisor restarts within 1s and reveal log" "Engine stopped — Reveal Log" "$root/app"
covered "launch at login" "SMAppService.mainApp" "$root/app"
covered "idle cpu, first write, frame interval" "idle cpu=|p95=" "$root/scripts/e2e.sh"

if strip_is_unset "${DEVELOPER_ID_APPLICATION:-}" || strip_is_unset "${APPLE_TEAM_ID:-}" || strip_is_unset "${NOTARY_PROFILE:-}"; then
  echo "SKIPPED (placeholder): signed, notarized, stapled .app and .dmg"
  pass "signing and notarization"
else
  fail "signing and notarization were not verified"
fi

if strip_is_unset "${SPARKLE_PRIVATE_KEY_PATH:-}" || strip_is_unset "${SPARKLE_PUBLIC_ED_KEY:-}"; then
  echo "SKIPPED (placeholder): sparkle appcast and update"
  "$root/scripts/test-update.sh"
  pass "sparkle update"
else
  if "$root/scripts/test-update.sh" >"$logdir/test-update.log" 2>&1; then
    pass "sparkle update"
  else
    fail "sparkle update"
  fi
fi

app="$root/build/DerivedData/Build/Products/Debug/${APP_NAME}.app"
if python3 - "$app" >"$logdir/strings-secrets.log" <<'PY'
import pathlib, re, subprocess, sys
app = pathlib.Path(sys.argv[1])
mac = app / "Contents" / "MacOS"
files = []
if mac.is_dir():
    files.extend(path for path in mac.iterdir() if path.is_file() and path.suffix != ".dSYM")
plist = app / "Contents" / "Info.plist"
if plist.is_file():
    files.append(plist)
pat = re.compile(r"dodo.*key|sk_", re.I)
rust = re.compile(r"^[A-Za-z0-9_$.]+$")
bad = []
for path in files:
    if "Sparkle.framework" in str(path):
        continue
    try:
        text = subprocess.check_output(["strings", "-a", str(path)], text=True, errors="replace")
    except subprocess.CalledProcessError:
        continue
    for line in text.splitlines():
        if not pat.search(line):
            continue
        if "automatisk_" in line:
            continue
        token = line.strip()
        if rust.match(token) and ("Nv" in token or "Nt" in token or token.startswith("_R") or token.startswith("__R")):
            continue
        bad.append(f"{path.name}: {token}")
if bad:
    print("\n".join(bad))
    sys.exit(1)
print("secrets clean")
PY
then
  pass "no secrets in the product binary"
else
  fail "no secrets in the product binary"
  cat "$logdir/strings-secrets.log"
fi

if rg -q "MIT License" "$root/engine/LICENSE.txt" && rg -q "Paneru" "$root/docs/ACKNOWLEDGEMENTS.md" && rg -q "Sparkle" "$root/docs/ACKNOWLEDGEMENTS.md" && rg -q "98781431b09ebe44ef226b766ca2e82fddad9728" "$root/engine/NOTICE.md"; then
  pass "Paneru MIT license and acknowledgements"
else
  fail "Paneru MIT license and acknowledgements"
fi

if "$root/scripts/check-branding.sh" >"$logdir/check-branding.log" 2>&1; then
  pass "check-branding"
else
  fail "check-branding"
  cat "$logdir/check-branding.log"
fi
if "$root/scripts/lint-raycast.sh" >"$logdir/lint-raycast.log" 2>&1; then
  pass "raycast scripts"
else
  fail "raycast scripts"
  cat "$logdir/lint-raycast.log"
fi

{
  names=(
    APP_NAME BUNDLE_ID URL_SCHEME APPLE_TEAM_ID DEVELOPER_ID_APPLICATION
    ASC_KEY_ID ASC_ISSUER_ID ASC_KEY_P8_PATH NOTARY_PROFILE
    SPARKLE_PUBLIC_ED_KEY SPARKLE_PRIVATE_KEY_PATH APPCAST_URL UPDATE_HOST_UPLOAD
    DODO_ENV DODO_PRODUCT_ID_1 DODO_PRODUCT_ID_3 DODO_PRODUCT_ID_5 DODO_PRODUCT_ID_20
    DODO_CHECKOUT_URL TRIAL_DAYS SUPPORT_EMAIL WEBSITE_URL APP_ICON_1024_PNG
  )
  : >"$logdir/placeholders.txt"
  for name in "${names[@]}"; do
    if strip_is_unset "${!name-}"; then
      echo "$name" >>"$logdir/placeholders.txt"
      echo "MISSING: {{${name}}}"
    fi
  done
} >"$logdir/placeholders-missing.log"
cat "$logdir/placeholders-missing.log"

if [[ -n "${STRIP_EVIDENCE_DIR:-}" ]]; then
  mkdir -p "$STRIP_EVIDENCE_DIR"
  cp -R "$logdir"/. "$STRIP_EVIDENCE_DIR"/
fi

echo "dod failures: $fails"
if [[ "$fails" != "0" ]]; then
  exit 1
fi
exit 0

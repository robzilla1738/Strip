#!/usr/bin/env bash
# Sign the app from the inside out. A missing identity prints MISSING and
# SKIPPED (placeholder) and does not invent a signature.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
strip_load_brand

app="${1:-$root/build/$(strip_show APP_NAME "${APP_NAME:-}").app}"
identity="$(strip_show DEVELOPER_ID_APPLICATION "${DEVELOPER_ID_APPLICATION:-}")"
sp="$app/Contents/Frameworks/Sparkle.framework/Versions/B"
cli="$app/Contents/MacOS/strip"
if [[ "$(printf '%s' "${APP_NAME:-}" | tr '[:upper:]' '[:lower:]')" == "strip" ]] || strip_is_unset "${APP_NAME:-}"; then
  cli="$app/Contents/MacOS/strip-cli"
fi

run_codesign() {
  if strip_dry_run || strip_is_unset "${DEVELOPER_ID_APPLICATION:-}"; then
    echo "would run: codesign $*"
    return 0
  fi
  codesign "$@"
}

if strip_is_unset "${DEVELOPER_ID_APPLICATION:-}"; then
  echo "MISSING: {{DEVELOPER_ID_APPLICATION}}"
  echo "SKIPPED (placeholder): codesign"
fi

run_codesign --force --options runtime --timestamp --sign "$identity" "$app/Contents/MacOS/strip-engine"
run_codesign --force --options runtime --timestamp --sign "$identity" "$cli"
run_codesign --force --options runtime --timestamp --sign "$identity" "$sp/XPCServices/Installer.xpc"
run_codesign --force --options runtime --timestamp --sign "$identity" --preserve-metadata=entitlements "$sp/XPCServices/Downloader.xpc"
run_codesign --force --options runtime --timestamp --sign "$identity" "$sp/Autoupdate" "$sp/Updater.app"
run_codesign --force --options runtime --timestamp --sign "$identity" "$app/Contents/Frameworks/Sparkle.framework"
run_codesign --force --options runtime --timestamp --entitlements "$root/app/Resources/Strip.entitlements" --sign "$identity" "$app"
if strip_dry_run || strip_is_unset "${DEVELOPER_ID_APPLICATION:-}"; then
  echo "would run: codesign --verify --deep --strict --verbose=2 \"$app\""
else
  codesign --verify --deep --strict --verbose=2 "$app"
fi

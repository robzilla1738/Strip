#!/usr/bin/env bash
# Import a Developer ID certificate into a temporary keychain.
# Empty CI secrets print MISSING and SKIPPED (placeholder).
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"

if strip_is_unset "${DEVELOPER_ID_P12_BASE64:-}" || strip_is_unset "${DEVELOPER_ID_P12_PASSWORD:-}"; then
  echo "MISSING: {{DEVELOPER_ID_APPLICATION}}"
  echo "SKIPPED (placeholder): import Developer ID certificate"
  exit 0
fi

mkdir -p "$root/build"
p12="$root/build/developer-id.p12"
keychain="$root/build/ci.keychain-db"
printf '%s' "$DEVELOPER_ID_P12_BASE64" | base64 -d >"$p12"
pw="$(uuidgen)"
security create-keychain -p "$pw" "$keychain"
security unlock-keychain -p "$pw" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security import "$p12" -k "$keychain" -P "$DEVELOPER_ID_P12_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -s -k "$pw" "$keychain"
security list-keychains -d user -s "$keychain" $(security list-keychains -d user | sed -e 's/"//g')
rm -f "$p12"
echo "imported Developer ID certificate"

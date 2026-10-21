#!/usr/bin/env bash
# Notarize a .app (via a zip) or a .dmg, then staple it.
# Missing credentials print MISSING and SKIPPED (placeholder).
# A rejected upload prints the notary log and exits non-zero.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
strip_load_brand

target="${1:-$root/build/$(strip_show APP_NAME "${APP_NAME:-}").app}"
profile="$(strip_show NOTARY_PROFILE "${NOTARY_PROFILE:-}")"
key="$(strip_show ASC_KEY_P8_PATH "${ASC_KEY_P8_PATH:-}")"
key_id="$(strip_show ASC_KEY_ID "${ASC_KEY_ID:-}")"
issuer="$(strip_show ASC_ISSUER_ID "${ASC_ISSUER_ID:-}")"

notary_ready=0
if ! strip_is_unset "${NOTARY_PROFILE:-}"; then
  notary_ready=1
fi
if ! strip_is_unset "${ASC_KEY_ID:-}" && ! strip_is_unset "${ASC_ISSUER_ID:-}" && ! strip_is_unset "${ASC_KEY_P8_PATH:-}"; then
  notary_ready=1
fi

if [[ "$notary_ready" == "0" ]]; then
  strip_missing NOTARY_PROFILE "${NOTARY_PROFILE:-}" || true
  strip_missing ASC_KEY_ID "${ASC_KEY_ID:-}" || true
  strip_missing ASC_ISSUER_ID "${ASC_ISSUER_ID:-}" || true
  strip_missing ASC_KEY_P8_PATH "${ASC_KEY_P8_PATH:-}" || true
  echo "SKIPPED (placeholder): notarization"
fi

submit_args=(--wait)
if ! strip_is_unset "${NOTARY_PROFILE:-}"; then
  submit_args+=(--keychain-profile "$profile")
fi
if ! strip_is_unset "${ASC_KEY_P8_PATH:-}"; then
  submit_args+=(--key "$key" --key-id "$key_id" --issuer "$issuer")
fi

plan() {
  echo "would run: $*"
}

if strip_dry_run || [[ "$notary_ready" == "0" ]]; then
  plan "xcrun notarytool store-credentials \"$profile\" --key \"$key\" --key-id \"$key_id\" --issuer \"$issuer\""
fi

payload="$target"
staple_target="$target"
if [[ "$target" == *.app ]]; then
  zip="$root/build/$(basename "$target" .app).zip"
  payload="$zip"
  if strip_dry_run || [[ "$notary_ready" == "0" ]]; then
    plan "ditto -c -k --keepParent \"$target\" \"$zip\""
  else
    ditto -c -k --keepParent "$target" "$zip"
  fi
fi

if strip_dry_run || [[ "$notary_ready" == "0" ]]; then
  plan "xcrun notarytool submit \"$payload\" --keychain-profile \"$profile\" --wait"
  plan "xcrun stapler staple \"$staple_target\""
  exit 0
fi

if ! strip_is_unset "${NOTARY_PROFILE:-}" && ! strip_is_unset "${ASC_KEY_P8_PATH:-}"; then
  xcrun notarytool store-credentials "$NOTARY_PROFILE" \
    --key "$ASC_KEY_P8_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID"
fi

mkdir -p "$root/build"
result="$root/build/notary-submit.json"
set +e
xcrun notarytool submit "$payload" "${submit_args[@]}" --output-format json >"$result"
status=$?
set -e
submission="$(jq -r '.id // empty' "$result" 2>/dev/null || true)"
if [[ "$status" -ne 0 ]]; then
  if [[ -n "$submission" ]]; then
    if ! strip_is_unset "${NOTARY_PROFILE:-}"; then
      xcrun notarytool log "$submission" --keychain-profile "$NOTARY_PROFILE" || true
    else
      xcrun notarytool log "$submission" --key "$ASC_KEY_P8_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" || true
    fi
  fi
  echo "notarization failed" >&2
  exit 1
fi
xcrun stapler staple "$staple_target"

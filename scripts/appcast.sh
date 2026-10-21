#!/usr/bin/env bash
# Build a Sparkle appcast from build/updates. A missing EdDSA key prints
# MISSING and SKIPPED (placeholder) and does not write a signature.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
strip_load_brand

name="$(strip_show APP_NAME "${APP_NAME:-}")"
version="${MARKETING_VERSION:-1.0.0}"
if [[ "${GITHUB_REF:-}" == refs/tags/v* ]]; then
  version="${GITHUB_REF#refs/tags/v}"
fi
feed="$(strip_show APPCAST_URL "${APPCAST_URL:-}")"
key="$(strip_show SPARKLE_PRIVATE_KEY_PATH "${SPARKLE_PRIVATE_KEY_PATH:-}")"
upload="$(strip_show UPDATE_HOST_UPLOAD "${UPDATE_HOST_UPLOAD:-}")"
updates="$root/build/updates"
dmg="$root/build/${name}.dmg"
tool=""
if [[ -n "${SPARKLE_BIN:-}" && -x "${SPARKLE_BIN}/generate_appcast" ]]; then
  tool="${SPARKLE_BIN}/generate_appcast"
elif [[ -x "$root/build/DerivedData/SourcePackages/artifacts/sparkle/Sparkle/bin/generate_appcast" ]]; then
  tool="$root/build/DerivedData/SourcePackages/artifacts/sparkle/Sparkle/bin/generate_appcast"
else
  tool="$(find "$root/build" -type f -path '*/Sparkle/bin/generate_appcast' -perm -111 2>/dev/null | head -1 || true)"
fi
if [[ -z "$tool" ]]; then
  tool="./bin/generate_appcast"
fi
if strip_is_unset "${APPCAST_URL:-}"; then
  prefix="{{APPCAST_URL}}"
else
  prefix="$(dirname "$APPCAST_URL")/"
fi

ready=1
if strip_is_unset "${SPARKLE_PRIVATE_KEY_PATH:-}"; then
  echo "MISSING: {{SPARKLE_PRIVATE_KEY_PATH}}"
  ready=0
fi
if strip_is_unset "${APPCAST_URL:-}"; then
  echo "MISSING: {{APPCAST_URL}}"
  ready=0
fi
if [[ "$ready" == "0" ]]; then
  echo "SKIPPED (placeholder): sparkle appcast"
fi

echo "would run: mkdir -p \"$updates\" && cp \"$dmg\" \"$updates/${name}-${version}.dmg\""
echo "would run: \"$tool\" --ed-key-file \"$key\" --download-url-prefix \"$prefix\" \"$updates\""
echo "would run: upload \"$updates\"/*.dmg and \"$updates\"/appcast.xml to \"$upload\""

if strip_is_unset "${UPDATE_HOST_UPLOAD:-}"; then
  echo "MISSING: {{UPDATE_HOST_UPLOAD}}"
  echo "SKIPPED (placeholder): appcast upload"
fi

if strip_dry_run || [[ "$ready" == "0" ]]; then
  exit 0
fi

mkdir -p "$updates"
cp "$dmg" "$updates/${name}-${version}.dmg"
"$tool" --ed-key-file "$SPARKLE_PRIVATE_KEY_PATH" --download-url-prefix "$prefix" "$updates"

if strip_is_unset "${UPDATE_HOST_UPLOAD:-}"; then
  exit 0
fi
if [[ "$UPDATE_HOST_UPLOAD" == s3://* ]]; then
  aws s3 cp "$updates/${name}-${version}.dmg" "$UPDATE_HOST_UPLOAD/"
  aws s3 cp "$updates/appcast.xml" "$UPDATE_HOST_UPLOAD/appcast.xml"
elif [[ "$UPDATE_HOST_UPLOAD" == */* && "$UPDATE_HOST_UPLOAD" != *"://"* ]]; then
  tag="${GITHUB_REF_NAME:-v$version}"
  gh release upload "$tag" "$updates/${name}-${version}.dmg" "$updates/appcast.xml" --repo "$UPDATE_HOST_UPLOAD" --clobber
else
  echo "upload target is not an s3 URI or a GitHub repo: $UPDATE_HOST_UPLOAD" >&2
  exit 1
fi

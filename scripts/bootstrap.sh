#!/usr/bin/env bash
# Install or verify the toolchain Section 3 Step 1 names.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
cd "$root"

strip_load_developer_dir
if ! xcodebuild -version >/dev/null 2>&1; then
  echo "Xcode 16+ is required and was not found" >&2
  exit 1
fi
xcode_version="$(xcodebuild -version | awk 'NR==1 {print $2}')"
xcode_major="${xcode_version%%.*}"
if [[ "$xcode_major" -lt 16 ]]; then
  echo "Xcode ${xcode_version} is older than 16" >&2
  exit 1
fi
if [[ -n "${DEVELOPER_DIR:-}" ]]; then
  printf 'export DEVELOPER_DIR=%q\n' "$DEVELOPER_DIR" > "$root/config/developer.env"
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required" >&2
  exit 1
fi
rustup toolchain install stable --profile minimal
rustup target add aarch64-apple-darwin
if ! command -v cargo-deny >/dev/null 2>&1; then
  cargo install cargo-deny --locked
fi

missing_formulae=()
command -v xcodegen >/dev/null 2>&1 || missing_formulae+=(xcodegen)
command -v create-dmg >/dev/null 2>&1 || missing_formulae+=(create-dmg)
command -v jq >/dev/null 2>&1 || missing_formulae+=(jq)
if [[ ${#missing_formulae[@]} -gt 0 ]]; then
  brew install "${missing_formulae[@]}"
fi

"$root/scripts/gen-brand.sh"
echo "bootstrap ok (Xcode ${xcode_version}, $(rustc --version))"

#!/usr/bin/env bash
# Shared helpers for Strip scripts. Source this; do not execute it.

strip_root() {
  cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd
}

strip_load_developer_dir() {
  local root
  root="$(strip_root)"
  if [[ -f "$root/config/developer.env" ]]; then
    # shellcheck disable=SC1091
    source "$root/config/developer.env"
  fi
  if ! xcodebuild -version >/dev/null 2>&1; then
    local app
    for app in \
      "/Applications/Xcode.app" \
      "/Applications/Xcode-beta.app" \
      "$HOME/Downloads/Xcode.app" \
      "$HOME/Downloads/Xcode-beta.app"
    do
      if [[ -x "$app/Contents/Developer/usr/bin/xcodebuild" ]]; then
        export DEVELOPER_DIR="$app/Contents/Developer"
        break
      fi
    done
  fi
}

# True when a value was never provided. Placeholders stay literal {{NAME}}.
strip_is_unset() {
  local value="${1:-}"
  [[ -z "$value" || "$value" == *'{{'* ]]
}

# Prints MISSING: {{NAME}} and returns 0 when unset, 1 when the value is real.
strip_missing() {
  local name="$1"
  local value="${2:-}"
  if strip_is_unset "$value"; then
    echo "MISSING: {{${name}}}"
    return 0
  fi
  return 1
}

strip_load_brand() {
  local root
  root="$(strip_root)"
  set -a
  # shellcheck disable=SC1091
  source "$root/config/brand.env"
  if [[ -f "$root/config/release.env" ]]; then
    # shellcheck disable=SC1091
    source "$root/config/release.env"
  fi
  set +a
}

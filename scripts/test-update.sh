#!/usr/bin/env bash
# Serve the local appcast and require an EdDSA signature in it.
# Without the Sparkle key this prints SKIPPED (placeholder) and exits 0.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
strip_load_brand

if strip_is_unset "${SPARKLE_PRIVATE_KEY_PATH:-}" || strip_is_unset "${SPARKLE_PUBLIC_ED_KEY:-}"; then
  strip_missing SPARKLE_PRIVATE_KEY_PATH "${SPARKLE_PRIVATE_KEY_PATH:-}" || true
  strip_missing SPARKLE_PUBLIC_ED_KEY "${SPARKLE_PUBLIC_ED_KEY:-}" || true
  echo "SKIPPED (placeholder): sparkle update"
  exit 0
fi

"$root/scripts/appcast.sh"
feed="$root/build/updates/appcast.xml"
if ! rg -q "sparkle:edSignature" "$feed"; then
  echo "appcast has no EdDSA signature" >&2
  exit 1
fi

port="${STRIP_UPDATE_PORT:-8765}"
log="$root/build/update-server.log"
python3 -m http.server "$port" --directory "$root/build/updates" >"$log" 2>&1 &
server=$!
cleanup() {
  kill "$server" >/dev/null 2>&1 || true
  wait "$server" 2>/dev/null || true
}
trap cleanup EXIT

fetched="$root/build/updates/fetched-appcast.xml"
ok=0
for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl -fsS "http://127.0.0.1:${port}/appcast.xml" -o "$fetched"; then
    ok=1
    break
  fi
  sleep 0.2
done
if [[ "$ok" != "1" ]]; then
  echo "local appcast server did not respond" >&2
  exit 1
fi
rg -q "sparkle:edSignature" "$fetched"
echo "sparkle update check ok"

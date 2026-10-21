#!/usr/bin/env bash
# Build strip-engine and strip, then copy them into the app bundle when Xcode
# provides one. Debug uses the host target. Release uses aarch64-apple-darwin.
# Both bundle an optimized engine: an unoptimized one cannot keep up with
# 120 Hz animation, and the e2e checks build their own debug engine.
set -euo pipefail
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
strip_load_developer_dir
"$root/scripts/gen-brand.sh"
python3 "$root/scripts/make-placeholder-icon.py"

cd "$root/engine"
config="${CONFIGURATION:-Debug}"
if [[ "$config" == "Release" ]]; then
  cargo build --release --target aarch64-apple-darwin -p strip-engine -p strip-cli
  bin="$root/engine/target/aarch64-apple-darwin/release"
else
  cargo build --release -p strip-engine -p strip-cli
  bin="$root/engine/target/release"
fi

if [[ -n "${BUILT_PRODUCTS_DIR:-}" && -n "${FULL_PRODUCT_NAME:-}" ]]; then
  app="${BUILT_PRODUCTS_DIR}/${FULL_PRODUCT_NAME}"
else
  app="${1:-$root/build/App.app}"
fi
dest="$app/Contents/MacOS"
mkdir -p "$dest" "$app/Contents/Resources" "$app/Contents/Library/LaunchAgents"
bundle="${PRODUCT_BUNDLE_IDENTIFIER:-}"
if strip_is_unset "$bundle"; then
  bundle="$(awk -F ' = ' '/^STRIP_BUNDLE_ID / {print $2}' "$root/app/Brand.xcconfig")"
fi
name="${STRIP_APP_NAME:-}"
if strip_is_unset "$name"; then
  name="$(awk -F ' = ' '/^STRIP_APP_NAME / {print $2}' "$root/app/Brand.xcconfig")"
fi
cp "$bin/strip-engine" "$dest/strip-engine"
# The app executable is named from APP_NAME. On a case-insensitive volume that
# name and the CLI name "strip" are one directory entry, so the CLI is stored
# as strip-cli and the install command exposes it as "strip".
cli_dest="$dest/strip"
if [[ "$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]')" == "strip" ]]; then
  cli_dest="$dest/strip-cli"
fi
cp "$bin/strip" "$cli_dest"
chmod 755 "$dest/strip-engine" "$cli_dest"
cp "$root/docs/ACKNOWLEDGEMENTS.md" "$app/Contents/Resources/ACKNOWLEDGEMENTS.md"

agent="$app/Contents/Library/LaunchAgents/${bundle}.engine.plist"
cat > "$agent" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${bundle}.engine</string>
  <key>BundleProgram</key>
  <string>Contents/MacOS/strip-engine</string>
  <key>AssociatedBundleIdentifiers</key>
  <array>
    <string>${bundle}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>STRIP_APP_NAME</key>
    <string>${name}</string>
  </dict>
</dict>
</plist>
EOF

identity="${EXPANDED_CODE_SIGN_IDENTITY:--}"
if [[ -z "$identity" || "$identity" == *'{{'* ]]; then
  identity="-"
fi
codesign --force --sign "$identity" "$dest/strip-engine" "$cli_dest" >/dev/null
echo "engine copied to $dest"

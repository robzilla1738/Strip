#!/usr/bin/env bash
# Archive, sign, notarize, build the disk image, and write the appcast.
# DRY_RUN=1 lists every command, prints MISSING for each unset placeholder,
# and does not sign or notarize.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
cd "$root"
strip_load_developer_dir
strip_load_brand
strip_report_placeholders

name="$(strip_show APP_NAME "${APP_NAME:-}")"
team="$(strip_show APPLE_TEAM_ID "${APPLE_TEAM_ID:-}")"
identity="$(strip_show DEVELOPER_ID_APPLICATION "${DEVELOPER_ID_APPLICATION:-}")"
bundle="$(strip_show BUNDLE_ID "${BUNDLE_ID:-}")"
version="1.0.0"
if [[ "${GITHUB_REF:-}" == refs/tags/v* ]]; then
  version="${GITHUB_REF#refs/tags/v}"
fi
build_num="${GITHUB_RUN_NUMBER:-1}"
app="$root/build/${name}.app"
archive="$root/build/${name}.xcarchive"

echo "would run: (cd engine && cargo build --release --target aarch64-apple-darwin -p strip-engine -p strip-cli)"
echo "would run: scripts/gen-brand.sh"
echo "would run: (cd app && xcodegen generate)"
echo "would run: xcodebuild -project app/Strip.xcodeproj -scheme Strip -configuration Release -archivePath \"$archive\" archive DEVELOPMENT_TEAM=\"$team\" CODE_SIGN_IDENTITY=\"$identity\" PRODUCT_BUNDLE_IDENTIFIER=\"$bundle\" ENABLE_HARDENED_RUNTIME=YES MARKETING_VERSION=\"$version\" CURRENT_PROJECT_VERSION=\"$build_num\""
echo "would run: xcodebuild -exportArchive -archivePath \"$archive\" -exportOptionsPlist \"$root/build/ExportOptions.plist\" -exportPath \"$root/build\""

archive_ready=1
if strip_is_unset "${APPLE_TEAM_ID:-}" || strip_is_unset "${DEVELOPER_ID_APPLICATION:-}" || strip_is_unset "${BUNDLE_ID:-}"; then
  echo "SKIPPED (placeholder): archive and export"
  archive_ready=0
fi

dmg="$root/build/${name}.dmg"
user_dry=0
if strip_dry_run; then
  user_dry=1
fi
# The first pass only lists commands. Execution happens below when credentials exist.
DRY_RUN=1 "$root/scripts/sign.sh" "$app"
DRY_RUN=1 "$root/scripts/notarize.sh" "$app"
DRY_RUN=1 "$root/scripts/make-dmg.sh" "$app"
DRY_RUN=1 "$root/scripts/notarize.sh" "$dmg"
echo "would run: spctl -a -vvv -t exec \"$app\""
echo "would run: spctl -a -vvv -t open --context context:primary-signature \"$dmg\""
echo "would run: xcrun stapler validate \"$dmg\""
DRY_RUN=1 "$root/scripts/appcast.sh"

if [[ "$user_dry" == "1" || "$archive_ready" == "0" ]]; then
  exit 0
fi
export DRY_RUN=0

(cd "$root/engine" && cargo build --release --target aarch64-apple-darwin -p strip-engine -p strip-cli)
"$root/scripts/gen-brand.sh"
(cd "$root/app" && xcodegen generate)
mkdir -p "$root/build"
cat > "$root/build/ExportOptions.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key>
  <string>developer-id</string>
  <key>teamID</key>
  <string>${APPLE_TEAM_ID}</string>
  <key>signingStyle</key>
  <string>manual</string>
  <key>signingCertificate</key>
  <string>Developer ID Application</string>
</dict>
</plist>
EOF
xcodebuild -project "$root/app/Strip.xcodeproj" -scheme Strip -configuration Release \
  -archivePath "$archive" archive \
  DEVELOPMENT_TEAM="$APPLE_TEAM_ID" CODE_SIGN_IDENTITY="$DEVELOPER_ID_APPLICATION" \
  PRODUCT_BUNDLE_IDENTIFIER="$BUNDLE_ID" ENABLE_HARDENED_RUNTIME=YES \
  MARKETING_VERSION="$version" CURRENT_PROJECT_VERSION="$build_num"
xcodebuild -exportArchive -archivePath "$archive" \
  -exportOptionsPlist "$root/build/ExportOptions.plist" -exportPath "$root/build"
DRY_RUN=0 "$root/scripts/sign.sh" "$app"
DRY_RUN=0 "$root/scripts/notarize.sh" "$app"
DRY_RUN=0 "$root/scripts/make-dmg.sh" "$app"
DRY_RUN=0 "$root/scripts/notarize.sh" "$dmg"
spctl -a -vvv -t exec "$app"
spctl -a -vvv -t open --context context:primary-signature "$dmg"
xcrun stapler validate "$dmg"
DRY_RUN=0 "$root/scripts/appcast.sh"

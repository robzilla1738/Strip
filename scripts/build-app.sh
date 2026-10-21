#!/usr/bin/env bash
# Generate the Xcode project and build the Debug app. Release signing stays in
# the release scripts.
#
# An ad-hoc signature changes with every build, and macOS ties the
# Accessibility grant to the signature, so each rebuild lost the grant. When a
# development certificate is in the keychain, Debug builds sign with it so the
# grant survives rebuilds. STRIP_DEV_IDENTITY overrides the choice; "-" forces
# ad-hoc signing.
set -euo pipefail
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
strip_load_developer_dir
"$root/scripts/gen-brand.sh"
python3 "$root/scripts/make-placeholder-icon.py"
(cd "$root/app" && xcodegen generate)
identity="${STRIP_DEV_IDENTITY:-}"
if [[ -z "$identity" ]]; then
  identity="$(security find-identity -v -p codesigning 2>/dev/null | awk -F '"' '/"Apple Development: / {print $2; exit}')"
fi
signing=()
if [[ -n "$identity" && "$identity" != "-" ]]; then
  team="$(security find-certificate -c "$identity" -p 2>/dev/null | openssl x509 -noout -subject 2>/dev/null | sed -n 's/.*OU *= *\([A-Z0-9]*\).*/\1/p')"
  signing=(CODE_SIGN_STYLE=Manual "CODE_SIGN_IDENTITY=$identity" "DEVELOPMENT_TEAM=$team")
  echo "signing Debug build with $identity"
fi
xcodebuild \
  -project "$root/app/Strip.xcodeproj" \
  -scheme Strip \
  -configuration Debug \
  -destination 'platform=macOS,arch=arm64' \
  "${signing[@]}" \
  build

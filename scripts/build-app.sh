#!/usr/bin/env bash
# Generate the Xcode project and build the Debug app. Signing identities stay
# in the release scripts.
set -euo pipefail
source "$(dirname "$0")/lib.sh"
root="$(strip_root)"
strip_load_developer_dir
"$root/scripts/gen-brand.sh"
python3 "$root/scripts/make-placeholder-icon.py"
(cd "$root/app" && xcodegen generate)
xcodebuild \
  -project "$root/app/Strip.xcodeproj" \
  -scheme Strip \
  -configuration Debug \
  -destination 'platform=macOS,arch=arm64' \
  build

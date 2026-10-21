# Decisions

Choices the spec leaves open, and how this tree resolves them.

## Fork layout

Paneru at `98781431b09ebe44ef226b766ca2e82fddad9728` is vendored unmodified at `engine/upstream` (git subtree, squashed). The shipping workspace does not compile that tree. Paneru's Bevy app threads Lua and virtual-workspace rows through the ECS, and Step 2 requires `rg mlua|lua` to be empty under `engine/crates`. The engine in `engine/crates` implements Sections 1 and 2 directly and keeps Paneru's MIT license and `engine/NOTICE.md`.

## Debug bundle identifier

`config/brand.env` keeps `BUNDLE_ID={{BUNDLE_ID}}`. `scripts/gen-brand.sh` writes `app/Brand.xcconfig`. When the bundle id is still a placeholder, the xcconfig uses `local.preview.strip` so Debug `xcodebuild test` can sign ad-hoc. That temporary id is not written to `config/release.env`. Release signing prints `MISSING: {{BUNDLE_ID}}` and skips until a real id is set.

## IPC client

The Swift app talks to the engine with `Network.framework` `NWConnection` over a Unix-domain endpoint (`app/Strip/Engine/IpcClient.swift`). Line framing matches the Rust codec.

## Product strings

User-visible name, bundle id, and URL scheme come from `config/brand.env` through the generated xcconfig and Info.plist (`$(STRIP_APP_NAME)`, `$(STRIP_URL_SCHEME)`, `$(STRIP_BUNDLE_ID)`). The fixed binary names `strip-engine` and `strip` are the Section 2.1 process names, not the product name.

## Focus index 9

Section 1.3 binds ⌃⌥9 and ⌃⌥⇧9 to `focus_index` / `move_to_index` with "9 = last". The command argument `n` is 1-based, and `n = 9` means the last column. Other values are clamped to the column list.

## Column indices on the wire

`focused_column` in State and `column` on focus events are 0-based. `focus_index` arguments stay 1-based, so `focus_index 3` focuses column 2.

## Materials

The strip indicator uses `NSVisualEffectView` material `.hudWindow` on a borderless non-activating panel, as Section 1.5 requires for the macOS 13 deployment target.

## Fling deceleration

`swipe_deceleration` is an exponential decay rate in 1/seconds: `v *= exp(-swipe_deceleration * dt)`.

## Enhanced UI clock

The guard restores `AXEnhancedUserInterface` 250 ms after the last write to that pid, using the monotonic clock. Tests drive the same function with an injected `Instant`.

## Keychain in unit tests

License tests use the shipped `LicenseController` against an in-memory `SecretStore`. The app binds that protocol to the Keychain. Tests do not prompt for Keychain UI.

## Definition-of-done search roots

`scripts/dod.sh` searches shipped sources (`engine/crates`, `app`, `tools`, `integrations`, `scripts`). It does not flag `TODO` comments inside the unmodified `engine/upstream` snapshot.

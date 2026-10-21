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

`swipe_deceleration` is an exponential decay rate in 1/seconds: `v *= exp(-swipe_deceleration * dt)`. Velocity samples blend with α = 0.3. The 1:1 factor uses the visible-frame width. Past either end of the strip, extra travel is multiplied by 0.3, and lift snaps to the column whose center is nearest the viewport center.

## Enhanced UI clock

The guard restores `AXEnhancedUserInterface` 250 ms after the last write to that pid, using the monotonic clock. Tests drive the same function with an injected `Instant`.

## Keychain in unit tests

License tests use the shipped `LicenseController` against an in-memory `SecretStore`. The app binds that protocol to the Keychain. Tests do not prompt for Keychain UI.

## Session filter

`STRIP_ONLY_OWNER` limits the engine to windows whose owner name matches. The on-device script sets it to `TestWindows`. Unset, the engine manages regular and accessory apps. The engine never reads this as a config key.

## Parking on macOS 27

The layout asks for a parked origin of `(maxX − s, maxY − s)` with the column size. Accessibility applies that frame for a borderless resizable window: the on-display overlap is 5×5 pt. A titled window is clamped so its title bar stays inside the visible frame (measured top at y ≈ 995 on this display), which exposes about 5×122 pt. `tools/TestWindows` opens borderless resizable windows, marks them `AXStandardWindow`, and keeps `--dialog` panels titled. The engine does not special-case the fixture. It always publishes the layout frame.

## Signals and panic

The SIGTERM and SIGINT handler only sets an atomic and writes one byte to a self-pipe. The main thread un-parks. The panic hook calls the same un-park function on the panicking thread, because the process is already unwinding and will not return to the main loop.

## Accessibility imports

`ax.rs` allows `deprecated` and `unexpected_cfgs`. The `cocoa` 0.26 API is itself deprecated and emits those warnings. SkyLight and `_AXUIElementGetWindow` stay behind `dlsym` in `private_api`.

## Synthetic hotkeys

The event tap is `kCGHIDEventTap` at the head of the HID stream, as Section 2.7 specifies. On this OS a freshly compiled helper is not in the PostEvent privacy list, so its `CGEvent.post` stays inside that process. `scripts/e2e.sh` runs `tools/post-key.swift` with the `swift` interpreter, which is allowed to post, and that script calls `CGEvent.post(tap: .cghidEventTap)`.

## Command thread

IPC `hello`, `query`, and `subscribe` run on the connection thread. `command`, `set_managing`, `reload_config`, and `shutdown` are posted to a condvar mailbox and applied on the main thread before the result line is written. A focus action emits a `focus` event even when the column index does not change. `toggle_float` and `send_next_display` are known actions and return `ok` without changing the model until the step 9 behaviours land, so a later tick cannot disagree with a float flag that does not exist yet.

## Animation driver

The spring integrates with semi-implicit Euler. When ζ is 1 and a step would cross the target, the step snaps to the target and zeroes velocity so a critically damped spring cannot overshoot.

Ticks come from `CVDisplayLink` on each display that has a moving strip. The output callback records the host-time interval, schedules `CFRunLoopPerformBlock` on the main run loop, and wakes that loop. The main thread then steps the springs. `NSScreen.displayLink` is the macOS 14 selector API and cannot call the Rust stepper directly; deployment stays at macOS 13, so CVDisplayLink is the driver on every supported release. Windows layout classifies as hidden stay parked once. A full-size column kept just outside the viewport would paint more than the parking sliver, which the on-device parking check rejects.

## Definition-of-done search roots

`scripts/dod.sh` searches shipped sources (`engine/crates`, `app`, `tools`, `integrations`, `scripts`). It does not flag `TODO` comments inside the unmodified `engine/upstream` snapshot.

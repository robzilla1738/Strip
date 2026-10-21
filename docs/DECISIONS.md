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

IPC `hello`, `query`, and `subscribe` run on the connection thread. `command`, `set_managing`, `reload_config`, and `shutdown` are posted to a condvar mailbox and applied on the main thread before the result line is written. A focus action emits a `focus` event even when the column index does not change. `toggle_float` removes the focused window from its strip and keeps that frame. Later ticks do not insert it again. `send_next_display` moves it onto the next `NSScreen` and focuses it there. With one screen, or when the target UUID is in `ignored_displays`, the command logs `multi-display skipped` and leaves the strips alone.

## Animation driver

The spring integrates with semi-implicit Euler. When ζ is 1 and a step would cross the target, the step snaps to the target and zeroes velocity so a critically damped spring cannot overshoot.

Ticks come from `CVDisplayLink` on each display that has a moving strip. The output callback records the host-time interval, schedules `CFRunLoopPerformBlock` on the main run loop, and wakes that loop. The main thread then steps the springs. `NSScreen.displayLink` is the macOS 14 selector API and cannot call the Rust stepper directly; deployment stays at macOS 13, so CVDisplayLink is the driver on every supported release. Windows layout classifies as hidden stay parked once. A full-size column kept just outside the viewport would paint more than the parking sliver, which the on-device parking check rejects.

## Float admission

The on-screen list keeps Core Graphics window levels 0 through 8. Level 0 is a normal window. Levels 3 and 8 are floating and modal panels, which is where a dialog lives. Higher levels are menus and status items and stay out of the list. A window floats when a matching rule sets `floating = true`, when its subrole is present and is not `AXStandardWindow`, when `AXSize` is not settable or min size equals max size, when the bundle is Quick Look, or when the window belongs to the engine process. An empty bundle id is matched as the process owner name, so an unpackaged tool can use `bundle_id = "TestWindows"`. Slot rules apply only after those checks decline to float. Fullscreen windows stay out of every strip until `AXFullScreen` clears, then return at the saved index.

## Session restore and wake

`state.json` is the query snapshot written on each tick and on exit. Launch matches windows for 10 seconds with `restore::match_windows`: `(CGWindowID, pid)` first, then a unique `(bundle, title, role, subrole)`. An ambiguous title matches nothing. Restored columns keep their saved width and are placed without a spring.

Wake and unlock notifications set a flag. The main thread, and `strip debug simulate_wake`, wait 1.5 seconds, re-read the active Space, re-enumerate windows, and re-apply frames with the springs snapped. The existing event tap is enabled again. A second tap is not created, because it would swallow each key twice. Strips stay keyed by display UUID. The Space id from SkyLight is stored on the session. When SkyLight is missing, `fallback_space_key` is the hash used for that id.

## Definition-of-done search roots

`scripts/dod.sh` searches shipped sources (`engine/crates`, `app`, `tools`, `integrations`, `scripts`). It does not flag `TODO` comments inside the unmodified `engine/upstream` snapshot.

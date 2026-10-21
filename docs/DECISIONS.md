# Decisions

Choices the spec leaves open, and how this tree resolves them.

## Fork layout

Paneru at `98781431b09ebe44ef226b766ca2e82fddad9728` is vendored unmodified at `engine/upstream` (git subtree, squashed). The shipping workspace does not compile that tree. Paneru's Bevy app threads Lua and virtual-workspace rows through the ECS, and Step 2 requires `rg mlua|lua` to be empty under `engine/crates`. The engine in `engine/crates` implements Sections 1 and 2 directly and keeps Paneru's MIT license and `engine/NOTICE.md`.

## Debug bundle identifier

`config/brand.env` keeps `BUNDLE_ID={{BUNDLE_ID}}`. `scripts/gen-brand.sh` writes `app/Brand.xcconfig`. When the bundle id is still a placeholder, the xcconfig uses `local.preview.strip` so Debug `xcodebuild test` can sign ad-hoc. That temporary id is not written to `config/release.env`. Release signing prints `MISSING: {{BUNDLE_ID}}` and skips until a real id is set.

## IPC client

The Swift app talks to the engine with `Network.framework` `NWConnection` over a Unix-domain endpoint (`app/Strip/Engine/IpcClient.swift`). Line framing matches the Rust codec.

A nil `transportProtocol` fails on this OS (`nw_endpoint_handler_set_protocol_instance called with null protocol_parameters`). `NWParameters(tls: nil, tcp:)` — the same stack as `NWParameters.tcp` — reaches `.ready` on `NWEndpoint.unix` and exchanges the engine hello as a raw byte stream. No TCP header was observed on the line. TCP options set `noDelay` so a command line is not delayed.

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

A `--dialog` fixture is an `NSPanel` with the utility style. On this OS that panel is window level 3 and reports `AXFloatingWindow`, which is not `AXStandardWindow`, so admission floats it. `NSPanel` hides itself when its process deactivates. The fixture sets `hidesOnDeactivate` to false so the panel stays on screen while the other TestWindows processes are active. The engine does not special-case that flag.

## Session restore and wake

`state.json` is the query snapshot written on each tick and on exit. Launch matches windows for 10 seconds with `restore::match_windows`: `(CGWindowID, pid)` first, then a unique `(bundle, title, role, subrole)`. An ambiguous title matches nothing. Restored columns keep their saved width and are placed without a spring.

Wake and unlock notifications set a flag. `NSWorkspaceDidWakeNotification` and `NSWorkspaceScreensDidWakeNotification` are observed on `NSWorkspace.notificationCenter`. `com.apple.screenIsUnlocked` stays on the distributed center. `CGDisplayRegisterReconfigurationCallback` requests the same resync and ignores the begin flag. The main thread, and `strip debug simulate_wake`, wait 1.5 seconds, re-enumerate windows, and re-apply frames with the springs snapped. When SkyLight is loaded the active Space is read again. When it is missing, wake keeps the fallback hash from `switch_space` so the strip key does not become `1`. The existing event tap is enabled again. A second tap is not created, because it would swallow each key twice. A Space change does not take that 1.5 second path. `NSWorkspaceActiveSpaceDidChangeNotification` calls `switch_space`.

Strips are keyed by `(display UUID, Space id)` when SkyLight is loaded. The active Space comes from `SLSGetActiveSpace`. On-screen windows stay on that Space. Off-screen windows are assigned with `SLSCopySpacesForWindows` and `SLSCopyWindowsWithOptionsAndTags`. An empty Accessibility role is resolved with `_AXUIElementCreateWithRemoteToken`. When that constructor matches the window and the role is still empty, the window is admitted as a standard window and inserted on its Space. A fullscreen Space (type 4) stays out of the strips. Without SkyLight the key is display-only until `activeSpaceDidChange`, which then stores `fallback_space_key` and does not rehash on every tick. Wake does not call `current_space()` on that path, so the hash is not replaced with `1`.

Focus commands call `AXRaise` and `AXFrontmost`. If that fails, `_SLPSSetFrontProcessWithOptions` and `SLPSPostEventRecordTo` run, then the process is activated. The writer feeds each size read-back to `observe_size`. Two identical misses store `size_override`. After a park, a read-back whose center is on another display calls `repair_relocated`, republishes with `s = 1` at the opposite corner, and marks that side as a neighbour for the session. The next `repair_park` returns that 1 pt frame. It does not return `None` and let the 5 pt layout through. Removing a window passes that display's real viewport into `reveal`.

## App private symbols

`app/Strip/App/PrivateAPI.swift` is the only Swift call site of `dlsym`. The app's checks use public APIs: Accessibility, preferences, IOKit, and `NSScreen`. No private symbol is required, so the startup check's required list is empty and nothing is missing. SkyLight and `_AXUIElementGetWindow` stay in the engine's `private_api` module.

## LaunchAgent paths

When `--socket`, `--config`, `--state`, or `--log` is omitted, the engine fills the path from `STRIP_APP_NAME` under Application Support and Logs. The generated agent plist sets that variable from the brand xcconfig. A placeholder name prints `MISSING: {{APP_NAME}}` and leaves the flags unset. A missing config file loads defaults so the first launch still comes up.

## Unset trial length

An unset `TRIAL_DAYS` prints `MISSING: {{TRIAL_DAYS}}` and is stored as an empty build setting. The license state machine treats that empty value as license required, the same runtime state as `0`. It is not written into `config/release.env`.

## Headless owner filter

`--headless-test` sets `STRIP_ONLY_OWNER` when the parent environment did not. The supervised engine started by that test does not move unrelated windows. A normal launch does not set the variable.

## CLI filename on case-insensitive volumes

Section 2.1 places the Rust CLI at `Contents/MacOS/strip` next to the app executable `Contents/MacOS/{{APP_NAME}}`. With `APP_NAME=Strip` those paths are the same directory entry on the default case-insensitive APFS volume, and copying the CLI replaces the app. The bundle stores that binary at `Contents/MacOS/strip-cli`. Install Command Line Tool symlinks `strip-cli` to `/usr/local/bin/strip` when `strip` is not a distinct file. A brand whose name does not collide still copies the CLI to `Contents/MacOS/strip`.

## Xcode target name

The Xcode project, scheme, and `app/Strip` directory keep the Section 2.2 names so `xcodebuild -scheme Strip` matches the spec. User-visible strings, the bundle id, and the URL scheme still come from `config/brand.env`.

## Config hot reload

The engine watches the config file's parent directory, so a temp-file rename is observed. The callback signals a `CFRunLoopSource` and wakes the main run loop. A source that was signaled before the loop waits still fires, which a bare `CFRunLoopWakeUp` would miss. The main thread reloads and ticks in that turn, so a new `gap_inner` is in the next query without waiting out the 1 second idle slice. The settings window rewrites the Section 1.6 document. A GUI save does not keep unknown keys.

## Tile and tab clicks

`focus_window` focuses the column that owns the window and selects that window as `focused_window` and `active_tab`. The indicator and the tab bar both send that command. Dragging an indicator tile sends `move_column`.

## Mac Setup panes

Separate Spaces, rearrange Spaces, Stage Manager, and macOS 15 tiling all open `x-apple.systempreferences:com.apple.Desktop-Settings.extension`. The table gives that URL for Separate Spaces and names the Desktop & Dock pane for the other three. A missing preference that must be off counts as off. A missing swipe integer is not exactly 0, so that check fails. Tiling is omitted when the OS major version is below 15. Display arrangement only warns. Conflicting apps have no URL; Quit calls `terminate()`. The engine emits a `tour` event with action `swipe` once, when a real HID gesture arms. `inject_gesture` does not emit that event.

## URL scheme and an already-running engine

`strip://<action>?n=<int>` uses the host as the action id and `n` as the command argument. `STRIP_ADOPT_ENGINE=1` makes the app connect to `STRIP_SOCKET` and neither spawn nor kill that engine. That mode also skips the login-item side effect and the first-run window, so an on-device check can drive the engine the script already started. A second process with the same bundle id forwards the URL on a distributed notification and exits before starting an engine. The CLI fills a missing `--socket` from `STRIP_SOCKET`, then `STRIP_APP_NAME`, then `CFBundleName` in the enclosing app bundle. Install Command Line Tool links `strip-cli` when that file is present.

## Secret scan

`strings | rg -i "dodo.*key|sk_"` also matches three things that are not credentials. Sparkle's Norwegian string `automatisk_` contains `sk_`. An Info.plist key that puts `Dodo` on the same line as `</key>` matches `dodo.*key`, so the payments host is stored as `StripPayEnv`. Rust v0 symbol names in the engine contain the letters `sk_` (for example `NCINvMsk_`). The check scans the app executable, its debug dylib, the engine, the CLI, and `Info.plist`. It skips `Sparkle.framework` and it ignores a hit whose whole line is a Rust mangled symbol. Any other match fails.

## Invalid license

Section 4.6 drops a license as soon as validation returns `valid: false`. The step list says "after grace". The grace period applies only when the network itself fails: 30 days from `last_validated_at`. A negative validation does not wait.

## Branding scan

`Roadmap.md` is the spec, and its compliance rules name a commercial product this app must not copy. `scripts/check-branding.sh` excludes that file and `docs/DECISIONS.md`. Every other tracked path is scanned. A hit fails the check.

## Config watch license

`notify` 8.2 is CC0-1.0, which is not on the Section 6 allow-list. The engine watches the config directory with `fsevent-sys` (MIT) at latency 0. The callback still ignores an event whose config stamp (mtime and length) did not change, so a `state.json` write in that directory does not reload.

## Release dry-run

`DRY_RUN=1` prints each Section 4 command and does not run it. Unset values stay `{{NAME}}` in those lines. The scripts do not write a Team ID, a notarization ticket, or an EdDSA signature. Archive, codesign, notarization, stapling, and the Sparkle appcast each print `SKIPPED (placeholder)` when the values they need are unset, then exit 0.

## Definition-of-done search roots

`scripts/dod.sh` searches shipped sources (`engine/crates`, `app`, `tools`, `integrations`, `scripts`). It does not flag `TODO` comments inside the unmodified `engine/upstream` snapshot.

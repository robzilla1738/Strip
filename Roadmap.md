# Strip — one-shot build spec

*A native scrolling window manager for macOS: SwiftUI app + Rust engine (forked from Paneru, MIT). This spec is written for a coding agent to execute end to end. "Strip" is a placeholder name; change it in `config/brand.env`.*

---

## 0. Master prompt (paste into the coding agent)

> You are building **Strip**, a native scrolling window manager for macOS 13+ on Apple silicon: a SwiftUI app plus a Rust engine forked from **Paneru** (MIT). This file (`Roadmap.md`) is the source of truth.
>
> - Read it fully, then execute **Section 3 (Build Steps)** in order in a single run, without stopping or asking questions. Record any open choice in `docs/DECISIONS.md`.
> - After each step, run its **Acceptance checks**. Fix and re-run until green. Commit `step N: <title>`.
> - Values written `{{LIKE_THIS}}` are placeholders. Read them from `config/release.env` or the environment; never invent them. When one is missing, print `MISSING: {{NAME}}` and continue in dry-run mode.
> - Brand values (name, bundle ID, URL scheme) come only from `config/brand.env`.
> - Keep Paneru's MIT notice. Copy nothing from ChainYourMac. Copy no GPL code.
> - Rust: clippy `-D warnings` clean and tests green. Swift: deployment target 13.0 and tests green. The Xcode project is generated with XcodeGen.
> - Finish by running `scripts/dod.sh` (Section 5). Fix all failures, then print the report, the artifact paths and the unset placeholders.

(The same prompt, expanded, is in `MASTER_PROMPT.md`.)

---

## 1. Product spec

### 1.1 Core model
- **Strip:** each managed window belongs to exactly one **column** on a horizontal **strip**. There is one strip per (display, macOS Space) pair. A strip has an ordered list of columns, a `scroll_x` offset (points), a focused column index, and optional `pinned_left` / `pinned_right` columns.
- **Column:** an ordered list of 1+ windows, a `width_ratio` (fraction of the display's visible width), a `display_mode` of `stacked` or `tabbed`, a `height_ratios` vector (stacked mode) and an `active_tab` index (tabbed mode).
- **Window:** CGWindowID, pid, bundle ID, AX element, title, role, subrole, `managed | floating | minimized | hidden | fullscreen`, `original_height`, and an optional `size_override` (from stubborn apps).
- **Invariant 1:** opening, closing or moving a window never changes another column's width.
- **Invariant 2:** no managed window crosses its display's edge.
- **Invariant 3:** every window appears in at most one strip.

### 1.2 Behaviours
| Area | Exact behaviour |
|---|---|
| New window | Insert as a new column immediately right of the focused column, at the first preset width ≥ 0.5 (`default_column_width`, default 0.5). Focus it and scroll it into view, unless its app is not frontmost (see No focus stealing). |
| Close window | Remove it. If its column empties, remove the column. Focus the column to the left; if none, the one to the right. Other widths are unchanged. |
| Scroll-into-view | After focus changes, set `scroll_x` to the smallest change that makes the focused column fully visible inside the viewport (visible frame minus `gap_outer`, minus pinned columns). Then apply the center mode. |
| Center modes | `never` (default): scroll-into-view only. `on_overflow`: center the focused column when the columns don't all fit in the viewport. `always`: always center the focused column. |
| Widths | Cycle `preset_column_widths` forward, wrapping (default `[0.25, 0.33, 0.5, 0.66, 0.75]`). Wider/narrower = ±`width_step` (0.10), clamped to [0.1, 1.0]. Full width toggles between 1.0 and the previous width. Center scrolls the focused column to the viewport center once, regardless of mode. |
| Heights | Stacked: windows share the column height by `height_ratios`. Taller/shorter = ±0.10 for the focused window, taken from or given to its neighbours, with each window ≥ 0.1. Equalize sets all ratios equal. |
| Stacks | Stack-into-left moves the focused window into the bottom of the left column. Stack-into-right moves it into the bottom of the right column. Pop-out-right / pop-out-left moves the focused window out of its stack into a new column immediately right / left of the current one, keeping the column's width. Move up/down inside a stack reorders. |
| Tabbed column | Toggles the column between stacked and tabbed. Tabbed: every window gets the full column height; only `active_tab` is on screen, and the others are hidden (Section 2.6). Focus up/down cycles tabs. A tab-bar overlay (Section 1.5) sits above the column; clicking a tab activates it. |
| Native macOS tabs | Windows of the same app that share a frame and form an AX tab group (one visible, siblings hidden at the same frame) are one column. Switching tabs, closing tabs and dragging a tab out work natively. A dragged-out tab becomes a new column to the right. |
| Pinning | Pin-left / pin-right moves the focused column to the left / right viewport edge at full height. The scrollable viewport shrinks by the pinned width + `gap_inner`. One pin per side. Pinning an occupied side returns the old pinned column to the strip at its previous index. Pressing the same key on the pinned column unpins it back to its previous index, clamped. |
| Float | Toggles the focused window between managed and floating. Floating windows keep their frame and are never moved by the engine. Unfloating inserts the window right of the focused column. |
| Auto-float | Float automatically when any of these hold: subrole ≠ `AXStandardWindow` (dialogs, sheets, `AXFloatingWindow`, `AXSystemDialog`); `kAXSizeAttribute` not settable, or min size == max size (fixed-size windows, e.g. Calculator); bundle ID is Quick Look (`com.apple.quicklook.*`, `com.apple.QuickLookUIFramework*`); the window belongs to Strip itself. |
| Rules | `[[rules]]` match `bundle_id` (exact) and/or `title` (regex), with optional `role` / `subrole`. Actions: `floating = true`, or `slot = N` (insert at 1-based column index N instead of next to focus). The first matching rule wins. Rules apply when a window is first managed. |
| Keep Original Height | Option `keep_original_height` (off). When on, height = min(`original_height`, viewport height), top-aligned. |
| Stubborn apps | After every AX size write, read the size back. If it differs by >2 pt on two consecutive writes with the same target, store `size_override = actual`, lay out using the actual size, and stop re-requesting that size until the user changes that column's width or height. |
| No focus stealing | If a new window's pid ≠ the frontmost app's pid, insert it right of the focused column without changing focus or `scroll_x`. |
| Fullscreen | Windows with `AXFullScreen = true`, or on a fullscreen-type Space, are `fullscreen`: never moved, never in a strip. When fullscreen ends, the window returns to its previous column index. |
| Minimize / hide app | The window leaves the strip (column removed, or the window removed from its stack). On restore, it is reinserted at its previous index, clamped. |
| Drag to reorder | Dragging a managed window by its title bar shows no ghost. On mouse-up, if the cursor is over another column, move the dragged window's column to that index (to the column's left half = before it, right half = after it), then re-layout. Dropping on another display moves the window into that display's strip at the drop index. |
| Focus follows mouse | Option `focus_follows_mouse` (off). Focus the managed window under the cursor after 100 ms of rest. Never scroll the strip. |
| Mouse follows focus | Option `mouse_follows_focus` (off). On keyboard focus change, warp the cursor to the focused window's center. |
| Multi-display | A strip per display, with independent focus and `scroll_x`. Send-to-next-display (ordered by `NSScreen.screens`) appends the window as a column right of the focused column on the target display, and focus follows. Per-display off switch (`ignored_displays`, a list of display UUIDs): windows there are never moved. |
| Spaces | A strip per (display UUID, Space ID). Switching Spaces activates that Space's strip. A window moved to another Space (via Mission Control) leaves its old strip and is inserted into the new one at the end. |
| Session restore | Persist all strips (window identity, column index, stack position, width, height ratio, display mode, pin) to `state.json` every 2 s when dirty, and on exit. On launch, for 10 s, match reappearing windows by (CGWindowID, pid), then (bundle ID, title, role, subrole), and rebuild columns in saved order without animation. Unmatched windows follow normal placement. |
| Sleep, lock, wake | On `NSWorkspace.didWakeNotification`, `screensDidWakeNotification`, `com.apple.screenIsUnlocked` and display reconfiguration: wait 1.5 s, re-enumerate displays, Spaces and windows, re-create the event tap and AX observers, and re-apply the layout without animation. |
| Quit / stop | Stop (⌃⌥Q) and app quit both restore every hidden window to a visible on-screen frame (its last on-screen frame, or cascaded inside the visible frame), then stop managing. SIGTERM, SIGINT and engine panic do the same (panic hook + signal handler). |

### 1.3 Default shortcuts (action IDs in parentheses)
All defaults use ⌃⌥. Every action accepts multiple bindings. The recorder stores keycode + modifiers and displays keys using the current keyboard layout.

| Keys | Action |
|---|---|
| ⌃⌥H / ⌃⌥← | Focus left (`focus_left`) |
| ⌃⌥L / ⌃⌥→ | Focus right (`focus_right`) |
| ⌃⌥K / ⌃⌥↑ | Focus up in stack / previous tab (`focus_up`) |
| ⌃⌥J / ⌃⌥↓ | Focus down in stack / next tab (`focus_down`) |
| ⌃⌥⌘← / ⌃⌥⌘→ | Focus first / last (`focus_first`, `focus_last`) |
| ⌃⌥1 … ⌃⌥8, ⌃⌥9 | Focus column 1–8, 9 = last (`focus_index`) |
| ⌃⌥⇧1 … ⌃⌥⇧9 | Move focused column to index 1–8, 9 = last (`move_to_index`) |
| ⌃⌥⇧H/L, ⌃⌥⇧←/→ | Move column left / right (`move_left`, `move_right`) |
| ⌃⌥⇧K/J, ⌃⌥⇧↑/↓ | Move window up / down in stack (`move_up`, `move_down`) |
| ⌃⌥⌘⇧← / → | Move column to first / last (`move_first`, `move_last`) |
| ⌃⌥R | Cycle preset width (`cycle_width`) |
| ⌃⌥F | Full width (`full_width`) |
| ⌃⌥C | Center column (`center`) |
| ⌃⌥= / ⌃⌥- | Wider / narrower (`wider`, `narrower`) |
| ⌃⌥⇧= / ⌃⌥⇧- | Taller / shorter (`taller`, `shorter`) |
| ⌃⌥0 | Equalize heights (`equalize_heights`) |
| ⌃⌥] | Stack into left column (`stack_left`) |
| ⌃⌥[ | Stack into right column (`stack_right`) |
| ⌃⌥⇧] | Pop out to the right (`unstack_right`) |
| ⌃⌥⇧[ | Pop out to the left (`unstack_left`) |
| ⌃⌥⇧T | Toggle tabbed column (`toggle_tabbed`) |
| ⌃⌥T | Float / unfloat (`toggle_float`) |
| ⌃⌥N | Send window to next display (`send_next_display`) |
| ⌃⌥P / ⌃⌥⇧P | Pin left / pin right, toggle (`pin_left`, `pin_right`) |
| ⌃⌥Q | Stop the window manager (`stop`) |

Shortcut sets are versioned (`shortcut_set_version = 1`). On upgrade, keep the user's bindings and add new default bindings only where the key combination is unused. The Shortcuts page has a "Use default set" button.

### 1.4 Trackpad gesture
- Horizontal swipe with `swipe_fingers` fingers (default 4; allowed 3, 4, 5; 0 = off). `swipe_natural` (true) sets the direction.
- The strip follows the fingers 1:1: Δ`scroll_x` = −Δ(mean normalized x of the touches) × trackpad-to-points factor `swipe_sensitivity` × viewport width (default 1.0).
- On lift, fling with the release velocity, decelerating at `swipe_deceleration` (4.0/s), then snap with the spring to the column whose center is nearest the viewport center, and focus it.
- Reversing direction mid-swipe keeps tracking. Past the first/last column, rubber-band (resistance 0.3, spring back).
- Windows entering from off-screen are moved into place on the first gesture frame so they glide in with the strip.
- Vertical swipes are ignored (macOS keeps them for Mission Control and App Exposé).

### 1.5 Visuals
- **Animation:** one spring for all strip motion. `animation_duration_ms` (500; 0 = instant) and `animation_damping` (0.9; 1.0 = no overshoot). Ticks are locked to the strip display's refresh rate. The first frame is written within the same event-loop turn as the keypress.
- **Strip indicator:**
  - A frosted, borderless, non-activating panel centered horizontally, 12 pt above the visible frame's bottom edge, on the display of the strip.
  - One 40 pt tile per column with the app icon; a stacked/tabbed column shows its first window's icon plus a small count badge. The focused tile is highlighted; pinned tiles show a pin glyph at their edge.
  - Appears on every navigation action and on swipe start. Fades out 1.2 s after the last action (fade 0.25 s). Stays visible while both ⌃ and ⌥ are held.
  - Click a tile → `focus_window`. Drag a tile → live reorder preview → `move_column(from, to)` on drop.
  - Option `show_indicator` (true).
- **Focus border:** option `focus_border` (off), `focus_border_width` (4), `focus_border_color` (`#4C8DFF`), `focus_border_radius` (10). A click-through overlay window that tracks the focused window's frame every animation frame.
- **Dim inactive:** option `dim_inactive` (off), `dim_inactive_opacity` (0.25). Dims all managed windows except the focused one.
- **Tab bar:** for tabbed columns, a 24 pt click-through-except-tabs overlay strip along the column top showing window titles. Click activates that tab.

### 1.6 Settings (`config.toml`; GUI writes it, engine hot-reloads it)
```toml
[general]
launch_at_login = true
start_managing_on_launch = true
show_in_dock = true
shortcut_set_version = 1

[layout]
preset_column_widths = [0.25, 0.33, 0.5, 0.66, 0.75]
default_column_width = 0.5
width_step = 0.10
gap_inner = 8            # 0–64 pt
gap_outer = 8            # 0–64 pt
dock_aware = true
center_focused_column = "never"   # never | on_overflow | always
keep_original_height = false

[animation]
animation_duration_ms = 500
animation_damping = 0.9

[swipe]
swipe_fingers = 4        # 0 | 3 | 4 | 5
swipe_natural = true
swipe_sensitivity = 1.0
swipe_deceleration = 4.0

[focus]
focus_follows_mouse = false
mouse_follows_focus = false
drag_to_reorder = true

[appearance]
show_indicator = true
focus_border = false
focus_border_width = 4
focus_border_color = "#4C8DFF"
focus_border_radius = 10
dim_inactive = false
dim_inactive_opacity = 0.25

[displays]
ignored_displays = []    # display UUID strings

[[rules]]
bundle_id = "com.apple.systempreferences"
floating = true

[bindings]
focus_left = ["ctrl+alt-h", "ctrl+alt-left"]
# … every action ID from 1.3, each a list of "mods-key" strings
```
Paths:
- Config: `~/Library/Application Support/{{APP_NAME}}/config.toml`.
- State: `…/state.json`.
- Socket: `…/engine.sock`.
- Logs: `~/Library/Logs/{{APP_NAME}}/engine.log` (rotating 5 MB × 3).

### 1.7 App (SwiftUI)
- **Menu bar extra** with:
  - a status line (Running / Stopped / Needs Accessibility);
  - Start, Stop, Restart Engine;
  - Open {{APP_NAME}}…, Check for Updates…;
  - Help ▸ Setup Guide, Reveal Engine Log, Reveal Config File;
  - About (with Acknowledgements), Quit.
- **Main window** (settings) with sidebar sections. Every control writes config immediately; there is no Save button.
  - **General:** launch at login, start on launch, show in Dock, license.
  - **Layout:** widths editor, default width, gaps, Dock-aware, center mode, keep original height.
  - **Trackpad:** fingers, direction, sensitivity, plus an inline conflict warning from Mac Setup.
  - **Appearance:** indicator, border, dim.
  - **Displays:** list of connected displays with a manage toggle.
  - **Rules:** list, plus an "Add rule…" app picker (running apps + /Applications) with float / slot N.
  - **Shortcuts:** grouped table of actions, each with multiple recorder fields, conflict highlighting, and "Use default set".
  - **Mac Setup:** checks with Fix buttons.
  - **About.**
- **First run:**
  1. Welcome.
  2. Accessibility grant (live status).
  3. Mac Setup checks.
  4. Interactive tour of 5 shortcuts: focus left/right, move, cycle width, stack, swipe. Each step waits until the user performs the action, detected through engine events.
  5. Done.
- **Automation:**
  - URL scheme `{{URL_SCHEME}}://<action_id>?n=<int>` (e.g. `strip://focus_index?n=3`, `strip://reload_config`).
  - CLI `strip <action_id> [n]`, `strip query state|displays|metrics --json`, `strip subscribe --json`.
  - Raycast script commands in `integrations/raycast/` (one per common action).
- **Updates:** Sparkle, automatic check daily, user-initiated check in the menu.
- **License:** Section 4.6.

---

## 2. Architecture, repo layout, IPC, platform requirements

### 2.1 Processes
```
{{APP_NAME}}.app
├─ Contents/MacOS/{{APP_NAME}}      SwiftUI app: UI, overlays (indicator, tab bar), supervisor, licensing, Sparkle
├─ Contents/MacOS/strip-engine      Rust engine (Paneru fork): AX, event tap, layout, animation, focus border, dim
├─ Contents/MacOS/strip             Rust CLI (same IPC as the app)
└─ Contents/Frameworks/Sparkle.framework
```
- The app launches the engine as a child process (`Process`, executable `Contents/MacOS/strip-engine`, args `--socket <path> --config <path> --state <path> --log <path>`). Because it is a child process, the engine inherits the app's Accessibility grant through TCC responsible-process attribution.
- Supervisor:
  - Restart on exit with backoff 0.5, 1, 2, 4, 8, 16, 30 s.
  - After 5 exits within 60 s, stop retrying and show "Engine stopped — Reveal Log / Restart".
  - On app quit, send `shutdown` and wait up to 2 s, then SIGTERM.
- Permission check: at startup, the engine sends `hello { ax_trusted }`. If the app is trusted and the engine reports `false`, switch to the fallback: register the engine with `SMAppService.agent(plistName:)` as `Contents/Library/LaunchAgents/{{BUNDLE_ID}}.engine.plist` (BundleProgram `Contents/MacOS/strip-engine`), and add a second onboarding row asking to enable the engine in the Accessibility list. Implement both paths; choose at runtime.

### 2.2 Repo layout
```
strip/
├─ config/brand.env               APP_NAME, BUNDLE_ID, URL_SCHEME, SUPPORT_EMAIL, WEBSITE
├─ config/release.env.example     every {{PLACEHOLDER}} from Section 4
├─ engine/                        Rust workspace (fork of karinushka/paneru @ 98781431b09ebe44ef226b766ca2e82fddad9728)
│  ├─ LICENSE.txt                 Paneru MIT (unchanged) + NOTICE.md naming the fork point
│  ├─ crates/strip-core/          pure model, layout, spring, gesture math (no macOS deps)
│  ├─ crates/strip-ipc/           protocol types (serde), socket server/client, framing
│  ├─ crates/strip-engine/        Paneru-derived platform + ECS + main (binary)
│  └─ crates/strip-cli/           CLI binary
├─ app/
│  ├─ project.yml                 XcodeGen spec
│  ├─ Strip/                      App.swift, MenuBar/, Settings/, Onboarding/, MacSetup/, Overlays/, Licensing/, Engine/ (supervisor + IPC client)
│  ├─ StripTests/                 unit tests (IPC codec, config round-trip, Mac Setup parsers, license state machine)
│  └─ Resources/                  Assets.xcassets (placeholder icon), Info.plist template, entitlements
├─ tools/TestWindows/             SwiftPM AppKit app that opens configurable test windows
├─ integrations/raycast/          script commands
├─ scripts/                       bootstrap.sh build-engine.sh build-app.sh e2e.sh sign.sh notarize.sh make-dmg.sh appcast.sh release.sh dod.sh
├─ docs/                          ARCHITECTURE.md DECISIONS.md TESTING.md ACKNOWLEDGEMENTS.md
└─ .github/workflows/             ci.yml release.yml (runs-on: macos-15)
```

### 2.3 IPC protocol
- **Transport:** Unix domain socket at `~/Library/Application Support/{{APP_NAME}}/engine.sock`, mode 0600. One UTF-8 JSON object per line (`\n`-terminated), max 1 MiB.
- **Version:** protocol version `1`. The first message on every connection is `hello` from the client; the server replies `hello` with its version and capabilities.

Client → engine:
```json
{"id": 1, "type": "hello", "client": "app|cli", "protocol": 1}
{"id": 2, "type": "command", "action": "focus_index", "args": {"n": 3}}
{"id": 3, "type": "command", "action": "focus_window", "args": {"window_id": 4711}}
{"id": 4, "type": "command", "action": "move_column", "args": {"display": "UUID", "from": 2, "to": 5}}
{"id": 5, "type": "query", "what": "state|displays|metrics|config_errors"}
{"id": 6, "type": "subscribe", "events": ["state", "focus", "indicator", "tour", "log"]}
{"id": 7, "type": "reload_config"}
{"id": 8, "type": "set_managing", "enabled": false}
{"id": 9, "type": "shutdown"}
```
Engine → client:
```json
{"id": 1, "type": "hello", "protocol": 1, "engine_version": "1.0.0", "ax_trusted": true, "private_api": {"skylight": true, "ax_get_window": true}}
{"id": 2, "type": "result", "ok": true}
{"id": 5, "type": "result", "ok": true, "data": { "...": "see State" }}
{"id": 2, "type": "result", "ok": false, "error": {"code": "no_focused_window", "message": "..."}}
{"type": "event", "event": "state", "data": { "...": "State" }}
{"type": "event", "event": "focus", "data": {"window_id": 4711, "display": "UUID", "column": 3}}
{"type": "event", "event": "indicator", "data": {"display": "UUID", "show": true, "hold": false}}
{"type": "event", "event": "tour", "data": {"action": "cycle_width"}}
```
**State** schema:
```json
{"managing": true,
 "displays": [{"uuid": "…", "name": "…", "frame": [x,y,w,h], "visible_frame": [x,y,w,h], "ignored": false,
   "active_space": 42,
   "strips": [{"space": 42, "scroll_x": 120.0, "focused_column": 1,
     "pinned_left": null, "pinned_right": 0,
     "columns": [{"index": 0, "width_ratio": 0.5, "mode": "stacked", "active_tab": 0,
       "windows": [{"id": 4711, "pid": 123, "bundle_id": "com.apple.Safari", "title": "…",
         "frame": [x,y,w,h], "target": [x,y,w,h], "visible": true, "height_ratio": 1.0}]}]}]}],
 "floating": [{"id": 99, "pid": 1, "bundle_id": "…"}]}
```
**Error codes:** `bad_request`, `unknown_action`, `no_focused_window`, `not_trusted`, `display_not_found`, `internal`.

The `action` values are exactly the action IDs in 1.3, plus `focus_window`, `move_column`, `stop`, `start`, `reload_config`.

### 2.4 Window discovery and events (engine)
- Enumerate windows with `NSWorkspace.runningApplications` (activationPolicy `.regular` or `.accessory`), then `AXUIElementCreateApplication(pid)` and `kAXWindowsAttribute`.
- Map AX element → CGWindowID with `_AXUIElementGetWindow` (dlsym from HIServices). Fallback: match `CGWindowListCopyWindowInfo` entries by pid + bounds.
- Get windows on other Spaces with `SLSCopyWindowsWithOptionsAndTags` and `_AXUIElementCreateWithRemoteToken`, as Paneru already does.
- Create one `AXObserver` per pid for: `kAXWindowCreatedNotification`, `kAXUIElementDestroyedNotification`, `kAXFocusedWindowChangedNotification`, `kAXMainWindowChangedNotification`, `kAXWindowMovedNotification`, `kAXWindowResizedNotification`, `kAXWindowMiniaturizedNotification`, `kAXWindowDeminiaturizedNotification`, `kAXApplicationHiddenNotification`, `kAXApplicationShownNotification`, `kAXTitleChangedNotification`. Add its run-loop source to the main run loop.
- Subscribe to `NSWorkspace` notifications: didLaunch / didTerminate / didActivateApplication, `activeSpaceDidChangeNotification`, willSleep / didWake, screensDidSleep / screensDidWake.
- Subscribe to `NSDistributedNotificationCenter` `com.apple.screenIsLocked` / `com.apple.screenIsUnlocked`.
- Register `CGDisplayRegisterReconfigurationCallback` for display changes.
- Set `AXUIElementSetMessagingTimeout(app, 0.25)` on every app element.

### 2.5 Moving and resizing (engine)
- Write with `AXUIElementSetAttributeValue` using `kAXSizeAttribute` and `kAXPositionAttribute` (`AXValueCreate(.cgSize/.cgPoint)`), in top-left-origin global coordinates. Convert from `NSScreen` with `y_ax = primary_screen_height − y_ns − h`.
- **Order for a resize + move:** size → position → size.
- **Animation frames write position only.** A size change is written once before the animation starts if the new size is larger, or once after it ends if smaller.
- **Enhanced UI:** before writing to a pid, read `AXEnhancedUserInterface` on the app element. If true, set false, write, and restore it 250 ms after the last write (Paneru's `EnhancedUiGuard`; keep it).
- **AX writer pool:**
  - One serial worker thread per pid.
  - The main thread publishes the latest target per window into a lock-free slot (`ArcSwap<Option<Frame>>`). The worker always writes the most recent target and drops stale ones.
  - Every write is timed and the read-back is recorded in metrics (`ax_write_ms` p50/p95/max per bundle ID).
  - AX element handles are retained CF objects, moved to workers as `Send` wrappers.
- **Read back** after the last frame of every animation. Apply the stubborn-app rule (1.2).

### 2.6 Off-screen hiding (engine)
- macOS clamps windows that are moved fully off a display, and moves them to an adjacent display if one lies in that direction. Implement exactly:
  1. **Partial off-screen** (column partly outside the viewport): place it at its true strip position, clipped at the display edge minus `sliver_width` (default 5 pt). Its frame may extend past the edge by at most `width − sliver_width`.
  2. **Fully hidden** (column entirely outside the viewport, inactive tab, or pinned-over area): park the window so only a `sliver_width` × `sliver_width` corner remains on its own display.
     - Corner choice: the bottom corner on the side the window is hidden to (left → bottom-left, right → bottom-right).
     - If a display touches that side edge (detect from `NSScreen.frame` adjacency: |edge − neighbour edge| < 1 and vertical overlap > 0), use the opposite bottom corner. If both sides have neighbours, use the bottom corner whose 1-pt-outside neighbourhood contains no display; if none qualifies, use bottom-left with `sliver_width = 1`.
     - Parked origin: bottom-right = (maxX − s, maxY − s); bottom-left = (minX − w + s, maxY − s), in AX coordinates of the display.
  3. With `gap_outer = 0`, use `s = 1`.
  4. After parking, read the frame back. If macOS relocated the window to another display (`SLSCopyManagedDisplayForWindow` or the frame's center is on another display), move it back with `s = 1` at the opposite corner and mark that side as "neighbour" for the session.
- Mac Setup recommends arranging extra displays above or below (Section 2.10).
- On quit, stop or crash, un-park every window (1.2 Quit).

### 2.7 Hotkeys and event tap (engine)
- Create one `CGEvent.tapCreate(tap: .cghidEventTap, place: .headInsertEventTap, options: .defaultTap, eventsOfInterest: keyDown | keyUp | flagsChanged | leftMouseDown | leftMouseUp | leftMouseDragged | mouseMoved | scrollWheel | gesture(29))` and add it to the main run loop. This needs Accessibility only.
- Key handling:
  - Match `keyDown` by (keycode, masked flags `ctrl|alt|shift|cmd`) against bindings. On a match, return `nil` (swallow) and dispatch the action synchronously on the main thread.
  - Handle `flagsChanged` to keep the indicator up while ⌃⌥ is held.
- Handle `tapDisabledByTimeout` / `tapDisabledByUserInput` by calling `CGEvent.tapEnable(tap, true)`. Also re-create the tap on wake.
- Keyboard layouts:
  - Bindings store keycodes.
  - Display names come from `TISCopyCurrentKeyboardLayoutInputSource` + `UCKeyTranslate` (app side).
  - The config string format is `"ctrl+alt-<keyname>"`. The key name is resolved to a keycode with the US-ANSI table at parse time; the recorder writes the keycode-derived name, so AZERTY, QWERTZ and Dvorak record what the user pressed.

### 2.8 Swipe (engine)
- In the same tap, for event type 29 (gesture), build `NSEvent(cgEvent:)` and read `allTouches()` → `NSTouch.normalizedPosition`, `identity` and `phase`. This is Paneru's `platform/input.rs`; keep it.
- When the touch count equals `swipe_fingers` and |Δx| > |Δy| × 1.5 over the first 8 pt of travel, begin a strip swipe. Otherwise ignore the gesture.
- Gesture events are observe-only: always pass them through, never swallow them.
- Math lives in `strip-core::gesture` (pure): anchor per touch identity, mean Δx, velocity via an exponential moving average (α = 0.3), fling + snap as in 1.4.
- Mac Setup (2.10) detects and fixes conflicting system trackpad settings.

### 2.9 Animation (engine)
- **Spring** (pure, `strip-core::spring`):
  - ω = 2π / (duration_s × 0.9), ζ = `animation_damping`.
  - Integrate with semi-implicit Euler, `dt = min(frame_dt, 1/30)`.
  - Done when |x − target| < 0.5 pt and |v| < 5 pt/s.
  - Retargeting mid-flight keeps the current velocity.
- **Ticks:**
  - `CVDisplayLinkCreateWithCGDisplay(display_id)` per display with an active animation; the output callback posts a tick to the main run loop (`CFRunLoopPerformBlock` + `CFRunLoopWakeUp`).
  - Stop the link when no animation is active (idle CPU ≈ 0).
  - On macOS 14+, prefer `NSScreen.displayLink(target:selector:)`, and fall back to CVDisplayLink.
- **Per frame:**
  - Compute targets for windows whose animated frame intersects the viewport (expanded by 1 column on each side).
  - Windows outside that range are parked once at the start (2.6), not animated.
  - Publish targets to the writer pool (2.5).
- First frame: the keypress handler computes the new target and publishes frame 0 immediately, before the next display-link tick.

### 2.10 Spaces and displays (engine)
- SkyLight via `dlsym` from `/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight`: `SLSMainConnectionID`, `SLSGetActiveSpace`, `SLSCopyManagedDisplaySpaces`, `SLSManagedDisplayGetCurrentSpace`, `SLSSpaceGetType` (type 4 = fullscreen), `SLSCopySpacesForWindows`, `SLSCopyManagedDisplayForWindow`, `SLSRegisterConnectionNotifyProc`.
- Strip key: (display UUID from `CGDisplayCreateUUIDFromDisplayID`, Space ID).
- **Fallback when any symbol is missing:** key strips by display UUID only. Treat `activeSpaceDidChangeNotification` as a strip switch keyed by a hash of the sorted on-screen CGWindowIDs (`CGWindowListCopyWindowInfo(.optionOnScreenOnly)`) for that display. Report `private_api.skylight=false` in `hello`.
- Use `NSScreen.visibleFrame`, which excludes the menu bar and the Dock when `dock_aware` is on; otherwise use `frame` minus the menu bar height. Apply `gap_outer` inside it.

### 2.11 Focus (engine)
- Focus = `AXUIElementPerformAction(window, kAXRaiseAction)` + `AXUIElementSetAttributeValue(app, kAXFrontmostAttribute, true)`.
- If that fails, use `_SLPSSetFrontProcessWithOptions` + `SLPSPostEventRecordTo` (Paneru's existing implementation). Then `NSRunningApplication(processIdentifier:).activate()`.

### 2.12 Mac Setup checks (app)
Read the values with `CFPreferencesCopyAppValue`, falling back to the `defaults read` output. Each failing check shows **Fix**, which opens the exact System Settings pane, plus one-line instructions.

| Check | Read | Pass when | Fix opens |
|---|---|---|---|
| Accessibility | `AXIsProcessTrusted()` + engine `hello.ax_trusted` | both true | `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility` |
| Swipe conflict (fingers = 4) | `com.apple.AppleMultitouchTrackpad` and `com.apple.driver.AppleBluetoothMultitouch.trackpad`: `TrackpadFourFingerHorizSwipeGesture` | 0 | `x-apple.systempreferences:com.apple.Trackpad-Settings.extension` ("Swipe between full-screen applications: Off") |
| Swipe conflict (fingers = 3) | `TrackpadThreeFingerHorizSwipeGesture`, `TrackpadThreeFingerDrag` | both 0 | Trackpad pane ("Swipe between full-screen applications: four fingers"; "Swipe between pages: two fingers"; Three Finger Drag off) |
| Separate Spaces | `com.apple.spaces` `spans-displays` | false/0 (= "Displays have separate Spaces" on) | `x-apple.systempreferences:com.apple.Desktop-Settings.extension` (log out after) |
| Rearrange Spaces | `com.apple.dock` `mru-spaces` | false | Desktop & Dock pane |
| Stage Manager | `com.apple.WindowManager` `GloballyEnabled` | false | Desktop & Dock pane |
| macOS tiling (15+) | `com.apple.WindowManager` `EnableTilingByEdgeDrag`, `EnableTopTilingByEdgeDrag`, `EnableTilingOptionAccelerator` | all false | Desktop & Dock pane |
| Display arrangement | `NSScreen.frame`s | no displays side by side (horizontal adjacency) | `x-apple.systempreferences:com.apple.Displays-Settings.extension` ("Arrange… put the display above or below") — warn only |
| Conflicting apps | `NSWorkspace.runningApplications` bundle IDs / names: Rectangle, Magnet, Loop, BetterSnapTool, OneMenu, AeroSpace, yabai, Amethyst, OmniWM, Nehir, Paneru, Moom, Divvy | none running | "Quit <name>" button (`terminate()`) |

Re-run the checks every 5 s while the Mac Setup page or onboarding is visible.

---

## 3. Build steps (execute in order; do not stop)

Every step ends with **Acceptance checks**. On-device checks run through `scripts/e2e.sh`. That script:
- builds `tools/TestWindows`;
- verifies `AXIsProcessTrusted()` for the host terminal through a tiny Swift helper. If it is false, the script opens the Accessibility pane and polls every 2 s until it is true;
- launches the engine with a temporary config;
- drives it via `strip` CLI commands;
- asserts on `strip query state --json` with `jq`.

### Step 1 — Bootstrap
- **Tasks:**
  - `git init`. Create the layout in 2.2, `config/brand.env`, and `config/release.env.example` (all placeholders from Section 4).
  - Write `scripts/bootstrap.sh`, which installs or verifies: Xcode 16+, `rustup` stable + `aarch64-apple-darwin`, `brew install xcodegen create-dmg jq`.
  - Vendor Paneru: `git subtree add --prefix engine/upstream https://github.com/karinushka/paneru 98781431b09ebe44ef226b766ca2e82fddad9728 --squash`. Then create the workspace crates in 2.2, moving Paneru sources into `crates/strip-engine` (keep the git history note in `engine/NOTICE.md`).
  - Copy Paneru's `LICENSE.txt` to `engine/LICENSE.txt` and to `docs/ACKNOWLEDGEMENTS.md`.
- **Acceptance:**
  - `scripts/bootstrap.sh` exits 0.
  - `cargo build --workspace --release` (in `engine/`) succeeds.
  - `grep -q "MIT License" engine/LICENSE.txt`.

### Step 2 — Trim and rename the engine
- **Tasks:**
  - Remove the Lua runtime (`src/lua*`, `mlua` deps, `lua` features) and virtual workspaces (rows). Keep the native-Spaces strips.
  - Remove Paneru's menubar item and Mach-service reader/client, `install`/`launchctl` service commands, and the app-launcher wrapper.
  - Rename the binary to `strip-engine`.
  - Replace the config loader with the schema in 1.6 (serde, `deny_unknown_fields` off, defaults exactly as listed) and hot reload via `notify` on the config path.
  - Map Paneru options:
    - `padding` → `gap_outer`;
    - new global `gap_inner` (horizontal gap between columns and vertical gap inside stacks);
    - `auto_center` → `center_focused_column`;
    - `sliver_width` stays internal (5);
    - `focus_follows_mouse` / `mouse_follows_focus` default false.
  - Replace `animation_speed` with the spring config.
- **Acceptance:**
  - `cargo clippy --workspace -- -D warnings` and `cargo test --workspace` pass.
  - `rg -n "mlua|lua" engine/crates` returns nothing.
  - A unit test parses the full 1.6 example and asserts every default.

### Step 3 — `strip-core`: pure model and layout
- **Tasks:**
  - Implement the 1.1 data types, plus `fn layout(strip, viewport, config, sizes) -> Vec<(WindowId, Frame, Visibility)>`.
  - Implement every 1.2 operation as pure functions on `Strip`: insert, remove, focus\_\*, move\_\*, cycle_width, wider/narrower, full_width, center, taller/shorter/equalize, stack_left/right, unstack_left/right, toggle_tabbed, pin_left/right, move_column, move_to_index, keep_original_height, size_override.
  - Implement scroll-into-view and the three center modes. Implement the hiding classification (visible / partial / hidden) and parked-corner computation from 2.6 (input: display frame + neighbour sides).
  - Wire the engine's ECS layout systems to call `strip-core::layout` (replace Paneru's layout math where it differs, keeping its ECS plumbing).
- **Acceptance:** `cargo test -p strip-core` includes:
  - property tests (`proptest`, 1,000 cases each): "insert/remove/move never changes other columns' width_ratio"; "every window in exactly one column"; "no managed frame crosses the display edge except partial columns, which keep ≥ sliver_width inside"; "hidden windows expose ≤ s×s pt on their own display";
  - table tests for each center mode, pinning (occupied side swap, unpin index), stacks both directions, tabbed visibility, and stubborn-app override.

### Step 4 — Platform layer hardening
- **Tasks:**
  - Implement 2.4, 2.5 (writer pool, enhanced-UI guard, size → position → size, read-back), 2.6 (parking + relocation repair), 2.10 (SkyLight + fallback) and 2.11.
  - Put all private symbols in `strip-engine::private_api` with `dlsym` + availability flags.
  - Add a `metrics` query: AX write p50/p95/max per bundle ID, frames per second, last frame duration, idle CPU.
  - Install a panic hook + SIGTERM/SIGINT handlers that un-park all windows.
- **Acceptance:**
  - Mock tests (`mockall` on `WindowApi`): the writer pool coalesces 100 rapid targets to ≤ 3 writes on a slow mock (20 ms/write), and the last write equals the last target.
  - Mock: enhanced-UI is restored after writes.
  - `e2e.sh` opens 6 TestWindows windows, then asserts:
    - all are managed;
    - widths equal the default;
    - columns are ordered by open order;
    - windows outside the viewport expose ≤ 5×5 pt;
    - after `kill -TERM <engine>`, every window's frame is fully inside a display.

### Step 5 — IPC server and CLI
- **Tasks:**
  - Implement `strip-ipc` (2.3): framing, serde types shared by server and CLI, and a golden JSON fixture for each message in `engine/crates/strip-ipc/fixtures/`.
  - Server: async on the engine's task pool. Commands are posted to the main thread; responses are sent after the action is applied.
  - `strip-cli`: `strip <action> [n]`, `strip query <what> --json`, `strip subscribe --json`.
- **Acceptance:**
  - Fixture round-trip tests pass.
  - `e2e.sh`: `strip focus_index 3` → `query state` reports focused column 2 (0-based); `strip subscribe --json` emits a `focus` event within 100 ms of `strip focus_right`; an unknown action returns `unknown_action`.

### Step 6 — Hotkeys
- **Tasks:**
  - Implement 2.7 with the full binding table from 1.3 as defaults. Support multiple bindings per action and swallow only matched combinations.
  - Add the `shortcut_set_version` upgrade merge (1.3).
- **Acceptance:**
  - Unit test: binding parser round-trips every default.
  - Unit test: the upgrade merge keeps user bindings and adds only non-conflicting new ones.
  - `e2e.sh` posts synthetic key events with `CGEvent(keyboardEventSource:virtualKey:keyDown:)` + `.post(tap: .cghidEventTap)` for ⌃⌥L, ⌃⌥R and ⌃⌥⇧H, and asserts state changes. It also asserts that an unbound ⌃⌥Z reaches a focused TestWindows text field.

### Step 7 — Animation
- **Tasks:**
  - Implement 2.9: spring in `strip-core::spring`, display-link driver in the engine, visible-range animation, position-only frames, and frame 0 on keypress.
- **Acceptance:**
  - `strip-core` tests: ζ = 1.0 never overshoots; ζ = 0.9 settles within 1.2 × duration; retargeting preserves velocity; `dt` capping holds.
  - `e2e.sh` with 8 windows: issue `focus_right` × 5, then read `metrics`. Assert:
    - p95 frame interval ≤ 1.5 × the display refresh interval for windows from TestWindows;
    - zero size writes during animation frames;
    - the display link stops within 1 s after settling;
    - idle CPU < 0.5% over 30 s (`ps -o %cpu`).

### Step 8 — Swipe
- **Tasks:**
  - Implement 2.8 and 1.4: gesture math in `strip-core::gesture`, the engine hook, and the config fields.
  - Add an engine test hook `strip debug inject_gesture --fingers 4 --dx <norm> --phase began|moved|ended` (compiled only with the `debug-hooks` feature) that feeds the same code path as real touches.
- **Acceptance:**
  - `strip-core` tests: 1:1 tracking (Δscroll = −Δx × viewport width × sensitivity), fling decay, snap to nearest column, rubber-band clamp, direction reversal.
  - `e2e.sh` (debug-hooks build): an injected 4-finger swipe of −0.5 moves `scroll_x` by +0.5 × viewport width ± 2 pt during `moved`, then snaps to a column boundary after `ended`. An injected 3-finger swipe with `swipe_fingers = 4` changes nothing.

### Step 9 — Multi-display, Spaces, pinning, rules, float, tabs, restore, lifecycle
- **Tasks:**
  - Implement all remaining 1.2 rows in the engine:
    - per-display strips, `send_next_display`, drag across displays, `ignored_displays`;
    - Spaces strips and fullscreen exclusion;
    - pinning layout;
    - rules (`slot`, `floating`) and auto-float;
    - native tab groups, tabbed columns, minimize/hide reinsert;
    - drag to reorder; focus-follows-mouse / mouse-follows-focus;
    - no focus stealing;
    - session restore (`state.json`, 10 s grace);
    - the sleep/lock/wake re-sync.
  - Add `strip debug simulate_wake` (debug-hooks) to call the wake path.
- **Acceptance:** `e2e.sh`:
  - `pin_left` → the pinned window's x equals visible minX + gap_outer, and it stays fixed across `focus_right` × 3.
  - A rule `{bundle_id = TestWindows, title = "Float.*", floating = true}` → a window titled "Float 1" is not in any strip.
  - A TestWindows `--fixed-size` window floats automatically. A `--dialog` window floats.
  - A background-launched window (TestWindows `--background`) doesn't change the focused column or `scroll_x`.
  - Restart the engine → columns restore in the same order with the same widths.
  - `simulate_wake` → state is identical and the tap is enabled.
  - On a machine with 2+ displays (detected; the check is skipped with a logged notice on a single display): `send_next_display` moves the window into the other display's strip, and `ignored_displays` leaves windows untouched.
  - `cargo test` covers Spaces fallback key generation and restore matching (ID match, fallback match, ambiguous title → unmatched).

### Step 10 — SwiftUI app shell, supervisor, IPC client
- **Tasks:**
  - Write `app/project.yml`: target macOS 13.0, arm64, hardened runtime, non-sandboxed, `LSUIElement = NO` with runtime activation-policy switching per `show_in_dock`.
    - SPM deps: Sparkle (`https://github.com/sparkle-project/Sparkle`, from 2.6.0). Do not add a hotkey library: the engine owns hotkeys.
    - Run-script phase: `scripts/build-engine.sh` copies `strip-engine` and `strip` into `Contents/MacOS/`.
  - Implement the engine supervisor (2.1), including the fallback LaunchAgent path; an async IPC client (`Network.framework` `NWConnection` over a Unix endpoint, or `FileHandle` + `DispatchSource`) with line framing and Codable types generated to mirror the Rust fixtures; and the menu bar extra (1.7).
- **Acceptance:**
  - `xcodegen generate && xcodebuild -scheme Strip -configuration Debug build test` passes.
  - Swift unit tests decode every Rust golden fixture (copied in by the test build phase).
  - A UI-less integration test launches the built app with `--headless-test`, which starts the engine, asserts `hello.ax_trusted`, sends `query state`, exits 0.
  - Kill the engine → the supervisor restarts it within 1 s (asserted in the same test).

### Step 11 — Settings UI, shortcut recorder, overlays
- **Tasks:**
  - Build every settings section in 1.7. Changes write `config.toml` atomically (temp file + rename); the engine hot-reloads it.
  - Shortcut recorder: `NSEvent.addLocalMonitorForEvents(.keyDown)` while a field is focused. Keycode + flags → binding, with a layout-aware label via `UCKeyTranslate`, conflict detection across actions, and multiple fields per action.
  - App picker for rules.
  - Overlays in the app:
    - **Strip indicator** (1.5) as an `NSPanel` (`.nonactivatingPanel`, `.borderless`, level `.statusBar`, `NSVisualEffectView` material `.hudWindow`), driven by `indicator` + `state` events, with click → `focus_window` and drag → `move_column`.
    - **Tab bar** for tabbed columns.
  - Focus border and dim stay in the engine (Paneru `overlay.rs` / dim code), driven by config.
- **Acceptance:**
  - Swift tests: config round-trip (load → modify each control's binding → save → parse in Rust via `strip-engine --validate-config <path>` exit 0); recorder maps keycode 4 + ⌃⌥ to `ctrl+alt-h`; conflict detection flags duplicates.
  - `e2e.sh`: write `gap_inner = 16` to the config file → within 500 ms the frames in `query state` reflect the new gap.
  - Snapshot test of the indicator view with 5 tiles (pinned + focused states).

### Step 12 — Onboarding and Mac Setup
- **Tasks:**
  - Implement first run (1.7) and every check in 2.12, with Fix buttons and 5 s re-polling.
  - The tour listens for `tour` events from the engine (the engine emits `tour` for each action executed).
  - Write the setup guide as an in-app page (`Help ▸ Setup Guide`) describing exactly the 2.12 fixes.
- **Acceptance:**
  - Swift unit tests feed each check parser fixture values (pass and fail) and assert the result plus the Fix URL.
  - The tour state machine advances on synthetic `tour` events.
  - The headless test asserts that first run is shown when `hasCompletedOnboarding` is false.

### Step 13 — Automation surfaces
- **Tasks:**
  - URL scheme handler (`CFBundleURLTypes`, `NSAppleEventManager` / `onOpenURL`) mapping to IPC commands.
  - Menu item "Install Command Line Tool…" symlinks `Contents/MacOS/strip` into `/usr/local/bin/strip` (admin prompt: `osascript -e 'do shell script "ln -sf <src> /usr/local/bin/strip" with administrator privileges'`), with fallback to `~/.local/bin/strip`.
  - Raycast script commands for focus left/right, cycle width, full width, center, pin left/right, toggle float, stop.
- **Acceptance:**
  - `open "strip://focus_index?n=2"` → state focused column = 1.
  - Every Raycast script has valid `@raycast.schemaVersion 1` headers (`scripts/lint-raycast.sh`).

### Step 14 — Licensing (Dodo Payments)
- **Tasks:** implement Section 4.6 in `app/Strip/Licensing/`.
- **Acceptance:**
  - Swift tests with a `URLProtocol` stub covering: activate 201 → licensed; 422 → "activation limit reached"; 404 → "invalid key"; validate `valid:false` → unlicensed after grace; offline for 13 days → still licensed; 31 days → unlicensed; deactivate clears the Keychain.
  - No API key string appears in the binary: `strings` on the built app | `rg -i "dodo.*key|sk_"` returns nothing.

### Step 15 — Packaging, signing, notarization, DMG, Sparkle, CI
- **Tasks:** implement Section 4 scripts and `.github/workflows/release.yml`. All scripts support `DRY_RUN=1` and print `MISSING: {{NAME}}` for each unset placeholder.
- **Acceptance:**
  - `DRY_RUN=1 scripts/release.sh` completes and lists every command it would run.
  - With real credentials present: `codesign --verify --deep --strict --verbose=2 build/Strip.app`, `spctl -a -vvv -t exec build/Strip.app` ("source=Notarized Developer ID"), `xcrun stapler validate build/Strip.dmg`, and `spctl -a -vvv -t open --context context:primary-signature build/Strip.dmg` all pass.
  - `appcast.xml` validates against Sparkle's `generate_appcast` output.

### Step 16 — Definition of done
- **Tasks:** write `scripts/dod.sh`, which runs every check in Section 5, and fix all failures.
- **Acceptance:** `scripts/dod.sh` exits 0 (signing and notarization items report `SKIPPED (placeholder)` when credentials are unset).

---

## 4. Packaging and release

### 4.1 Placeholders (`config/release.env`)
| Placeholder | What it is |
|---|---|
| `{{APP_NAME}}` | Product name (default `Strip`) |
| `{{BUNDLE_ID}}` | e.g. `com.yourcompany.strip` |
| `{{URL_SCHEME}}` | e.g. `strip` |
| `{{APPLE_TEAM_ID}}` | 10-character Apple Developer Team ID |
| `{{DEVELOPER_ID_APPLICATION}}` | Signing identity, e.g. `Developer ID Application: Robert Courson (TEAMID)` |
| `{{ASC_KEY_ID}}`, `{{ASC_ISSUER_ID}}`, `{{ASC_KEY_P8_PATH}}` | App Store Connect API key for notarytool (preferred) |
| `{{NOTARY_PROFILE}}` | Keychain profile name created by `notarytool store-credentials` (e.g. `strip-notary`) |
| `{{SPARKLE_PUBLIC_ED_KEY}}` | Base64 EdDSA public key (`SUPublicEDKey`) |
| `{{SPARKLE_PRIVATE_KEY_PATH}}` or CI secret `SPARKLE_PRIVATE_KEY` | EdDSA private key for `sign_update` / `generate_appcast` |
| `{{APPCAST_URL}}` | e.g. `https://updates.example.com/appcast.xml` |
| `{{UPDATE_HOST_UPLOAD}}` | Upload target for DMGs + appcast (e.g. an S3/R2 bucket URI or GitHub Releases repo) |
| `{{DODO_ENV}}` | `test` or `live` (→ `https://test.dodopayments.com` / `https://live.dodopayments.com`) |
| `{{DODO_PRODUCT_ID_1}}`, `{{DODO_PRODUCT_ID_3}}`, `{{DODO_PRODUCT_ID_5}}`, `{{DODO_PRODUCT_ID_20}}` | Dodo product IDs per tier (activation limits 1/3/5/20 set in the Dodo dashboard's License Key entitlement) |
| `{{DODO_CHECKOUT_URL}}` | Buy-page / checkout link opened by "Buy License" |
| `{{TRIAL_DAYS}}` | Trial length before a license is required (0 = license required to run) |
| `{{SUPPORT_EMAIL}}`, `{{WEBSITE_URL}}` | Shown in About / Help |
| `{{APP_ICON_1024_PNG}}` | 1024×1024 icon source (a generated placeholder icon is used until supplied) |

### 4.2 Build
```bash
source config/brand.env; source config/release.env
(cd engine && cargo build --release --target aarch64-apple-darwin -p strip-engine -p strip-cli)
(cd app && xcodegen generate)
xcodebuild -project app/Strip.xcodeproj -scheme Strip -configuration Release \
  -archivePath build/Strip.xcarchive archive \
  DEVELOPMENT_TEAM="$APPLE_TEAM_ID" CODE_SIGN_IDENTITY="$DEVELOPER_ID_APPLICATION" \
  PRODUCT_BUNDLE_IDENTIFIER="$BUNDLE_ID" ENABLE_HARDENED_RUNTIME=YES
xcodebuild -exportArchive -archivePath build/Strip.xcarchive \
  -exportOptionsPlist app/ExportOptions.plist -exportPath build/
```
`app/ExportOptions.plist`: `method = developer-id`, `teamID = {{APPLE_TEAM_ID}}`, `signingStyle = manual`, `signingCertificate = Developer ID Application`.

### 4.3 Sign (inside-out, every nested binary)
`app/Resources/Strip.entitlements`: empty dict, hardened runtime, no sandbox. No Accessibility entitlement exists or is needed.
```bash
APP=build/Strip.app; ID="$DEVELOPER_ID_APPLICATION"
for b in "$APP/Contents/MacOS/strip-engine" "$APP/Contents/MacOS/strip"; do
  codesign --force --options runtime --timestamp --sign "$ID" "$b"; done
SP="$APP/Contents/Frameworks/Sparkle.framework/Versions/B"
codesign --force --options runtime --timestamp --sign "$ID" "$SP/XPCServices/Installer.xpc"
codesign --force --options runtime --timestamp --sign "$ID" --preserve-metadata=entitlements "$SP/XPCServices/Downloader.xpc"
codesign --force --options runtime --timestamp --sign "$ID" "$SP/Autoupdate" "$SP/Updater.app"
codesign --force --options runtime --timestamp --sign "$ID" "$APP/Contents/Frameworks/Sparkle.framework"
codesign --force --options runtime --timestamp --entitlements app/Resources/Strip.entitlements --sign "$ID" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
```
Keep the signing identity and bundle ID constant across releases; the Accessibility grant is tied to them.

### 4.4 Notarize, DMG, staple
```bash
xcrun notarytool store-credentials "$NOTARY_PROFILE" \
  --key "$ASC_KEY_P8_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID"   # once
ditto -c -k --keepParent build/Strip.app build/Strip.zip
xcrun notarytool submit build/Strip.zip --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple build/Strip.app
create-dmg --volname "$APP_NAME" --window-size 540 380 --icon-size 128 \
  --icon "$APP_NAME.app" 140 190 --app-drop-link 400 190 \
  --background app/Resources/dmg-background.png build/Strip.dmg build/Strip.app
codesign --force --timestamp --sign "$DEVELOPER_ID_APPLICATION" build/Strip.dmg
xcrun notarytool submit build/Strip.dmg --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple build/Strip.dmg
spctl -a -vvv -t exec build/Strip.app
```
On notarization failure, `scripts/notarize.sh` runs `xcrun notarytool log <id> --keychain-profile "$NOTARY_PROFILE"` and prints the issues.

### 4.5 Sparkle 2
- Info.plist: `SUFeedURL = {{APPCAST_URL}}`, `SUPublicEDKey = {{SPARKLE_PUBLIC_ED_KEY}}`, `SUEnableAutomaticChecks = YES`, `SUScheduledCheckInterval = 86400`, `SUAutomaticallyUpdate = YES`.
- Keys (once): `./bin/generate_keys` from the Sparkle distribution. It prints the public key; export the private key with `generate_keys -x sparkle_private.pem` and store it as CI secret `SPARKLE_PRIVATE_KEY`.
- Per release:
  ```bash
  mkdir -p build/updates && cp build/Strip.dmg "build/updates/Strip-$VERSION.dmg"
  ./bin/generate_appcast --ed-key-file "$SPARKLE_PRIVATE_KEY_PATH" \
    --download-url-prefix "$(dirname "$APPCAST_URL")/" build/updates
  # upload build/updates/*.dmg and build/updates/appcast.xml to {{UPDATE_HOST_UPLOAD}}
  ```
- `CFBundleShortVersionString` = semver; `CFBundleVersion` = monotonically increasing integer (CI run number).
- App: `SPUStandardUpdaterController(startingUpdater: true, …)`, with menu item "Check for Updates…" → `checkForUpdates(_:)`.
- On update, the supervisor sends `shutdown` before Sparkle relaunches (`SPUUpdaterDelegate.updaterWillRelaunchApplication`).

### 4.6 Licensing (Dodo Payments)
- **Base URL:** `https://{{DODO_ENV}}.dodopayments.com`. The license endpoints are public and need no API key; never embed one.
- **Activate:** `POST /licenses/activate` with `{"license_key": K, "name": "<APP_NAME>-<first 12 hex of SHA256(IOPlatformUUID)>"}` → 201 `{ "id": "lki_…", "product": {"product_id": …}, … }`.
  - Verify that `product.product_id` is one of `{{DODO_PRODUCT_ID_*}}`.
  - Store `{license_key, instance_id, product_id, activated_at, last_validated_at}` in the Keychain (`kSecClassGenericPassword`, service `{{BUNDLE_ID}}.license`).
  - Errors: 403 → "key not active", 404 → "invalid key", 422 → "activation limit reached — deactivate another Mac".
- **Validate:** `POST /licenses/validate` with `{"license_key": K, "license_key_instance_id": I}` → `{"valid": bool}`. Call it at launch and every 24 h. Network errors keep the license valid until `last_validated_at` is more than 30 days old. `valid:false` → unlicensed immediately.
- **Deactivate:** `POST /licenses/deactivate` with `{"license_key": K, "license_key_instance_id": I}` → clear the Keychain.
- **Machine ID:** `IORegistryEntryCreateCFProperty(IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice")), "IOPlatformUUID")`, SHA-256, hex. Never send the raw UUID.
- **States:**
  - `trial(days_left)`: `{{TRIAL_DAYS}}` from first launch, stored in the Keychain.
  - `licensed`.
  - `unlicensed`: the engine is set to `set_managing false`, and the main window shows the activation view with "Buy License" (opens `{{DODO_CHECKOUT_URL}}`).
- **UI:** General ▸ License shows the state, masked key, product tier, Deactivate button, and Enter Key field.
- **Revocation and refunds** are handled by `valid:false` on the next validation.

### 4.7 CI (`.github/workflows/release.yml`)
- Trigger: tags `v*`. Runner: `macos-15`.
- Steps: checkout → `scripts/bootstrap.sh` → import the Developer ID `.p12` from secrets `DEVELOPER_ID_P12_BASE64` + `DEVELOPER_ID_P12_PASSWORD` into a temporary keychain → write the ASC `.p8` from secret → `scripts/release.sh` → upload artifacts.
- `ci.yml` on every push runs `cargo clippy`, `cargo test` and `xcodebuild test`. On-device e2e runs locally via `scripts/e2e.sh`, not on CI, because CI runners cannot grant Accessibility.

---

## 5. Definition of done (`scripts/dod.sh` checks each line)

**Build and quality**
- [ ] `cargo clippy --workspace -- -D warnings` clean; `cargo test --workspace` green (including the proptest invariants).
- [ ] `xcodebuild test` green.
- [ ] `scripts/e2e.sh` green.
- [ ] No `TODO|FIXME|unimplemented!|todo!` in shipped sources (`rg` returns nothing).

**Feature parity (each covered by an e2e or unit assertion)**
- [ ] Strip insert/close/scroll-into-view; all 3 center modes; preset/wider/narrower/full/center widths; gaps inner/outer; Dock-aware.
- [ ] Stacks: stack left/right, pop out left/right, reorder, taller/shorter/equalize.
- [ ] Tabbed columns with a clickable tab bar; native macOS tab groups as one column.
- [ ] Pin left/right with toggle and swap.
- [ ] Float toggle; auto-float for dialogs, fixed-size windows, Quick Look and own windows; rules (float, slot).
- [ ] Keep Original Height; stubborn-app override; no focus stealing; native fullscreen ignored.
- [ ] Every default shortcut in 1.3 works; multiple bindings; upgrade merge; layout-aware recorder.
- [ ] Swipe 3/4/5/off, natural/reversed, 1:1 tracking, fling, snap, rubber-band.
- [ ] Spring animation honours duration/damping; frame 0 on keypress; position-only frames; display link idles.
- [ ] Strip indicator: show/fade, hold with ⌃⌥, click to focus, drag to reorder.
- [ ] Focus border; dim inactive.
- [ ] Multi-display strips; send to next display; drag across displays; per-display ignore.
- [ ] Spaces strips with SkyLight, and the fallback path tested.
- [ ] Session restore; sleep/lock/wake re-sync; un-park on stop/quit/crash/signal.
- [ ] Focus follows mouse / mouse follows focus (off by default).
- [ ] Drag to reorder.
- [ ] URL scheme, CLI (`query`, `subscribe`) and Raycast scripts.

**App**
- [ ] Menu bar extra; settings apply instantly; Mac Setup checks with Fix links; first-run tour.
- [ ] Engine supervisor restarts within 1 s; Reveal Engine Log.
- [ ] Launch at login via `SMAppService.mainApp`.

**Performance**
- [ ] Idle CPU < 0.5%; keypress → first AX write < 16 ms; p95 frame interval ≤ 1.5 × refresh on TestWindows.

**Ship**
- [ ] Signed (hardened runtime), notarized, stapled `.app` and `.dmg` — or `SKIPPED (placeholder)`.
- [ ] Sparkle appcast generated and EdDSA-signed; update from N-1 to N verified (`scripts/test-update.sh` serves a local appcast on `python3 -m http.server`).
- [ ] Dodo activate/validate/deactivate flows pass the stub tests; no secrets in the binary.

**Legal**
- [ ] `engine/LICENSE.txt` (Paneru MIT) present; Acknowledgements screen lists Paneru, Sparkle and every other dependency license (generated by `cargo about` / `swift package` license scan into `docs/ACKNOWLEDGEMENTS.md` and bundled).
- [ ] `scripts/check-branding.sh`: `rg -i "chainyourmac|chain your mac"` over the repo (excluding `docs/DECISIONS.md` provenance notes) returns nothing.

---

## 6. License compliance

1. **Paneru (MIT)**
   - Keep `engine/LICENSE.txt` with the original copyright notice, unmodified.
   - Add `engine/NOTICE.md`: "Engine derived from Paneru (https://github.com/karinushka/paneru) at commit 98781431b09ebe44ef226b766ca2e82fddad9728, MIT License."
   - Show the MIT text in About ▸ Acknowledgements.
2. **ChainYourMac**
   - Copy nothing: no code or binaries, no UI text, website copy, name, logo, icons, screenshots, demo, colours or DMG art. Do not disassemble or reverse-engineer it.
   - Implement behaviours from this spec only. Product name, icon and website are Robert's own.
3. **GPL projects (OmniWM, Nehir, niri, PaperWM, Loop, alt-tab-macos, and any other GPL/LGPL/AGPL source)**
   - Copy no code, and do not translate their code line by line.
   - Use only the public-API knowledge already stated in this spec.
4. **Permissively licensed sources** (MIT/Apache-2.0/BSD: Paneru, AeroSpace, Amethyst, PaperWM.spoon, yabai, Rectangle, Hammerspoon, Swindler, Sparkle, objc2, create-dmg)
   - Code may be reused with its license notice added to `docs/ACKNOWLEDGEMENTS.md` and the in-app Acknowledgements screen.
5. **Dependency audit:** `cargo deny check licenses` with allow-list `MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, Zlib, MPL-2.0`. It fails the build on any GPL-family license.

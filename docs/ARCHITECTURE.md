# Architecture

Strip is two processes. The SwiftUI app owns the menu bar, settings, overlays, licensing, and Sparkle. `strip-engine` owns Accessibility, the event tap, layout, and animation. They speak newline-delimited JSON over a Unix socket (protocol 1). `strip` is a CLI on the same socket.

`strip-core` is pure: the strip model, layout, parking, springs, and gestures. The engine calls it. Tests of those functions do not need a window server.

Private macOS symbols are resolved with `dlsym` in `strip-engine::private_api` and checked at startup. If any SkyLight symbol is missing, strips key by display UUID and Space changes use a hash of on-screen window ids. `hello.private_api.skylight` is false in that case.

One strip exists per display and Space. Layout never changes another column's `width_ratio` when a window is inserted, closed, or moved. Windows that leave the viewport are parked so only a sliver remains on their own display.

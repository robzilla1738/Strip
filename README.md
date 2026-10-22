# Strip

A native scrolling window manager for macOS 13+ on Apple silicon, built with a SwiftUI menu bar app and a Rust layout/animation engine (derived from [Paneru](https://github.com/karinushka/paneru), MIT).

## Build & Run

```sh
./scripts/bootstrap.sh
./scripts/build-app.sh
open build/DerivedData/Build/Products/Debug/Strip.app
```

## Verification

- `./scripts/dod.sh` — runs Clippy (`-D warnings`), Rust unit/property tests, Xcode unit/headless tests, end-to-end on-device checks, license checks, secret scan, and branding checks.
- `./scripts/shortcuts.sh` — exercises every default keyboard shortcut as a real HID event against live test windows.

## Default Shortcuts

Hold `⌃⌥` at any time to peek at the strip indicator, or open **Shortcuts → Keyboard Shortcuts Reference…** (`⌘/`) from the menu bar icon.

| Shortcut | Action |
|---|---|
| `⌃⌥H` / `⌃⌥←` | Focus Left |
| `⌃⌥L` / `⌃⌥→` | Focus Right |
| `⌃⌥K` / `⌃⌥↑` | Focus Up in Stack / Previous Tab |
| `⌃⌥J` / `⌃⌥↓` | Focus Down in Stack / Next Tab |
| `⌃⌥⌘←` / `⌃⌥⌘→` | Focus First / Last Column |
| `⌃⌥1` … `⌃⌥9` | Focus Column 1–8 (`9` = Last) |
| `⌃⌥⇧H` / `⌃⌥⇧←` | Move Column Left |
| `⌃⌥⇧L` / `⌃⌥⇧→` | Move Column Right |
| `⌃⌥⇧K` / `⌃⌥⇧↑` | Move Window Up in Stack |
| `⌃⌥⇧J` / `⌃⌥⇧↓` | Move Window Down in Stack |
| `⌃⌥⌘⇧←` / `⌃⌥⌘⇧→` | Move Column to First / Last |
| `⌃⌥⇧1` … `⌃⌥⇧9` | Move Column to 1–8 (`9` = Last) |
| `⌃⌥R` | Cycle Column Width |
| `⌃⌥F` | Toggle Full Width |
| `⌃⌥C` | Center Focused Column |
| `⌃⌥=` / `⌃⌥-` | Wider / Narrower (`±10%`) |
| `⌃⌥⇧=` / `⌃⌥⇧-` | Taller / Shorter in Stack (`±10%`) |
| `⌃⌥0` | Equalize Stack Heights |
| `⌃⌥]` / `⌃⌥[` | Stack into Left / Right Column |
| `⌃⌥⇧]` / `⌃⌥⇧[` | Pop Out to Right / Left |
| `⌃⌥⇧T` | Toggle Tabbed Column |
| `⌃⌥T` | Float / Unfloat Window |
| `⌃⌥N` | Send to Next Display |
| `⌃⌥P` / `⌃⌥⇧P` | Pin / Unpin Left / Right Edge |
| `⌃⌥Q` | Stop Managing Windows |

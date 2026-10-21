# Testing

From `engine/`:

```bash
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

The `strip-core` tests cover the Section 1.1 invariants (1,000 proptest cases), center modes, pinning, stacks, tabs, stubborn sizes, the spring, and gestures. `strip-ipc` round-trips the golden fixtures. `strip-engine` tests the config example, binding merge, writer-pool coalescing, the enhanced-UI guard, restore matching, and the SkyLight-missing fallback.

The app:

```bash
scripts/gen-brand.sh
cd app && xcodegen generate
xcodebuild -project Strip.xcodeproj -scheme Strip -configuration Debug -destination 'platform=macOS,arch=arm64' test
```

`scripts/e2e.sh` drives a real engine. It needs Accessibility for the terminal. If that grant is missing it records the failure and exits non-zero instead of skipping assertions.

`DRY_RUN=1 scripts/release.sh` prints the commands it would run and `MISSING: {{NAME}}` for each unset placeholder. `scripts/dod.sh` runs the Section 5 checklist.

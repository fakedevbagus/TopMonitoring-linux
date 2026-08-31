## Summary

Describe the user-visible behavior and why the change is needed.

## Validation

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --all-features`
- [ ] `cargo test --no-default-features`
- [ ] `./scripts/test-no-wayland.sh check` (for X11/no-Wayland changes)
- [ ] Tested the candidate binary with isolated config before packaging
- [ ] Tested relevant Wayland/X11 behavior
- [ ] Updated `CHANGELOG.md` and all applicable release documentation
- [ ] Updated screenshots when the visible UI changed materially
- [ ] No imported command becomes trusted automatically

## Screenshots / logs

Attach before/after screenshots for visual changes and concise logs for fixes.

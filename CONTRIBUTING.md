# Contributing

Thanks for improving TopMonitoring.

1. Create a focused branch: `git checkout -b feature/name`.
2. Update code and every document required by `RELEASE_CHECKLIST.md`.
3. Run the appropriate local gate:
   - no-Wayland/X11: `./scripts/test-no-wayland.sh check`
   - full feature matrix: the commands in `docs/TECHNICAL.md`
4. Run GUI smoke and manual tests from `target/release/topmonitoring` with an
   isolated profile. Do not overwrite the stable package for source testing.
5. Include logs and before/after screenshots for visible changes.
6. Open a PR only after the working tree is clean and all available gates pass.

Do not create a `.deb` before terminal and manual tests pass. For the no-Wayland
variant, follow `docs/TESTING.md` and use
`./scripts/package-no-wayland.sh --confirmed-manual`.

Bug reports should include distro, desktop environment, X11/Wayland session,
TopMonitoring version/commit, reproduction steps, relevant config with secrets
or commands redacted, and concise logs.

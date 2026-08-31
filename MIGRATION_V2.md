# Migrating from TopMonitoring 1.x to 2.0

## Automatic migration

TopMonitoring reads the existing
`~/.config/topmonitoring/config.toml`, fills all new fields, validates ranges,
adds missing built-in metrics, and writes schema version 2 the next time you
save. A parse failure preserves the invalid source as `config.toml.bak` and
starts with safe defaults.

Back up the file before first launch if it contains extensive custom CSS.

## Behavior changes

- Metrics now have a `zone`, `priority`, `compact`, `format`, `fg_color`, and
  `bg_color`. Existing metrics default to the center zone and safe values;
  missing built-ins are appended.
- Custom modules gain trust, interval, timeout, output cap, priority, format,
  tooltip, colors, and optional click action.
- Importing any file always clears trust for executable behavior and disables
  custom modules containing commands. This is intentional.
- Base theme CSS and user CSS are loaded independently. Invalid advanced CSS
  no longer replaces the safe base theme.
- History rotates at `history_max_mb`; the previous file becomes
  `history.csv.1`.
- The desktop/application ID changed to
  `io.github.fakedevbagus.TopMonitoring`. Re-enable autostart once to replace
  legacy entries.

## Debian package-name migration

TopMonitoring 1.x no-Wayland releases used the package name
`topmonitoring-no-wayland`; current packages use the unified name
`topmonitoring`. Version 2.0.1 declares the old package as conflicting and
replaced, allowing APT to transition ownership of shared files. Removing the
legacy package does not remove per-user files in `~/.config/topmonitoring`.
Back up `config.toml` before upgrading and verify it after first launch.

## Suggested first-run review

1. Open Settings → **Built-in modules** and assign start/center/end zones.
2. Review every click command and set **Trusted** only when expected.
3. Open **Custom modules**, verify commands, cadence, timeout, and output cap,
   then enable trusted modules.
4. Review Appearance because old CSS can override new visual tokens.
5. Save, restart, and verify docking on the intended monitor.

## Rollback

Stop TopMonitoring 2, restore your backed-up 1.x configuration, and reinstall
the earlier binary. Version 1.x ignores neither every new field nor the new
desktop ID reliably, so do not share one live config between both versions.

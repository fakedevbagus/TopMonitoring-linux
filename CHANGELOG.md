# Changelog

All notable changes to this project are documented here, following
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [1.1.1] - 2026-07-04
### Fixed
- A regression where malformed generated CSS silently broke theme switching
  and custom fonts.
- App icon regenerated with proper safe-zone padding to avoid cropping at
  small sizes (title bar, taskbar).
### Added
- Critical metrics now blink instead of just changing color.

## [1.1.0] - 2026-07-04
### Added
- GUI threshold editor (warn/crit per metric), middle-click detail popups,
  saved appearance presets, colored/filled graphs, auto-dim, best-effort
  Intel GPU clock reading.
### Changed
- Full English codebase, UI, and documentation.
### Fixed
- Custom modules now run on background threads (no more UI freezes).
- Settings window is now single-instance.
- Corrupted config is backed up instead of silently discarded.
- Refresh interval now applies live without restarting the app.

## [1.0.0] - 2026-07-03
### Added
- Initial full release: X11/Wayland dock, complete metric set, hwmon sensor
  window, graphs, custom modules, live-apply settings, export/import,
  autostart, packaging.
# Changelog

All notable changes to TopMonitoring are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [1.3.0]

### Added
- Volume slider on the bar with live sync to the OS mixer (`wpctl`, falling back to `pactl`) and a mute toggle button whose icon updates live (🔊/🔇).
- Brightness slider on the bar (`brightnessctl`, falling back to direct `/sys/class/backlight` read/write).
- Bluetooth status metric (adapter power state + connected device count/names via `bluetoothctl`).
- VPN status metric (interface-based heuristic: `tun`/`tap`/`wg`/known VPN provider interfaces).
- Mini Process Manager popup: top processes by CPU or RAM, sortable, refreshing live every second.
- History logging: optionally append CPU%/RAM%/temperature to a CSV file (`~/.config/topmonitoring/history.csv`) on a configurable interval, with a Settings shortcut to open the log folder.
- Update checker: checks GitHub Releases for a newer version from Settings.
- `SIGUSR1` signal handling to show/hide the bar, so it can be bound to a custom desktop keyboard shortcut (`pkill -SIGUSR1 topmonitoring`).

### Changed
- Slider (Volume/Brightness) styling now follows the active theme's accent color instead of the default GTK look, and is vertically centered in the bar.
- Replaced a Bluetooth glyph that required a Nerd Font (and could render as a blank box) with a plain device count.
- Added a `.dim` style so inactive states (e.g. VPN off) are visibly faded instead of having no visual distinction.

## [1.2.0]

### Added
- Quick action buttons: Lock screen, Screenshot, and Shutdown (double-click within 4s to confirm), each independently toggleable in Settings.
- Wi-Fi metric: active SSID and connection status via `nmcli`.
- Pending package updates count metric.
- Media metric: MPRIS "now playing" artist/title via `playerctl`.
- Disk metric now also detects external/removable mounted drives, not just `/`.

## [1.1.1]

### Fixed
- A regression where malformed generated CSS silently broke theme switching and custom fonts.
- Critical metrics now blink instead of just changing color.
- Regenerated the app icon with proper safe-zone padding to prevent cropping at small sizes.

## [1.1.0]

### Added
- GUI threshold editor (per-metric warn/crit spin boxes).
- Middle-click detail popups.
- Saved appearance presets.
- Colored/filled graphs.
- Auto-dim on idle.
- Best-effort Intel GPU clock reading.

### Changed
- Stability pass: async custom modules, single settings window, live interval changes, corrupt-config recovery.
- Full English codebase, UI, and docs.

## [1.0.0]

### Added
- Initial release: X11/Wayland dock, complete base metric set, hwmon sensor window, graphs, custom modules, live-apply settings, export/import, autostart, packaging.

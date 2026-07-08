# TopMonitoring — Technical Documentation (v1.3)

> Official technical reference for developing, installing, configuring, and distributing TopMonitoring.

**TopMonitoring** is a native Linux system-monitor topbar that reserves real screen space (a true dock, not an overlay), shows rich system metrics in the spirit of LibreHardwareMonitor, and is fully customizable through its GUI. Built with **Rust + GTK4**.

## Table of contents

1. [Overview & Features](#1-overview--features)
2. [Architecture & Tech Stack](#2-architecture--tech-stack)
3. [Prerequisites](#3-prerequisites)
4. [Installation](#4-installation)
5. [Project Structure](#5-project-structure)
6. [Configuration Reference](#6-configuration-reference)
7. [Metric Reference](#7-metric-reference)
8. [Custom Modules](#8-custom-modules)
9. [Usage Guide](#9-usage-guide)
10. [Packaging & Distribution](#10-packaging--distribution)
11. [Troubleshooting](#11-troubleshooting)
12. [Roadmap](#12-roadmap)
13. [Changelog](#13-changelog)
14. [Professional Release Checklist](#14-professional-release-checklist)

## 1. Overview & Features

One lightweight binary that docks to a screen edge, forces fullscreen windows to shrink around it, exposes deep hardware sensors, and offers a live-apply settings panel.

| Category | Features |
|---|---|
| Display | Full-width topbar, top/bottom position, centered metrics (CenterBox), multi-monitor support |
| Reserved space | Wayland `wlr-layer-shell` (exclusive zone) + X11 `_NET_WM_STRUT_PARTIAL` |
| Metrics | CPU, RAM, swap, temperature, fan, GPU (utilization/temp/VRAM/power/clock), disk (including external/removable drives), disk I/O, network, uptime, load, battery, Wi-Fi, pending package updates, media now-playing, and more |
| Deep sensors | **Hardware Sensors** window reads every `hwmon` channel (voltage, current, wattage, fan, temperature) |
| Graphs | Colored, filled sparkline history for CPU / RAM / Network that changes color near warning/critical levels |
| Customization | Dark/light theme, live color picker, custom CSS, font & size, saved appearance presets |
| Interaction | Left-click a metric to launch an app, middle-click for a detail popup, rich tooltips, blinking + desktop notifications when a metric turns critical |
| Custom modules | Run any shell command and show its output on the bar, executed off the UI thread |
| System controls | Volume and Brightness sliders with live OS sync and a mute toggle, Bluetooth and VPN status, optional Quick Action buttons (Lock, Screenshot, Shutdown-with-confirm) |
| Extras | Mini Process Manager, CSV history logging, GitHub release update checker, SIGUSR1 show/hide shortcut for custom keybinding |
| Configuration | Live-apply with a single Save button, export/import config, autostart installer, reset to defaults |
| Stability | Single-instance guard, async custom-module execution, live interval changes, corrupt-config recovery |

## 2. Architecture & Tech Stack

| Component | Technology | Why |
|---|---|---|
| Language | Rust (edition 2021) | Native, memory-safe, small binaries |
| GUI | GTK4 (`gtk4-rs`) | Standard Linux panel toolkit, CSS-based styling |
| Reserved space (Wayland) | `gtk4-layer-shell` | Exclusive-zone layer-shell protocol |
| Reserved space (X11) | `x11rb` • `gdk4-x11` | Sets `_NET_WM_STRUT_PARTIAL` directly on the window's XID |
| System metrics | `sysinfo` | Cross-platform CPU/RAM/disk/network/temperature |
| NVIDIA GPU | `nvml-wrapper` | Power, clock, fan, VRAM, temperature |
| AMD / Intel GPU | Read `sysfs` (`/sys/class/drm`, `hwmon`) | No extra dependency required |
| Motherboard sensors | `/sys/class/hwmon` | Labeled voltage, current, power, fan, temperature |
| Configuration | `serde` • `toml` | Human-readable `config.toml` |

> **Core flow:** `main` → `Application::activate` (guarded to run once) → `build_bar` creates the layer-shell/dock window, builds metric widgets from config, then a restartable `glib::timeout` polls sensors on every interval and updates labels/graphs. Custom shell-command modules run on background threads and report back through an `mpsc` channel so a slow command never freezes the UI.

## 3. Prerequisites

### 3.1 Runtime

- Linux with Wayland (KDE/Hyprland/Sway) **or** X11 (XFCE, i3, etc.)
- `libgtk-4`, `libgtk4-layer-shell`, `libsensors`
- (Optional) NVIDIA driver + `nvidia-smi` for NVIDIA GPU metrics
- (Optional, for full feature coverage) `wpctl`/`pactl` (volume), `brightnessctl` (brightness), `bluetoothctl`/BlueZ (bluetooth), `nmcli`/NetworkManager (Wi-Fi), `playerctl` (media now-playing), `curl` (update checker), a screenshot tool such as `gnome-screenshot`/`xfce4-screenshooter`/`spectacle`/`flameshot`, and `loginctl`/`systemctl` (lock/shutdown quick actions) — each feature degrades gracefully (shows `n/a` or hides the control) if its tool isn't installed

### 3.2 Build

- Rust toolchain (rustup, stable)
- Dev libraries: `gtk4-devel`, `gtk4-layer-shell-devel`, `lm_sensors-devel`, `pkg-config`, a C compiler

## 4. Installation

### 4.1 System dependencies

```bash
# Debian/Ubuntu
sudo apt install -y build-essential pkg-config libgtk-4-dev \
  libgtk4-layer-shell-dev lm-sensors libsensors-dev
# Fedora
sudo dnf install -y gcc pkgconf-pkg-config gtk4-devel gtk4-layer-shell-devel lm_sensors lm_sensors-devel
# Arch
sudo pacman -S --needed base-devel gtk4 gtk4-layer-shell lm_sensors
```

### 4.2 Enable sensors

```bash
sudo sensors-detect --auto && sensors
```

### 4.3 Build & install

```bash
git clone https://github.com/fakedevbagus/TopMonitoring-linux.git
cd TopMonitoring-linux
chmod +x install.sh && ./install.sh
# or manually:
cargo build --release
sudo install -Dm755 target/release/topmonitoring /usr/local/bin/topmonitoring
```

## 5. Project Structure

```
topmonitoring/
├── Cargo.toml            # metadata, dependencies, release profile, .deb packaging
├── src/
│   ├── main.rs           # window, layout, polling, metrics, settings, sensors
│   └── config.rs         # Config struct, load/save, build_css, default_metrics
├── assets/               # app icon (SVG + PNG set + favicon)
├── docs/
│   └── TECHNICAL.md      # this document
├── topmonitoring.desktop # desktop entry
├── install.sh / uninstall.sh
├── README.md
├── CHANGELOG.md
└── LICENSE
```

## 6. Configuration Reference

File: `~/.config/topmonitoring/config.toml` (created automatically). If this file becomes corrupted, TopMonitoring backs it up as `config.toml.bak` and falls back to defaults instead of failing to start.

| Field | Type | Default | Description |
|---|---|---|---|
| `height` | int | 30 | Bar height (px) |
| `margin_top` | int | 0 | Offset from the screen edge (avoid overlapping another panel) |
| `position` | string | "top" | `top` / `bottom` |
| `monitor` | int | -1 | Monitor index (-1 = default) |
| `theme` | string | "dark" | `dark` / `light` |
| `custom_bg` | string | "" | Background color (e.g. `#101020`) |
| `custom_css` | string | "" | Custom CSS (target the `.topbar` selector) |
| `animated_bg` | bool | false | Rotating hue background |
| `auto_hide` | bool | false | Auto-dim the bar when the pointer is away, full opacity on hover |
| `notifications` | bool | true | Desktop notification when a metric turns critical |
| `font_family` | string | JetBrains Mono | Bar font |
| `font_size` | int | 12 | Font size |
| `interval_ms` | int | 1000 | Refresh interval — applies live, no restart needed |
| `metrics` | array | — | Metric list: id, label, enabled, command, warn, crit |
| `custom_modules` | array | — | Shell-command modules (name, label, command, enabled) |
| `presets` | array | built-in set | Saved appearance presets (theme, background, CSS, font) |
| `net_iface` | string | "" | Network interface to monitor ("" = auto-detect) |
| `gpu_index` | int | 0 | GPU index to monitor on multi-GPU systems |
| `qa_lock` | bool | false | Show a Lock screen quick-action button on the bar |
| `qa_screenshot` | bool | false | Show a Screenshot quick-action button on the bar |
| `qa_shutdown` | bool | false | Show a Shutdown quick-action button (click twice within 4s to confirm) |
| `history_enabled` | bool | false | Log CPU%/RAM%/temperature to a CSV file on an interval |
| `history_interval_secs` | int | 60 | Seconds between history log rows (5–3600) |
| `check_updates` | bool | true | Enable the GitHub release update checker in Settings |

> **Custom thresholds:** every metric's `warn` and `crit` fields are editable directly in the Settings window (per-metric spin boxes). `0` means "use the built-in default" for that metric.

## 7. Metric Reference

| ID | Description | Source |
|---|---|---|
| `clock` / `date` | Time / date | glib DateTime |
| `cpu` / `cpu_graph` | Usage % / colored sparkline | sysinfo |
| `cpumodel` / `freq` | CPU model & frequency | sysinfo |
| `cpu_power` | CPU package power (RAPL) | powercap |
| `vcore` | CPU core voltage | hwmon |
| `memory` / `ram_graph` / `memavail` / `swap` | RAM & swap | sysinfo |
| `temp` / `fan` | Highest temperature & fan RPM | hwmon |
| `gpu` | Utilization % + temperature + VRAM | NVML / sysfs |
| `gpu_power` / `gpu_clock` / `gpu_memclock` / `gpu_fan` | Deep GPU metrics | NVML / sysfs (Intel: clock only, best-effort) |
| `disk` / `diskio` | Usage & read/write throughput, including external/removable mounted drives | sysinfo / diskstats |
| `network` / `net_graph` / `netttl` | Rate & lifetime totals | sysinfo |
| `battery` | Capacity & charging status | power_supply |
| `procs` / `uptime` / `load` | Process count, uptime, load average | /proc, sysinfo |
| `host` / `kernel` / `os` | System info | sysinfo |
| `wifi` | Active SSID & connection status | nmcli |
| `pkg_updates` | Count of pending package updates | package manager |
| `media` | MPRIS "now playing" artist/title | playerctl |
| `bluetooth` | Adapter power state & connected device count/names | bluetoothctl |
| `volume` | Interactive slider: output volume & mute toggle | wpctl / pactl |
| `brightness` | Interactive slider: screen brightness | brightnessctl / sysfs |
| `vpn` | VPN interface active/inactive (heuristic) | sysinfo network interfaces |

> **Intel GPU note:** Intel integrated GPUs only expose a best-effort `gpu_clock` reading via `sysfs`. Utilization and power are not available without the `intel_gpu_top` tool (not integrated in this version), so `gpu`/`gpu_power`/`gpu_fan` will show `n/a` on Intel-only systems.

## 8. Custom Modules

Display the output of any shell command on the bar. Each module runs on its own background thread, so a slow command never blocks the UI; a new run only starts after the previous one returns.

```
name: weather   label: ☀   command: curl -s wttr.in/Jakarta?format=1
```

> Prefer fast, lightweight commands — they are re-run on every interval tick.

## 9. Usage Guide

- **Open Settings:** click the ⚙ icon on the right side of the bar, or right-click anywhere on the bar.
- **Live-apply:** every change is applied immediately. Click **💾 Save** to persist it. Closing the window without saving reverts to the last saved state.
- **Left-click a metric → launch an app:** set the command field for that metric (e.g. `xfce4-taskmanager`).
- **Middle-click a metric → detail popup:** shows the same rich detail available in the tooltip (per-core CPU, per-sensor temperatures, per-interface network, graph min/avg/max, etc.).
- **Volume & Brightness:** drag the sliders directly on the bar; click the speaker icon to toggle mute. Both stay in sync with changes made elsewhere (e.g. media keys).
- **Quick actions:** enable Lock, Screenshot, and/or Shutdown buttons in Settings; Shutdown requires two clicks within 4 seconds to confirm.
- **Process Manager:** button in Settings → popup listing the top CPU/RAM processes, sortable, refreshing every second.
- **History logging:** enable in Settings, set an interval (5–3600s), and use "Open history log folder" to find `history.csv` (timestamp, CPU%, RAM%, temperature).
- **Update checker:** "Check for Updates…" button in Settings compares the running version against the latest GitHub release.
- **Show/hide shortcut:** the app responds to `SIGUSR1` (`pkill -SIGUSR1 topmonitoring`) by toggling bar visibility — bind this command to a keyboard shortcut in your desktop environment's settings, since a global hotkey can't be registered directly by the app.
- **Hardware Sensors:** button in Settings → full list of voltage/current/wattage/fan/temperature readings per chip, refreshed live, with its own Close button.
- **Theme & color:** dark/light dropdown, live color picker, or custom CSS (target the `.topbar` selector). Settings and the Hardware Sensors window are unaffected by the topbar's theme — only the bar itself changes color.
- **Presets:** save your current look as a named preset and re-apply it later with one click.
- **Auto-dim:** when enabled, the bar fades to low opacity a moment after the pointer leaves it, and returns to full opacity on hover.
- **Autostart:** the **Enable autostart** button writes `~/.config/autostart/topmonitoring.desktop` (with the executable path safely quoted).
- **Portability:** use Export/Import to move `config.toml` to another machine.
- **Quit:** use the **⏻ Quit TopMonitoring** button in Settings (there is no window titlebar close button by design, since the bar is a borderless dock).

## 10. Packaging & Distribution

### 10.1 Optimized release binary

```bash
cargo build --release   # LTO + strip, small binary
```

### 10.2 .deb package

```bash
cargo install cargo-deb
cargo deb                # -> target/debian/topmonitoring_*.deb
sudo dpkg -i target/debian/topmonitoring_*.deb
```

### 10.3 AppImage (optional)

Use `linuxdeploy` with the GTK plugin to bundle `libgtk-4` and its runtime dependencies.

### 10.4 Step-by-step .deb walkthrough

1. Install the packaging tool once: `cargo install cargo-deb`
2. Make sure `Cargo.toml` has a `[package.metadata.deb]` section (maintainer, description, runtime dependencies, recommended optional tools, and the list of files to bundle, including the icon).
3. Build the package: `cargo deb` (this runs an optimized release build first, then assembles the `.deb`).
4. Inspect the result before publishing:
   - `dpkg -I target/debian/topmonitoring_*.deb` (metadata)
   - `dpkg -c target/debian/topmonitoring_*.deb` (file listing)
5. Test locally: `sudo dpkg -i target/debian/topmonitoring_*.deb`, then run `topmonitoring` from a normal terminal (not from inside the project folder).
6. Remove the test install if needed: `sudo dpkg -r topmonitoring`.

> **No `gtk4-layer-shell` on your system?** Build a variant without the Wayland dock feature: add a `[package.metadata.deb.variants.no-wayland]` block with `depends = "libgtk-4-1, libsensors5"`, then run `cargo deb --no-default-features --variant no-wayland`.

### 10.5 Publishing a GitHub Release

1. Tag the commit: `git tag -a v1.3.0 -m "v1.3.0"` then `git push origin v1.3.0`.
2. On GitHub: Releases → Draft a new release → pick the tag.
3. Attach the generated `.deb` file as a release asset.
4. Paste the matching Changelog entry into the release notes.
5. Publish.

## 11. Troubleshooting

| Symptom | Cause |
|---|---|
| Bar doesn't dock / behaves like an overlay | Compositor without layer-shell support (GNOME) |
| Bar overlaps an existing panel | Offset too small |
| `TEMP 0°C` | Sensors not detected yet |
| `gpu_power` = n/a | GPU/driver doesn't expose power |
| `cpu_power` = n/a | RAPL needs read permission |
| Fan/voltage missing | Motherboard chip driver not loaded |
| Volume/Brightness slider missing or stuck at 0 | `wpctl`/`pactl` or `brightnessctl` not installed, or no backlight/mixer permission |
| Bluetooth/VPN metric shows n/a | `bluetoothctl` not installed, or no active VPN interface detected |
| SIGUSR1 shortcut does nothing | Desktop keyboard shortcut not bound to `pkill -SIGUSR1 topmonitoring`, or binary name differs |
| `pkg-config` can't find `gtk4-layer-shell-0` during `cargo deb` | Dev package not installed, or not available in your distro's repo — install `libgtk4-layer-shell-dev` or build a `no-wayland` variant |
| Launching the app again does nothing | Expected: TopMonitoring is single-instance and just re-focuses the existing bar instead of opening a duplicate |

## 12. Roadmap

- [ ] True system-tray integration (StatusNotifierItem/AppIndicator)
- [ ] Native Intel GPU utilization via `intel_gpu_top`
- [ ] Per-monitor independent bars
- [ ] Config schema versioning for safer migrations

## 13. Changelog

| Version | Changes |
|---|---|
| 1.3.0 | Added Volume and Brightness sliders with live OS sync and mute toggle, Bluetooth and VPN status metrics, a mini Process Manager, CSV history logging with a configurable interval, a GitHub release update checker, and a SIGUSR1 show/hide shortcut for custom keybindings; refreshed slider styling to match the active theme and fixed a Bluetooth icon that could fail to render without a Nerd Font |
| 1.2.0 | Added Quick action buttons (Lock, Screenshot, Shutdown-with-confirm), a Wi-Fi status metric, pending package updates count, MPRIS media now-playing, and external/removable disk detection |
| 1.1.1 | Fixed a regression where malformed generated CSS silently broke theme switching and custom fonts; critical metrics now blink instead of just changing color; regenerated the app icon with proper safe-zone padding to prevent cropping at small sizes |
| 1.1.0 | Stability pass (async custom modules, single settings window, live interval changes, corrupt-config recovery), GUI threshold editor, middle-click detail popups, saved appearance presets, colored/filled graphs, auto-dim, best-effort Intel GPU clock, full English codebase/UI/docs |
| 1.0.0 | Initial full release: X11/Wayland dock, complete metric set, hwmon sensor window, graphs, custom modules, live-apply settings, export/import, autostart, packaging |

See also [CHANGELOG.md](../CHANGELOG.md) for the Keep a Changelog–formatted version.

## 14. Professional Release Checklist

- [x] README with badges, screenshots section, and clear installation steps
- [x] MIT LICENSE
- [x] CHANGELOG.md following Keep a Changelog conventions
- [x] CONTRIBUTING.md
- [x] GitHub Actions CI (build + `cargo fmt --check` + `cargo clippy`)
- [x] Multi-size app icon (SVG + PNG set + favicon) with proper safe-zone padding
- [ ] CODE_OF_CONDUCT.md
- [ ] SECURITY.md (how to report vulnerabilities)
- [ ] GitHub issue templates (bug report, feature request) and a pull request template
- [ ] `.gitattributes` (normalize line endings, mark generated files)
- [ ] Repository description, topics (`rust`, `gtk4`, `system-monitor`, `linux`, `wayland`, `x11`), and a social preview image
- [ ] At least one tagged GitHub Release with a `.deb` asset attached
- [ ] Demo screenshot or short GIF in the README

---

License: MIT. Contributions and issues are welcome via the project repository.

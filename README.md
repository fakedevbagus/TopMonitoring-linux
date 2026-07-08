# TopMonitoring

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-orange?logo=rust)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Linux-informational?logo=linux)](https://github.com/fakedevbagus/TopMonitoring-linux)
[![CI](https://github.com/fakedevbagus/TopMonitoring-linux/actions/workflows/ci.yml/badge.svg)](https://github.com/fakedevbagus/TopMonitoring-linux/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/fakedevbagus/TopMonitoring-linux?include_prereleases)](https://github.com/fakedevbagus/TopMonitoring-linux/releases)
[![GitHub issues](https://img.shields.io/github/issues/fakedevbagus/TopMonitoring-linux)](https://github.com/fakedevbagus/TopMonitoring-linux/issues)

A native Linux **system-monitor topbar** built with Rust + GTK4. Unlike an overlay widget, TopMonitoring **reserves real screen space** (via `wlr-layer-shell` on Wayland or `_NET_WM_STRUT_PARTIAL` on X11), so fullscreen apps automatically shrink around it instead of being covered.

> Full technical reference: [`docs/TECHNICAL.md`](docs/TECHNICAL.md)

## Screenshot

<img width="1560" height="941" alt="Screenshot_2026-07-08_21-07-33" src="https://github.com/user-attachments/assets/aa721469-ed03-4615-b374-89b5c2e8d12d" />

## Features

- **Full-width topbar** — top or bottom position, multi-monitor support, resizable height
- **Deep system metrics** — CPU, RAM, swap, temperature, fan, GPU (NVIDIA/AMD/Intel), disk (incl. external/removable drives), disk I/O, network, uptime, load, battery, Wi-Fi, pending package updates, media now-playing
- **Hardware Sensors window** — every `hwmon` channel: voltage, current, wattage, fan, temperature
- **Colored graphs** — filled sparkline history for CPU/RAM/Network
- **System controls** — Volume & Brightness sliders with live OS sync, mute toggle, Bluetooth & VPN status
- **Quick actions** — optional Lock / Screenshot / Shutdown-with-confirm buttons
- **Mini Process Manager** — top CPU/RAM processes, sortable, live refresh
- **History logging** — CSV log of CPU%/RAM%/temperature on a configurable interval
- **Update checker** — compares your version against the latest GitHub release
- **Show/hide shortcut** — responds to `SIGUSR1` for custom keybinding
- **Custom modules** — run any shell command and show its output on the bar
- **Fully customizable** — dark/light theme, live color picker, custom CSS, fonts, saved presets
- **Live-apply settings** — change anything, see it instantly, save when ready
- **Interaction** — left-click a metric to launch an app, middle-click for a detail popup, desktop notifications on critical thresholds

## Installation

### Option A: install the `.deb` package

```bash
sudo dpkg -i topmonitoring_*.deb
```

### Option B: build from source

```bash
# Debian/Ubuntu
sudo apt install -y build-essential pkg-config libgtk-4-dev \
  libgtk4-layer-shell-dev lm-sensors libsensors-dev
# Fedora
sudo dnf install -y gcc pkgconf-pkg-config gtk4-devel gtk4-layer-shell-devel lm_sensors lm_sensors-devel
# Arch
sudo pacman -S --needed base-devel gtk4 gtk4-layer-shell lm_sensors

sudo sensors-detect --auto && sensors   # calibrate hardware sensors

git clone https://github.com/fakedevbagus/TopMonitoring-linux.git
cd TopMonitoring-linux
cargo build --release
sudo install -Dm755 target/release/topmonitoring /usr/local/bin/topmonitoring
```

**Building without Wayland support** (no `gtk4-layer-shell` available on your system): use X11 struts only.

```bash
cargo build --release --no-default-features
```

### Optional runtime tools

Each control degrades gracefully (shows `n/a` or hides itself) if its tool isn't installed:

| Feature | Requires |
|---|---|
| Volume slider | `wpctl` (PipeWire) or `pactl` (PulseAudio) |
| Brightness slider | `brightnessctl` |
| Bluetooth status | `bluetoothctl` (BlueZ) |
| Wi-Fi status | `nmcli` (NetworkManager) |
| Media now-playing | `playerctl` |
| Update checker | `curl` |
| Screenshot quick action | `gnome-screenshot` / `xfce4-screenshooter` / `spectacle` / `flameshot` |
| Lock / Shutdown quick actions | `loginctl` / `systemctl` |

## Usage

Right-click the bar (or click the ⚙ icon) to open **Settings**. Every change applies live; click **💾 Save** to persist it. See the full [Usage Guide](docs/TECHNICAL.md#9-usage-guide) for details on every feature.

## Configuration

Config lives at `~/.config/topmonitoring/config.toml` and is created automatically on first run. Full field reference: [`docs/TECHNICAL.md`](docs/TECHNICAL.md#6-configuration-reference).

## Packaging

```bash
cargo install cargo-deb
cargo deb   # -> target/debian/topmonitoring_*.deb
```

See [`docs/TECHNICAL.md`](docs/TECHNICAL.md#10-packaging--distribution) for the full packaging and GitHub Release walkthrough, including how to build a `no-wayland` variant.

## Changelog

See [`CHANGELOG.md`](CHANGELOG.md).

## License

[MIT](LICENSE)

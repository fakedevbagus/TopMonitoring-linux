# 📊 TopMonitoring

![CI](https://github.com/satriabagusanjaya/topmonitoring-linux/actions/workflows/ci.yml/badge.svg)
![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-stable-orange.svg)
![Platform](https://img.shields.io/badge/platform-Linux-lightgrey.svg)
![Release](https://img.shields.io/github/v/release/satriabagusanjaya/topmonitoring-linux?include_prereleases)

A native Linux **system-monitor topbar** that docks to a screen edge like a
real panel — forcing fullscreen windows to shrink around it instead of
overlaying them — with deep hardware sensors and full live customization.

Built with **Rust + GTK4**. Works on both **Wayland** (layer-shell) and
**X11** (window struts).

![Screenshot](assets/screenshot.png)

## ✨ Features

- True dock behavior: Wayland `wlr-layer-shell` and X11 `_NET_WM_STRUT_PARTIAL`
- Deep metrics: CPU, RAM, swap, temperature, fan, GPU (NVIDIA/AMD/Intel),
  disk, disk I/O, network, uptime, load, battery, and more
- **Hardware Sensors** window: every voltage/current/wattage/fan/temperature
  channel from `hwmon`, grouped by chip
- Colored, filled sparkline graphs that shift color near critical levels
- Metrics **blink** when they cross your critical threshold
- Left-click a metric to launch an app, middle-click for a detail popup
- Custom shell-command modules, run safely off the UI thread
- Live-apply Settings: every change previews instantly, one Save button,
  closing without saving reverts automatically
- Saved appearance presets, live color picker, custom CSS, font & size
- Export/import config, one-click autostart, multi-monitor support

## 🚀 Installation

### Option A — Debian/Ubuntu `.deb` package
Download the latest `.deb` from [Releases](../../releases) and:
​
sudo dpkg -i topmonitoring_*.deb

### Option B — Build from source
​
Debian/Ubuntu
sudo apt install -y build-essential pkg-config libgtk-4-dev \
libgtk4-layer-shell-dev lm-sensors libsensors-dev
sudo sensors-detect --auto
git clone https://github.com/satriabagusanjaya/topmonitoring-linux.git
cd topmonitoring-linux
chmod +x install.sh && ./install.sh
See the [full documentation](docs/) for Fedora/Arch instructions and troubleshooting.

## ⚙️ Configuration

Everything is configurable from the GUI: click the **⚙** icon on the bar, or
**right-click** anywhere on it. Settings apply live; click **💾 Save** to
persist. The config file lives at `~/.config/topmonitoring/config.toml`.

## 📦 Packaging

​
cargo install cargo-deb
cargo deb   # -> target/debian/topmonitoring_*.deb

## 🤝 Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Please read our
[Code of Conduct](CODE_OF_CONDUCT.md) before participating.

## 🔒 Security

See [SECURITY.md](SECURITY.md) for how to report vulnerabilities.

## 📄 License

MIT © 2026 satriabagusanjaya — see [LICENSE](LICENSE).

## 🪟 Related

Looking for the Windows version? See
[topmonitoring](https://github.com/satriabagusanjaya/topmonitoring).
# TopMonitoring

**A native Linux topbar system monitor** — a true reserve-space dock (not an overlay), full-width, resizable, with deep hardware sensors and full live customization.

Built with **Rust + GTK4**.

---

## Features

- **True dock, not an overlay** — reserves real screen space via Wayland `wlr-layer-shell` (exclusive zone) or X11 `_NET_WM_STRUT_PARTIAL`, so fullscreen windows automatically shrink around it.
- **Full metric set** — CPU (usage, per-core, model, frequency, package power, Vcore), RAM/swap, temperature & fan, GPU (NVIDIA via NVML, AMD/Intel via sysfs: utilization, VRAM, temp, power, clock), disk usage + I/O (including external/removable drives), network rate + lifetime totals, uptime, load average, process count, battery, host/kernel/OS info, Wi-Fi SSID/status, pending package updates, and MPRIS "now playing" media info.
- **System controls, right from the bar** — Volume and Brightness sliders with live OS sync and a mute toggle, Bluetooth and VPN status, and optional Quick Action buttons (Lock screen, Screenshot, Shutdown-with-confirm).
- **Mini Process Manager** — a lightweight popup listing the top CPU/RAM-consuming processes, sortable, refreshing live.
- **History logging** — optionally logs CPU/RAM/temperature to a CSV file on a configurable interval, with a one-click shortcut to open the log folder.
- **Update checker** — checks GitHub Releases for a newer version.
- **Show/hide via keyboard shortcut** — the bar listens for `SIGUSR1` so you can bind a desktop keyboard shortcut to toggle it.
- **Deep hardware sensors** — a dedicated Hardware Sensors window reads every `hwmon` channel (voltage, current, wattage, fan, temperature).
- **Graphs** — colored, filled sparkline history for CPU / RAM / Network that shifts color near warning/critical thresholds.
- **Full customization** — dark/light theme, live color picker, custom CSS, font & size, saved appearance presets, auto-dim on idle.
- **Interaction** — left-click a metric to launch an app, middle-click for a detail popup, rich tooltips, blinking + desktop notifications on critical metrics.
- **Custom modules** — run any shell command and show its output on the bar, executed off the UI thread.
- **Live-apply settings** — every change applies immediately; a single Save button persists it. Export/import config, autostart installer, reset to defaults.

## Screenshot

_Add a screenshot or short GIF of the bar here._

## Installation

### Option 1 — `.deb` package (Debian/Ubuntu/Mint)

```bash
sudo dpkg -i topmonitoring_*.deb
```

### Option 2 — Build from source

**Install build dependencies:**

```bash
# Debian/Ubuntu/Mint
sudo apt install build-essential pkg-config libgtk-4-dev \
  libgtk4-layer-shell-dev lm-sensors libsensors-dev

# Fedora
sudo dnf install gcc pkgconf-pkg-config gtk4-devel gtk4-layer-shell-devel lm_sensors lm_sensors-devel

# Arch
sudo pacman -S --needed base-devel gtk4 gtk4-layer-shell lm_sensors
```

**Calibrate sensors (required for temperature readings):**

```bash
sudo sensors-detect --auto
sensors   # should print CPU temperature, fan, voltage
```

**Build & install:**

```bash
git clone <repo-url> && cd topmonitoring
cargo build --release
sudo install -Dm755 target/release/topmonitoring /usr/local/bin/topmonitoring
```

On a compositor without Wayland layer-shell support (or if `libgtk4-layer-shell-dev` isn't available), build without it:

```bash
cargo build --release --no-default-features
```

### Optional runtime tools (for full feature coverage)

TopMonitoring degrades gracefully (showing `n/a` or hiding the control) when a tool below isn't installed:

| Feature | Tool needed |
|---|---|
| Volume slider | `wpctl` (PipeWire, usually preinstalled) or `pactl` (PulseAudio) |
| Brightness slider | `brightnessctl` (or `/sys/class/backlight` access) |
| Bluetooth status | `bluetoothctl` (BlueZ) |
| Wi-Fi SSID/status | `nmcli` (NetworkManager) |
| Media now-playing | `playerctl` |
| Update checker | `curl` |
| Screenshot quick action | `gnome-screenshot`, `xfce4-screenshooter`, `spectacle`, or `flameshot` |
| Lock/Shutdown quick actions | `loginctl` / `systemctl` |

## Usage

- **Open Settings:** click the ⚙ icon on the bar, or right-click anywhere on it.
- **Live-apply:** changes apply immediately; click **💾 Save** to persist them.
- **Left-click a metric:** launches the app configured for it.
- **Middle-click a metric:** opens a detail popup (per-core CPU, per-sensor temps, per-interface network, graph min/avg/max, etc).
- **Volume/Brightness:** drag the sliders on the bar directly; click the speaker icon to toggle mute.
- **Process Manager:** Settings → "Open Process Manager…".
- **History logging:** Settings → enable "Log metrics history to CSV", set the interval, and use "Open history log folder" to find `history.csv`.
- **Update checker:** Settings → "Check for Updates…".
- **Show/hide shortcut:** bind a custom keyboard shortcut in your desktop environment's settings to run:
  ```bash
  pkill -SIGUSR1 topmonitoring
  ```
  Each press toggles the bar's visibility.
- **Autostart:** Settings → "Enable autostart" (writes `~/.config/autostart/topmonitoring.desktop`).
- **Portability:** Export/Import moves `config.toml` between machines.
- **Quit:** Settings → "⏻ Quit TopMonitoring" (there's no titlebar close button by design, since the bar is a borderless dock).

## Configuration

Config lives at `~/.config/topmonitoring/config.toml` (created automatically). If it becomes corrupted, TopMonitoring backs it up as `config.toml.bak` and falls back to defaults rather than failing to start.

See the full field reference in the project's technical documentation.

## Packaging

```bash
cargo install cargo-deb
cargo deb   # -> target/debian/topmonitoring_*.deb
```

## License

MIT. Contributions and issues are welcome via the project repository.

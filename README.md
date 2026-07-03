![CI](https://github.com/satriabagusanjaya/topmonitoring/actions/workflows/ci.yml/badge.svg)
![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-stable-orange.svg)
![Platform](https://img.shields.io/badge/platform-Linux-lightgrey.svg)

# 📊 TopMonitoring
Topbar **system monitor native** untuk Linux — nempel di tepi layar sebagai
*dock* sejati (memaksa window fullscreen menyusut, bukan overlay), penuh metrik
& sensor ala LibreHardwareMonitor, dan sepenuhnya bisa dikustomisasi.

Dibangun dengan **Rust + GTK4**.

## ✨ Fitur
- Dock sejati: Wayland (`wlr-layer-shell`) & X11 (`_NET_WM_STRUT_PARTIAL`)
- Metrik lengkap: CPU, RAM, swap, suhu, fan, GPU (util/temp/VRAM/power/clock),
  disk, disk I/O, network, uptime, load, baterai, dll
- Jendela **Sensor Hardware**: seluruh sensor `hwmon` (voltase/arus/watt/fan/suhu)
- Sparkline riwayat CPU/RAM/Network
- Kustomisasi: tema gelap/terang, color picker live, CSS sendiri, font
- Klik metrik → jalankan aplikasi, notifikasi kritis, modul custom (shell)
- Pengaturan **live-apply** + Simpan, export/import config, autostart, multi-monitor

## 🚀 Instalasi
1. Dependency sistem (Debian/Ubuntu)
sudo apt install -y build-essential pkg-config libgtk-4-dev \
libgtk4-layer-shell-dev lm-sensors libsensors-dev
sudo sensors-detect --auto
2. Build & pasang
chmod +x install.sh && ./install.sh
Lihat `Dokumentasi Teknis` untuk distro lain (Fedora/Arch).

## ⚙️ Konfigurasi
File otomatis: `~/.config/topmonitoring/config.toml`.
Semua bisa diatur lewat GUI (klik ⚙ atau klik kanan pada bar).

## 📦 Packaging
​
cargo install cargo-deb && cargo deb   # -> target/debian/*.deb

## 📄 Lisensi
MIT © 2026 satriabagusanjaya
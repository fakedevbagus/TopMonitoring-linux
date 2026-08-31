# TopMonitoring 2.0 — Audit & Major Upgrade Plan

> **Status:** rencana saja; belum ada implementasi kode.
>
> **Target:** stabilisasi v1.3.x lalu major upgrade v2.0 yang lebih cepat, aman,
> indah, mudah dipakai, dan sangat dapat dikustomisasi.
>
> **Snapshot audit:** arsip `TopMonitoring-linux-main.zip`, package v1.3.0.

## 1. Keputusan eksekutif

TopMonitoring sudah memiliki cakupan metrik yang kuat untuk proyek berukuran
kecil. Nilai utamanya jelas: topbar native GTK4, dukungan Wayland/X11, sensor
hardware, grafik, kontrol sistem, dan modul command.

Namun, v1.3.0 belum aman untuk langsung ditambah fitur besar. Beberapa masalah
fondasi akan makin mahal jika UI baru dibangun di atas struktur saat ini:

1. Banyak pembacaan metrik dan proses eksternal berjalan di GTK main thread.
   Dampaknya adalah freeze berkala dan overhead tinggi.
2. `src/main.rs` sudah menjadi monolit 2.489 baris; Settings sendiri sekitar 638
   baris dan `update_metric` sekitar 329 baris.
3. Ada fitur yang didokumentasikan tetapi tidak tersambung atau tidak bekerja
   sesuai klaim, terutama Quick Actions, auto update check, ikon mute, dan ikon
   Bluetooth.
4. Import konfigurasi dapat mengaktifkan custom shell module secara otomatis.
   Belum ada trust review, timeout, pembatas output, atau concurrency limit.
5. Metadata paket `.deb` tampaknya masuk ke varian `no-wayland`, sehingga paket
   default berisiko kehilangan metadata dan aset yang dimaksudkan.
6. Belum ada test suite; verifikasi parser, migrasi config, scheduler, dan
   platform behavior tidak tersedia.

**Rekomendasi utama:** jangan melakukan redesign kosmetik di `main.rs` saat ini.
Kerjakan hotfix v1.3.1 terlebih dahulu, lalu bangun v2.0 dengan scheduler
asinkron, provider modular, config schema v2, dan editor layout berbasis modul.

## 2. Ruang lingkup dan metode audit

### 2.1 Yang diperiksa

- Seluruh 32 file dalam arsip.
- `Cargo.toml`, `Cargo.lock`, CI, installer, desktop entry, dokumentasi, dan aset.
- `src/main.rs` dan `src/config.rs` secara menyeluruh.
- Screenshot UI 1560×941.
- Kesesuaian antara README/changelog dengan implementasi.
- Arsitektur, correctness, performa, keamanan, UX, visual, accessibility,
  packaging, maintainability, dan testability.

### 2.2 Statistik snapshot

| Item | Nilai |
| --- | ---: |
| Total file | 32 |
| Source/config/docs yang dihitung | 3.436 baris |
| `src/main.rs` | 2.489 baris |
| `src/config.rs` | 326 baris |
| Direct dependencies | 9 |
| Packages di lockfile | 144 |
| Pemanggilan `Command::new` | 23 |
| Pemanggilan `apply_all()` | 35 |
| Test Rust yang ditemukan | 0 |
| File sementara yang ikut arsip | `src/main.rs.crdownload` |

### 2.3 Batas audit

- Sandbox audit tidak menyediakan `rustc`/`cargo`, sehingga build, Clippy,
  runtime GTK, dan pengujian pada hardware asli belum dapat dijalankan.
- `bash -n install.sh uninstall.sh` lulus.
- URL GitHub publik tidak dapat dimuat langsung saat audit; sumber utama adalah
  arsip yang diberikan pengguna.
- Temuan runtime yang memerlukan Ubuntu/Wayland/X11 nyata harus dikonfirmasi
  lagi di fase baseline sebelum perubahan kode.

## 3. Peta arsitektur saat ini

```text
Application::activate
└── build_bar
    ├── load Config TOML
    ├── create GTK window + Wayland layer shell / X11 strut
    ├── rebuild all metric widgets
    ├── start one GLib polling timer
    │   ├── refresh sysinfo state
    │   ├── read sysfs/procfs
    │   ├── run several external commands
    │   ├── update every widget
    │   └── optionally append CSV
    └── open one large Settings window
```

State UI, pengambilan data, command execution, rendering, konfigurasi, dan
platform integration masih bercampur di satu file dan satu alur polling.

## 4. Scorecard kualitatif

Skala 1–5; ini adalah penilaian snapshot, bukan nilai absolut proyek.

| Area | Skor | Ringkasan |
| --- | ---: | --- |
| Cakupan produk | 4.0 | Banyak metrik dan integrasi sudah dicoba |
| Visual topbar | 2.5 | Ringkas, tetapi padat dan kurang hierarki |
| UX Settings | 1.5 | Satu form panjang, sulit dipindai dan dikelola |
| Fleksibilitas kustomisasi | 2.5 | Ada CSS/preset/module, tetapi modelnya masih dangkal |
| Arsitektur | 1.5 | Monolit dan coupling tinggi |
| Responsivitas | 1.0 | Banyak I/O dan subprocess sinkron di main thread |
| Correctness | 2.0 | Ada beberapa mismatch dan bug terverifikasi statis |
| Keamanan lokal | 2.0 | Shell module belum punya trust boundary/resource limit |
| Accessibility | 1.5 | Ikon/glyph, warna, blink, dan target kecil belum ditangani |
| Testing | 1.0 | Tidak ada test suite |
| Packaging/release | 1.5 | Metadata varian salah lingkup; installer belum lengkap |
| Dokumentasi | 3.0 | Cukup lengkap, tetapi beberapa klaim sudah tidak sinkron |

## 5. Temuan audit terprioritas

### 5.1 P0 — wajib dibenahi sebelum feature expansion

#### TM-001 — GTK main thread melakukan pekerjaan blocking

**Bukti:** timer di `src/main.rs:202-235` memanggil `update_slot` dan
`update_metric` langsung. Jalur itu menjalankan `iwgetid`/`nmcli`, `apt`, tiga
pemanggilan `playerctl`, `wpctl`/`pactl`, `brightnessctl`, `bluetoothctl`, scan
`/sys`, scan `/proc`, dan penulisan CSV.

**Dampak:** bar dapat tersendat setiap refresh; `apt list --upgradable` dapat
membekukan UI cukup lama. Interval 200 ms memperburuk overhead.

**Perbaikan:** semua provider I/O dipindahkan ke scheduler worker. Main thread
hanya menerima snapshot immutable dan merender perubahan yang benar-benar
berbeda.

#### TM-002 — Quick Actions hanya memiliki config dan switch

**Bukti:** `qa_lock`, `qa_screenshot`, dan `qa_shutdown` ada di
`src/config.rs:77-83` serta UI di `src/main.rs:2017-2043`, tetapi tidak ada
renderer/action untuk `loginctl`, `systemctl`, atau tool screenshot di source.

**Dampak:** fitur yang diklaim README/changelog tidak muncul atau tidak bekerja.

**Perbaikan hotfix:** implementasikan dengan capability detection dan dialog
konfirmasi, atau sembunyikan switch serta koreksi dokumentasi sampai siap.

#### TM-003 — Imported config dapat menjalankan shell command tanpa trust review

**Bukti:** import mengganti seluruh `Config` di `src/main.rs:2238-2252`.
Custom module aktif kemudian dijalankan otomatis setiap tick melalui
`sh -c` di `src/main.rs:695-710` dan `737-741`.

**Risiko:** config yang dibagikan dapat mengeksekusi command arbitrer sebagai
user. Command tidak memiliki timeout, batas output, limit jumlah proses, atau
kill-on-disable.

**Perbaikan:** semua command dari import dinonaktifkan sampai pengguna melihat
review screen. Tambahkan timeout, output cap, concurrency cap, process-group
termination, exit status, dan status error yang terlihat.

#### TM-004 — Metadata paket `.deb` salah lingkup

**Bukti:** `[package.metadata.deb]` kosong lalu seluruh `depends`, `assets`, dan
metadata berada di `[package.metadata.deb.variants.no-wayland]` pada
`Cargo.toml:27-50`.

**Dampak:** `cargo deb` default berisiko tidak membawa desktop entry, ikon,
dokumentasi, dan dependency metadata yang diharapkan.

**Perbaikan:** pindahkan metadata umum ke `[package.metadata.deb]`; varian hanya
mengoverride dependency yang berbeda. Tambahkan smoke test isi paket.

#### TM-005 — Tidak ada test suite dan build audit belum reproducible

**Bukti:** tidak ditemukan `#[test]`, `mod tests`, atau direktori tests.
CI hanya menjalankan format, Clippy, dan default release build.

**Dampak:** parser `/proc`/`sysfs`, migrasi config, rate calculation, dan
platform behavior mudah regresi.

**Catatan:** fungsi private `count_amd_gpus` hanya muncul pada definisinya;
`read_disk_io` tidak memiliki call site aktif. Dengan `-D warnings`, ini patut
diperiksa karena dapat mematahkan CI.

### 5.2 P1 — correctness, reliability, dan performa

#### TM-006 — Import tersimpan ke disk tetapi live state dikembalikan ke config lama

Import memanggil `save()` lalu menutup Settings (`src/main.rs:2246-2252`). Close
handler selalu mengembalikan snapshot awal (`2465-2474`). Akibatnya UI dapat
kembali ke state lama, sedangkan restart berikutnya membaca hasil import.

**Fix:** setelah import sukses, update `original`, refresh seluruh form, dan
jangan menutup tanpa status eksplisit. Lebih baik sediakan Preview → Review
commands → Apply.

#### TM-007 — Update checker tidak sesuai repo dan bukan auto-check

- URL di `src/main.rs:1547` mengarah ke
  `satriabagusanjaya/topmonitoring-linux`, bukan repo pengguna
  `fakedevbagus/TopMonitoring-linux`.
- `check_updates` hanya dibaca/ditulis oleh switch; tidak ada startup call.
- Versi dibandingkan dengan `latest != current`, bukan semantic version.
- JSON diparsing dengan split string.

**Fix:** satu konstanta repository URL, parser JSON, semver comparison, startup
check yang benar-benar menghormati opt-in, cache, dan tombol buka release.
Jangan auto-install sebagai root.

#### TM-008 — Network rate salah ketika interval bukan 1 detik

`Networks::received()/transmitted()` ditampilkan langsung sebagai `/s` di
`src/main.rs:953-977`, padahal nilai delta perlu dinormalisasi terhadap elapsed
time. `net_graph` juga mengabaikan `net_iface`, dan lifetime total mengabaikan
pilihan interface.

**Fix:** simpan monotonic timestamp per provider, hitung bytes/second, gunakan
interface filter konsisten, dan uji dengan fixture serta interval 200/1000/5000
ms.

#### TM-009 — Auto-dim memiliki race

Timer leave dibuat di `src/main.rs:136-152`, tetapi handler enter hanya menghapus
class dan tidak membatalkan timer. Bar bisa redup setelah pointer sudah masuk
kembali. Menonaktifkan fitur ketika sudah redup juga tidak segera menghapus
class.

**Fix:** simpan satu cancellable timer yang dapat diakses enter/leave/toggle.
Tambahkan reduced-motion behavior.

#### TM-010 — Proteksi drag slider tidak pernah diaktifkan

`src/main.rs:727` memeriksa CSS class `dragging`, tetapi tidak ada kode yang
menambah/menghapus class tersebut. Poll dapat melawan drag user. Setter juga
men-spawn command untuk setiap perubahan kecil.

**Fix:** gunakan GTK gesture state nyata, debounce 50–100 ms, optimistic UI, dan
satu write in-flight per device.

#### TM-011 — Ikon mute tidak disinkronkan dan Bluetooth masih memakai glyph PUA

- Mute button dibuat di `src/main.rs:553-573`, tetapi `read_volume()` membuang
  nilai `_muted`; tombol tidak disimpan di `SlotWidget` dan ikon tidak berubah.
- Bluetooth connected state masih menambahkan `\u{f294}` di
  `src/main.rs:1048`, bertentangan dengan changelog yang menyatakan glyph Nerd
  Font sudah dihapus.

**Fix:** gunakan GTK symbolic icon dengan fallback teks; simpan reference tombol
mute dan render state `muted/low/high`.

#### TM-012 — Config save tidak atomic dan seluruh error disembunyikan

`src/config.rs:228-237` mengabaikan kegagalan `create_dir_all`, serialisasi, dan
write. Tombol tetap menampilkan “Saved”. Tidak ada schema version, validasi,
clamp, dedupe ID, atau migrasi formal.

**Fix:** tulis temporary file → `fsync` → atomic rename; return `Result`; tampilkan
toast error; tambahkan `schema_version`, validator, backup rotation, dan migrasi
v1 → v2.

#### TM-013 — X11 multi-monitor dapat underflow pada koordinat negatif

`mx`, `my` bertipe signed lalu dicast ke `u32` di `src/main.rs:399-410`.
Monitor di kiri/atas primary lazim memiliki koordinat negatif.

**Fix:** hitung strut terhadap root screen secara signed, clamp setelah validasi,
dan uji layout monitor kiri/kanan/atas/bawah.

#### TM-014 — `apply_all()` terlalu mahal

Perubahan satu karakter pada font, command, CSS, atau label dapat memanggil
`apply_all()`, membangun ulang semua widget dan me-restart polling timer. Ada 35
call site.

**Fix:** pisahkan change domains:

- style token update;
- layout diff;
- module config update;
- scheduler interval update;
- platform geometry update.

Gunakan debounce untuk text editor dan preview transaksi lokal.

#### TM-015 — Base CSS dan custom CSS berada dalam provider yang sama

`build_css` menyisipkan nilai font/background mentah lalu menambahkan custom CSS.
Satu CSS invalid dapat merusak style utama atau menghasilkan UI tak terbaca.

**Fix:** provider terpisah untuk base/theme/user override, validation preview,
Reset visual, safe mode, dan tombol “restore last working style”.

#### TM-016 — History CSV sinkron dan tumbuh tanpa batas

Penulisan berjalan dari polling UI (`src/main.rs:1526-1539`). Tidak ada retention,
rotation, ukuran maksimum, schema metadata, atau handling disk-full. Temperatur
history memakai sensor positif pertama, sedangkan bar memakai maksimum.

**Fix:** writer worker + SQLite ring/retention, downsampling, export CSV, dan
sensor source eksplisit.

#### TM-017 — External command fallback hanya memeriksa spawn

`set_volume`, `toggle_mute`, dan `set_brightness` menganggap `spawn().is_ok()`
sebagai sukses. Jika binary ada tetapi command gagal, fallback tidak dijalankan.

**Fix:** adapter dengan capability probe, await exit status, error typing, dan
fallback hanya setelah kegagalan aktual.

#### TM-018 — Polling cadence tidak sesuai karakter provider

Media, volume, brightness, Bluetooth, Wi-Fi, dan package updates menggunakan
poll utama yang sama. Refresh 200 ms dapat memanggil command sangat sering;
refresh 5 detik membuat clock/slider terasa lambat.

**Fix:** cadence per provider dan event-driven subscription bila tersedia.
Contoh: clock 1 s, CPU 1 s, volume event, media MPRIS event, battery 5 s,
package update 30 menit.

### 5.3 P2 — UX, visual, accessibility, dan repo quality

#### TM-019 — Settings adalah satu form panjang tanpa information architecture

Screenshot menunjukkan label/control panjang dalam satu kolom, hierarki lemah,
dan banyak scroll. Daftar 38 metrik memakai field dan tombol panah kecil per
baris.

**Fix:** sidebar kategori, search, module cards, drag-and-drop, sticky action bar,
inline validation, dan preview topbar permanen.

#### TM-020 — Topbar tidak punya overflow/adaptive layout

Semua modul ditempatkan pada satu center box. Konten panjang seperti media,
mount disk, SSID, dan custom output dapat menabrak ruang pada layar sempit atau
scale factor tinggi.

**Fix:** tiga zona, min/max width, ellipsize, priority, compact variant, overflow
menu, dan breakpoint berdasarkan available width.

#### TM-021 — Accessibility belum menjadi bagian model UI

- Ikon gear, panah, dan delete tidak semuanya memiliki accessible label.
- Status kritis bergantung pada warna dan blink 500 ms.
- Tidak ada reduced-motion option.
- Target interaktif di bar sangat kecil.
- Private-use glyph bergantung font tertentu.

**Fix:** symbolic icon + fallback, tooltip dan accessible name, bentuk/ikon selain
warna, opsi non-blink, keyboard focus yang terlihat, serta WCAG AA untuk
Settings/popover.

#### TM-022 — Model kustomisasi belum cukup leluasa

User hanya dapat rename, enable, reorder built-in metric, satu command klik,
threshold, global CSS, dan custom command text. Belum ada zona, group, separator,
format template, per-module style, per-module interval, conditional class,
action mouse berbeda, truncation, atau per-monitor profile.

#### TM-023 — Beberapa nilai kurang presisi/kurang jelas

- `TEMP` adalah maksimum seluruh components, bukan CPU tertentu.
- CPU frequency hanya core pertama.
- GPU index lintas NVIDIA/AMD/Intel tidak memiliki identitas stabil.
- VPN hanya heuristic nama interface.
- Package update hanya APT.
- `human_rate` tidak memiliki bytes/s dan label unit kurang konsisten.
- Grafik CPU/RAM memakai threshold fixed, bukan override milik modul.

**Fix:** provider menghasilkan typed values dengan source identity, unit,
quality/status, dan timestamp. User memilih sensor/device secara eksplisit.

#### TM-024 — Process Manager belum benar-benar “manager”

Saat ini hanya daftar top 20 yang dibangun ulang tiap detik. Belum ada search,
PID, command, user, detail, kill/terminate, renice, confirmation, atau error
feedback.

#### TM-025 — Installer dan desktop integration belum lengkap

`install.sh` memasang binary dan desktop file, tetapi tidak memasang ikon,
menjalankan cache update, memeriksa dependency, atau menyediakan uninstall yang
simetris. Nama desktop file juga tidak selaras dengan application ID
`com.satria.topmonitoring`.

#### TM-026 — Repo hygiene dan dokumentasi tidak sinkron

- `src/main.rs.crdownload` ikut arsip.
- `style.css` tidak direferensikan source.
- `SECURITY.md` dan PR template kosong.
- Technical checklist masih menandai beberapa file yang sebenarnya sudah ada.
- Tidak ada release/package workflow, SBOM, atau dependency policy.

## 6. Arah produk v2.0

### 6.1 Prinsip produk

1. **Bar selalu ringan:** monitoring tool tidak boleh menjadi sumber beban.
2. **Customizable without CSS:** kebutuhan umum tersedia lewat GUI; CSS adalah
   advanced escape hatch.
3. **Progressive disclosure:** default sederhana, opsi tingkat lanjut tetap ada.
4. **Trust by design:** imported commands tidak pernah auto-run tanpa review.
5. **Capability-aware:** UI hanya menawarkan fitur yang didukung sistem.
6. **Cross-desktop:** tetap GTK4 murni untuk shell utama; jangan mengunci desain
   ke GNOME/libadwaita kecuali support matrix berubah.
7. **Accurate and explainable:** setiap nilai memiliki unit, source, timestamp,
   dan status error.

### 6.2 Default support matrix yang disarankan

| Platform | Status v2 |
| --- | --- |
| Ubuntu 24.04 LTS / 26.04 LTS | Tier 1 |
| Linux Mint 22+ | Tier 1 |
| KDE Wayland, Hyprland, Sway/wlroots | Layer-shell Tier 1 |
| X11: Cinnamon, XFCE, KDE | Strut Tier 1 |
| GNOME Wayland | Explicit floating fallback; jangan klaim reserved space |
| Fedora current / Arch | Tier 2, best effort |
| Ubuntu 22.04 | Legacy build bila GTK baseline diturunkan |

`gtk4` saat ini meminta feature `v4_12`; baseline OS harus ditulis eksplisit di
README dan CI.

## 7. Desain visual v2

### 7.1 Konsep visual

Nama konsep: **Calm Telemetry** — data padat tetapi tetap tenang, modular, dan
mudah dibaca.

- Hindari bar berupa satu kalimat panjang.
- Kelompokkan modul menjadi surface kecil atau gunakan separator halus.
- Gunakan angka tabular untuk nilai yang berubah.
- Ikon harus symbolic dan tidak tergantung Nerd Font.
- Status normal netral; warna hanya untuk state bermakna.
- Border tipis lebih diutamakan daripada shadow/glass.
- Animasi maksimal 120–180 ms dan menghormati reduced motion.

### 7.2 Design tokens awal

#### Dark

| Token | Nilai |
| --- | --- |
| Canvas | `#19191B` |
| Surface | `#222225` |
| Raised/hover | `#2C2C30` |
| Text primary | `#F5F5F5` |
| Text secondary | `#A1A1AA` |
| Border | `rgba(255,255,255,0.14)` |
| Accent | `#5E9FE8` |
| Positive | `#72BC8F` |
| Warning | `#EAC26B` |
| Critical | `#E97366` |

#### Light

| Token | Nilai |
| --- | --- |
| Canvas | `#FFFFFF` |
| Surface | `#F7F7F8` |
| Raised/hover | `#EEEEF0` |
| Text primary | `#262628` |
| Text secondary | `#6F6F76` |
| Border | `#DEDEE2` |
| Accent | `#2783DE` |
| Positive | `#46A171` |
| Warning | `#D5803B` |
| Critical | `#E56458` |

Semua pasangan foreground/background harus lolos WCAG AA untuk Settings dan
popover. Bar compact boleh lebih kecil, tetapi accessible name tetap wajib.

### 7.3 Varian bawaan

1. **Compact:** teks minimal, tinggi 28–30 px, tanpa card background.
2. **Balanced:** tinggi 34–36 px, grouping dan separator halus; default.
3. **Telemetry:** tinggi 40–44 px, mini graph dan secondary value.
4. **Terminal:** monospace, flat, tanpa animasi.

Preset menyimpan seluruh layout/profile, bukan hanya warna dan font.

### 7.4 Struktur Settings baru

```text
TopMonitoring Settings
├── Overview        status runtime, capabilities, live preview
├── Bars & Monitors per-monitor bar, edge, height, zones, overflow
├── Modules         catalog, search, drag-and-drop, module editor
├── Appearance      theme, tokens, typography, spacing, icon pack
├── Alerts          rule builder, cooldown, hysteresis, actions
├── History         retention, charts, export, privacy
├── Integrations    audio, network, Bluetooth, media, package backends
├── Profiles        create, clone, schedule, import/export
└── About & Debug   version, logs, diagnostics, update status
```

Gunakan sticky Save/Discard bar. Perubahan divisualisasikan di preview; apply ke
bar hanya melalui diff, bukan rebuild total setiap keystroke.

### 7.5 Layout editor

- Tiga zona: left, center, right.
- Drag-and-drop module dan group.
- Separator, spacer, flexible spacer, group/collapse.
- Width policy: content, fixed, min/max, stretch.
- Overflow policy: hide by priority, compact, scroll, atau menu.
- Preview width 1366, 1024, dan custom monitor width.
- Per-monitor override dan clone layout.
- Undo/redo selama sesi edit.

## 8. Model kustomisasi v2

Contoh berikut konseptual; syntax final ditetapkan melalui ADR sebelum coding.

```toml
schema_version = 2
active_profile = "default"

[[profiles]]
id = "default"
name = "Daily"

[[profiles.bars]]
monitor = "primary"
edge = "top"
height = 36
variant = "balanced"
overflow = "priority-menu"
left = ["clock", "workspace"]
center = ["cpu", "memory", "network"]
right = ["media", "audio", "tray"]

[[modules]]
id = "cpu"
kind = "builtin.cpu"
enabled = true

[modules.view]
template = "{icon} {usage:.0}%"
compact_template = "{usage:.0}%"
icon = "cpu-symbolic"
min_width = 64
max_width = 120
priority = 90

[modules.refresh]
mode = "interval"
interval_ms = 1000
timeout_ms = 1500

[[modules.rules]]
when = "usage >= 90 for 10s"
class = "critical"
notify = true
cooldown_s = 300
hysteresis = 5

[[modules.actions]]
gesture = "primary-click"
action = "open:processes"

[[modules.actions]]
gesture = "scroll"
action = "cycle:view"
```

### 8.1 Properti yang harus dapat diubah per modul

- Label, icon, icon pack, format template, unit, precision.
- Foreground, background, border, radius, padding, gap, opacity.
- Normal/warning/critical/disabled styles.
- Min/max width, ellipsize, compact view, overflow priority.
- Refresh mode, interval, timeout, cache TTL.
- Device/source selector.
- Tooltip layout dan popover content.
- Left/middle/right click, double click, scroll up/down.
- Alert rule, cooldown, hysteresis, sound, notification action.
- Visibility condition, misalnya battery-only atau SSID tertentu.

### 8.2 External/custom module protocol

Adopsi pola yang familiar seperti Waybar, tetapi tambahkan safety:

```json
{
  "text": "42%",
  "tooltip": "CPU package average: 42%",
  "class": ["normal"],
  "percentage": 42,
  "icon": "cpu-symbolic"
}
```

Wajib tersedia:

- `text`, `tooltip`, `class`, `percentage`, dan optional structured fields.
- Interval per module atau long-running stream mode.
- Timeout default 2 detik dan output cap default 16 KiB.
- Max concurrent process global dan per module.
- Kill process group saat timeout/disable/exit.
- `exec-if`, environment allowlist, working directory explicit.
- Error badge dan last-success timestamp.
- Import review yang menampilkan seluruh command dan action.
- Tombol “Trust and enable”; default imported command adalah disabled.

## 9. Arsitektur target

```text
src/
├── main.rs
├── app/
│   ├── lifecycle.rs
│   └── state.rs
├── core/
│   ├── config/
│   │   ├── schema_v2.rs
│   │   ├── migrate.rs
│   │   └── validate.rs
│   ├── metric.rs
│   ├── scheduler.rs
│   ├── rules.rs
│   ├── history.rs
│   └── capability.rs
├── providers/
│   ├── cpu.rs
│   ├── memory.rs
│   ├── thermal.rs
│   ├── gpu.rs
│   ├── disk.rs
│   ├── network.rs
│   ├── power.rs
│   ├── audio.rs
│   ├── bluetooth.rs
│   ├── media.rs
│   └── packages.rs
├── modules/
│   ├── builtin.rs
│   ├── external.rs
│   └── protocol.rs
├── platform/
│   ├── wayland.rs
│   ├── x11.rs
│   ├── notifications.rs
│   └── autostart.rs
├── ui/
│   ├── bar/
│   ├── settings/
│   ├── popovers/
│   └── theme/
└── storage/
    ├── atomic_config.rs
    └── sqlite_history.rs
```

### 9.1 Kontrak data

Provider menghasilkan snapshot typed, bukan langsung memodifikasi GTK widget.

```text
MetricSnapshot
- module_id
- timestamp_monotonic
- value + unit
- secondary values
- status: ok | unavailable | stale | error
- source/device identity
- quality/error detail
```

Renderer hanya memproses diff snapshot. UI tidak boleh membaca `/proc`, `/sys`,
file, network, atau menunggu subprocess.

### 9.2 Scheduler

- Satu worker runtime terkelola, bukan thread baru setiap tick.
- Cadence per provider dan request coalescing.
- Event-driven adapter untuk MPRIS, UPower, NetworkManager, dan BlueZ bila ada.
- Backoff saat provider gagal.
- Cancellation token saat module dinonaktifkan atau profile berubah.
- Shared cache agar satu source tidak dibaca berkali-kali.
- Metrics internal: duration, errors, skipped runs, last success.

### 9.3 Platform layer

- Deteksi backend dari GDK/display capability, bukan hanya environment variable.
- Wayland layer-shell capability check dan pesan fallback eksplisit.
- X11 strut calculation dengan signed geometry dan multi-monitor fixtures.
- Tidak ada command `sudo` dari UI.
- Operasi privilege tinggi dipisahkan ke helper opsional yang dapat diaudit;
  tidak diperlukan pada v2.0 awal.

## 10. Fitur baru yang direkomendasikan

### 10.1 Must-have untuk v2.0

1. **Per-monitor bars & profiles**
   - layout berbeda per monitor;
   - clone/sync layout;
   - profile berdasarkan docked/undocked atau power state.
2. **Visual layout editor**
   - drag-and-drop, zones, groups, spacers, overflow priority.
3. **Per-module styling dan template**
   - tanpa harus menulis CSS.
4. **Custom module v2**
   - JSON protocol, timeout, per-module interval, actions, trust review.
5. **Rule-based alerts**
   - duration, hysteresis, cooldown, notification action.
6. **History dashboard**
   - SQLite retention, chart 1h/24h/7d, CSV/JSON export.
7. **Rich detail popovers**
   - chart, min/avg/max, device/source, last update, diagnostics.
8. **Capability center**
   - menjelaskan dependency yang tersedia/tidak dan cara memperbaikinya.
9. **Safe mode**
   - start tanpa custom CSS/commands melalui CLI flag.
10. **Real update status**
    - semver-aware, opt-in, membuka release page, tanpa auto-root install.

### 10.2 High-value setelah v2.0 core stabil

- Media controls: play/pause, next/previous, player selector.
- Audio device selector, mute state, microphone state.
- Network details: IP, gateway, link speed, latency target opt-in.
- Package backends: APT, Snap, Flatpak dengan cadence terpisah.
- Battery health, charge rate, time remaining, power profile.
- Sensor picker dengan calibration offset dan custom display unit.
- GPU engine breakdown jika backend mendukung.
- Process detail, search, terminate/kill dengan confirmation dan permission error.
- Service status module untuk user-defined systemd units.
- StatusNotifierItem/AppIndicator tray integration.
- CLI/DBus control: switch profile, toggle bar, reload config, dump diagnostics.
- Theme/profile export tanpa command, serta bundle terpisah untuk trusted scripts.

### 10.3 Jangan dimasukkan ke v2.0 awal

- Marketplace yang mengunduh dan menjalankan plugin arbitrer.
- Remote monitoring/cloud account.
- Fan curve, voltage, atau hardware write yang memerlukan root.
- Auto-update installer dengan privilege.

Fitur tersebut memperbesar threat surface dan mengalihkan fokus dari kualitas
bar lokal.

## 11. Roadmap implementasi

Estimasi untuk satu developer penuh waktu; sesuaikan setelah baseline build di
Ubuntu nyata.

| Fase | Durasi | Output |
| --- | ---: | --- |
| 0. Baseline | 2–3 hari | Reproduce build, benchmark, issue list, ADR awal |
| 1. v1.3.1 stabilization | 1–2 minggu | P0/P1 hotfix, docs sync, package fix |
| 2. Core v2 | 2–3 minggu | Config v2, provider traits, scheduler, snapshots |
| 3. Visual shell | 2–3 minggu | Design tokens, new bar renderer, Settings IA, preview |
| 4. Customization | 2–3 minggu | Layout editor, profiles, module templates/styles/actions |
| 5. Rules & history | 2 minggu | Alert engine, SQLite history, charts/export |
| 6. Platform hardening | 1–2 minggu | Multi-monitor, Wayland/X11 matrix, packaging |
| 7. Beta/RC | 1–2 minggu | QA, accessibility, migration, release assets |

**Total realistis:** sekitar 11–17 person-weeks untuk v2.0 yang matang.

### 11.1 Urutan PR yang disarankan

1. `chore/baseline-ci-and-tests`
2. `fix/v1-runtime-blocking-and-feature-mismatch`
3. `fix/packaging-install-desktop-integration`
4. `feat/config-v2-migration-validation`
5. `refactor/provider-scheduler-snapshots`
6. `feat/design-tokens-and-bar-layout`
7. `feat/settings-navigation-and-live-preview`
8. `feat/custom-module-v2-sandbox-limits`
9. `feat/rules-and-history`
10. `feat/per-monitor-profiles`
11. `release/v2-beta-hardening`

Hindari satu PR besar yang sekaligus mengubah engine, visual, dan config.

## 12. Detail fase stabilisasi v1.3.1

Sebelum redesign, rilis patch yang dapat dipercaya.

### Wajib

- [ ] Perbaiki/hapus switch Quick Actions yang belum tersambung.
- [ ] Perbaiki repository URL dan semver update checker.
- [ ] Implementasikan startup check atau ubah label/dokumentasi.
- [ ] Perbaiki import/revert inconsistency.
- [ ] Nonaktifkan custom command dari config import sampai dikonfirmasi.
- [ ] Tambahkan timeout dan output cap custom command.
- [ ] Pindahkan command provider paling lambat dari main thread.
- [ ] Normalisasi network rate dengan elapsed time.
- [ ] Perbaiki auto-dim race dan slider drag/debounce.
- [ ] Sinkronkan mute icon; hapus private-use Bluetooth glyph.
- [ ] Return `Result` dari config save dan gunakan atomic write.
- [ ] Perbaiki `.deb` metadata dan installer icon.
- [ ] Hapus `.crdownload` serta dead/unused artifacts.
- [ ] Sinkronkan README, changelog, technical checklist.

### Test minimum untuk patch

- [ ] Config default/load/corrupt backup/save failure.
- [ ] Import config dengan command selalu disabled sebelum trust.
- [ ] Version comparison.
- [ ] Network rate pada tiga interval.
- [ ] `/proc/diskstats`, Wi-Fi, audio, Bluetooth parser fixtures.
- [ ] X11 negative monitor coordinates.
- [ ] Quick Actions capability and confirmation behavior.

## 13. Test, CI, dan release quality gates

### 13.1 Unit tests

- Parser fixture untuk setiap provider.
- Formatting/unit conversion.
- Alert hysteresis/cooldown.
- Config validation dan migrasi v1 → v2.
- External-module JSON protocol dan malformed output.
- Atomic-save recovery.

### 13.2 Integration tests

- Fake provider → scheduler → snapshot → renderer.
- Timeout/cancel/disable external process.
- Profile switch tanpa thread/process leak.
- Import review dan trust flow.
- History retention/export.

### 13.3 UI/visual tests

Render dan periksa manual/visual regression untuk:

- dark/light/high-contrast;
- 100%, 125%, 150%, dan 200% scaling;
- width 1024, 1366, 1920, ultrawide;
- overflow menu;
- empty/loading/stale/error/critical states;
- Settings desktop dan narrow window;
- reduced motion;
- keyboard-only navigation.

### 13.4 CI matrix

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`
- `cargo test --no-default-features`
- Ubuntu 24.04 and 26.04 build jobs.
- Wayland-feature and X11-only builds.
- `cargo audit` dan `cargo deny`.
- License/SBOM generation.
- `.deb` build + `dpkg-deb --contents` assertions.
- Desktop file validation dan AppStream validation.
- Pinned GitHub Actions commit SHAs untuk release workflow.

### 13.5 Performance budgets

Pada mesin referensi, default 1-second refresh:

- Idle CPU target `< 1%`; stretch target `< 0.5%`.
- Tidak ada GTK main-thread task blocking `> 50 ms`.
- Render diff p95 `< 8 ms`.
- RSS target `< 80 MiB` setelah 30 menit.
- Tidak ada pertumbuhan thread/FD/memory selama soak test 8 jam.
- External command default timeout `2 s`, output cap `16 KiB`.
- History write tidak terjadi di UI thread.

## 14. Security dan privacy gates

- Imported commands disabled by default.
- UI review menampilkan command, environment, interval, dan actions.
- Tidak ada shell untuk built-in provider.
- Tidak ada privileged action tanpa explicit confirmation.
- Log tidak menyimpan SSID, media title, process command, atau hostname kecuali
  user memilihnya.
- Retention history default terbatas dan dapat dihapus dari UI.
- Update check tidak mengirim telemetry selain request release biasa.
- Debug bundle meminta review sebelum export.
- Security policy berisi private reporting channel dan supported versions.

## 15. Definition of Done v2.0

v2.0 baru layak dirilis jika semua syarat berikut terpenuhi:

- [ ] Tidak ada filesystem/network/subprocess blocking di GTK main thread.
- [ ] Semua fitur README memiliki automated test atau capability-gated behavior.
- [ ] Config v1 bermigrasi tanpa kehilangan urutan, label, threshold, preset, dan
      custom module.
- [ ] Imported shell modules tidak berjalan sebelum user trust.
- [ ] Layout tidak overflow pada support widths/scales.
- [ ] Multi-monitor X11 dengan koordinat negatif dan Wayland layer-shell lulus.
- [ ] Settings dapat digunakan keyboard-only dan status tidak bergantung warna.
- [ ] Dark/light contrast lulus WCAG AA untuk Settings/popovers.
- [ ] CPU/memory/network/disk rates tervalidasi terhadap reference tools.
- [ ] `.deb` memasang binary, desktop entry, icon, license, dan dependency yang
      benar; uninstall simetris.
- [ ] Soak test 8 jam tanpa leak atau process accumulation.
- [ ] Dokumentasi, screenshots, changelog, security policy, dan release notes
      sinkron dengan binary.

## 16. Keputusan teknis default sebelum coding

Agar implementasi tidak menunggu terlalu banyak keputusan, gunakan default ini:

1. **Baseline OS:** Ubuntu 24.04 LTS; tambahkan 26.04 di CI.
2. **UI toolkit:** GTK4 murni dengan internal design tokens; libadwaita hanya
   dievaluasi untuk Settings jika tidak merusak cross-desktop consistency.
3. **Config:** TOML schema v2 + migrasi eksplisit.
4. **History:** SQLite lokal + export CSV/JSON.
5. **Built-ins:** native Rust provider; external command hanya untuk custom
   module.
6. **Import:** data visual boleh langsung dipreview; semua executable behavior
   disabled sampai trust.
7. **GNOME Wayland:** floating fallback yang jujur, bukan klaim reserved space.
8. **Release strategy:** v1.3.1 hotfix → v2.0 alpha → beta → RC.

## 17. Referensi produk/teknis

- Project: <https://github.com/fakedevbagus/TopMonitoring-linux>
- GTK4 documentation: <https://docs.gtk.org/gtk4/>
- GTK4 Rust book / libadwaita overview:
  <https://gtk-rs.org/gtk4-rs/stable/latest/book/libadwaita.html>
- Waybar custom module protocol inspiration:
  <https://man.archlinux.org/man/waybar-custom.5.en>
- Mission Center feature benchmark: <https://missioncenter.io/>

## 18. Kesimpulan

TopMonitoring tidak memerlukan penambahan fitur acak; proyek memerlukan
**re-platforming internal yang terukur**. Prioritas yang benar adalah:

1. stabilkan correctness, security, packaging, dan UI responsiveness;
2. pecah monolit menjadi provider/scheduler/snapshot architecture;
3. bangun ulang visual dan Settings di atas model layout/module v2;
4. baru tambahkan profiles, rules, history, per-monitor bars, dan richer
   integrations.

Dengan urutan ini, v2.0 dapat terlihat jauh lebih modern sekaligus lebih ringan,
akurat, aman, dan benar-benar dapat dikustomisasi tanpa menjadikan custom CSS
atau shell command sebagai satu-satunya jalan.

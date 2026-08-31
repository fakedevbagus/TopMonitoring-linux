# TopMonitoring 4 — Unified Control & Telemetry Major Upgrade Plan

**Status:** draft disetujui untuk dijadikan panduan pengembangan (approved 2026-09-01)
**Baseline:** `3.0.0-alpha.5-r16` (commit `fc70a7b` pada branch `major/v3`)
**Repo resmi:** <https://github.com/fakedevbagus/TopMonitoring-linux.git>
**Target utama:** Ubuntu/Linux desktop; paket `no-Wayland` (X11) tetap menjadi kelas
pertama; paket Wayland (`gtk4-layer-shell`) tetap didukung.
**Visi:** mengubah TopMonitoring dari "topbar system-monitor" menjadi **platform
monitoring + kontrol terpadu** untuk Linux desktop — tetap native Rust/GTK4,
tetap ringan, tetap satu proses.

---

## 1. Keputusan eksekutif

TopMonitoring 3 belum selesai (Phase 4–6 dari `MAJOR_UPGRADE_V3_PLAN.md` belum
dieksekusi, dan `3.0.0-alpha.5` belum lulus gate rilis). Membangun v4 di atas
source yang belum pernah dirilis akan menggandakan risiko regresi. Karena itu
rencana ini dibagi **dua tahap wajib berurutan**:

1. **Tahap 0 — Finalisasi v3.0.0:** tuntaskan Phase 4 (visual-effect engine +
   Theme Studio), Phase 5 (layout editor + aksesibilitas + action toast), dan
   Phase 6 (schema v3, hardening, rilis `.deb`). Hasil: baseline rilis yang
   dapat di-rollback dan menjadi dasar refactor besar.
2. **Tahap 1 — TopMonitoring 4.0** dalam enam fase:
   - Fase A — Solid Foundation (refactor modular + lib/bin + typed metrics);
   - Fase B — Persistence SQLite + dashboard terpadu;
   - Fase C — Multi-bar, profiles, dan control plane (CLI + D-Bus);
   - Fase D — Alerting engine;
   - Fase E — Backend expansion (SMART/NVMe, battery health, Snap, GPU detail);
   - Fase F — Developer ecosystem (endpoint read-only lokal) + rilis v4.0.0.

Iterasi pertama yang dieksekusi adalah **Tahap 0 → Fase A → Fase B** sesuai
persetujuan pemilik proyek.

---

## 2. Snapshot baseline (3.0.0-alpha.5-r16)

| Aspek | Nilai |
|---|---:|
| `Cargo.toml` | Rust 2021, GTK4 `0.9` + `gtk4-layer-shell` 0.4 (optional, fitur `wayland`), sysinfo 0.33, nvml-wrapper 0.10, x11rb 0.13 |
| Source total | 7 modul, 9.967 baris |
| `src/main.rs` | 5.818 baris (monolit) |
| Unit tests | 44 |
| Module registry | 38 built-in `ModuleDescriptor` |
| Thread model | `core-collector`, `fast-providers`, `slow-providers`, `actions`, `processes`, `sensors`, `hwmon-picker` |
| Config schema | `schema_version = 2` (flat ~35 field) |
| Release gates | `scripts/test-no-wayland.sh check|smoke|manual` + CI Wayland/X11 matrix |
| Git | baseline `fc70a7b` di branch `major/v3`; remote `origin` = repo GitHub |

Kekuatan yang wajib dipertahankan:

- worker isolation + immutable snapshots (`CoreSnapshot`/`ExternalSnapshot`);
- tidak ada provider I/O di GTK thread (dijaga gate statis `check_source.py`);
- keamanan command: timeout, cap output, process-group TERM/KILL, concurrency
  cap 4, trust opt-in untuk import;
- config resilience: `#[serde(default)]`, clamp, de-dup, atomic write `0600`;
- allocator layout murni & deterministik (`layout.rs`);
- `ProviderHealth` + diagnosa redacted (tanpa SSID/media title/username);
- gate rilis berjenjang dan dokumentasi yang sinkron per fase.

Temuan audit yang menjadi alasan v4 (hutang teknis):

| ID | Temuan | Lokasi |
|---|---|---|
| V4-F01 | `main.rs` monolit 5.818 baris; target arsitektur plan v3 §10 tidak pernah dieksekusi | `src/main.rs` |
| V4-F02 | Schema config masih v2 flat; tidak ada section `[layout]`/`[effects]`/`[storage]`/dll. | `src/config.rs` |
| V4-F03 | `animated_bg` masih men-recompile seluruh CSS di polling loop (temuan V3-008 belum selesai) | `main.rs:673-679` |
| V4-F04 | History CSV + widget sederhana; tidak ada agregasi jangka panjang | `main.rs:2615-2961` |
| V4-F05 | Tidak ada control plane (CLI/D-Bus); single-instance hanya lewat GTK | `main.rs:407-424` |
| V4-F06 | Tidak ada multi-bar / profil per skenario | `config.rs` |
| V4-F07 | Tidak ada mesin alerting; `ALERTS` masih thread-local ad-hoc | `main.rs:404` |
| V4-F08 | Windows process/sensor rebuild list tiap 200 ms (temuan V3-012) | `main.rs:3059-3211` |
| V4-F09 | `eprintln!` satu-satunya log; tidak ada struktur diagnosa | seluruh crate |
| V4-F10 | Backend expansion tertahan (SMART/NVMe, battery health, Snap) | `backend.rs` |

## 3. Tahap 0 — Finalisasi v3.0.0

### Step 0.1 · Gate alpha.5 (bukti sebelum lanjut)

1. `cargo fmt --check` (kalau gagal: `cargo fmt --all`).
2. `cargo test --locked --all-features` → **44/44**; ulangi `--no-default-features`.
3. `cargo clippy --locked --all-targets --all-features -- -D warnings` → 0 warning.
4. `cargo build --release --locked` untuk kedua varian fitur.
5. `pkill -x topmonitoring || true` lalu `./scripts/test-no-wayland.sh smoke` dan
   manual Phase 3B (`docs/TESTING.md §11`) — termasuk kategori mulai:
   Ctrl+F/Esc, filter catalog, More kosong/`More N`, lazy editor, pane
   responsive, Save/Discard/Reset, buka Settings 10 menit tanpa stale UI/crash.
6. Bump versi → `3.0.0-alpha.6` dan perbarui `SOURCE_CANDIDATE`.

**Gate:** 44/44 test, 0 clippy warning, locked build, smoke + manual log yang
dicatat sebagai bukti, commit tunggal di `major/v3`.

### Step 0.2 · Phase 4a — Infrastruktur visual-effect engine

- File baru `src/effects.rs` (state murni + pengeksekusi GTK).
- Efek: `Off`, `OrbitGlow`, `EmberOrbit`, `AuroraSweep`, `CriticalBeacon`,
  `StaticFallback`.
- Render lewat **frame clock** (`GtkOverlay` + `add_tick_callback`), bukan CSS
  recompile. Budget p95 < 3 ms @ 30 FPS. Tick berhenti saat window
  hidden/unmapped. Layer non-interaktif (`set_can_target(false)`).
- Konfigurasi flat baru (`#[serde(default)]` agar config lama tetap parse):
  `effects_enabled=false`, `effects_kind`, `effects_fps_cap=30`,
  `effects_intensity/trail/thickness/speed` — semua nilai di-clamp saat migrasi;
  `reduce_motion=true` memaksa fallback statis; on-battery policy auto-off.

### Step 0.3 · Phase 4b — Hapus jalur animasi CSS lama

- Buang blok hue-recompile `animated_bg` di `main.rs:673-679`.
- `animated_bg=true` pada config lama → ditampilkan sebagai saran yang
  **dinonaktifkan** (tidak menjalankan efek apa pun).
- Perluas `scripts/check_source.py`: larang `load_from_string` dari dalam
  polling loop utama.

### Step 0.4 · Phase 4c — Theme Studio

- Tombol warna native + fallback CSS value tervalidasi untuk token:
  accent/surface/warning/critical, bar background+opacity, font
  family/size/weight, module padding/gap/radius/border, shadow.
- Live preview pada 1366 px + simulasi sempit; **undo** ke state terakhir dan
  **Restore safe visual defaults**.

### Step 0.5 · Phase 5 — Layout editor, feedback aksi, aksesibilitas

- Layout editor: katalog modul → drop-zone `start/center/end`; kontrol keyboard
  untuk move; preview tier Normal/Compact/Tiny per modul; penjelasan mengapa
  sebuah modul masuk **More**.
- Action toast / feedback inline (V3-009): status pending → sukses/gagal +
  nama backend + detail error di bar.
- A11y: urutan fokus lengkap, accessible names, target klik ≥ 44 px, kontras
  WCAG AA, alur keyboard penuh Settings; samakan desain History/Process/Sensors.

### Step 0.6 · Phase 6 — Schema v3 + hardening + rilis v3.0.0

- `CONFIG_SCHEMA_VERSION = 3` dengan section `[layout]`, `[effects]`,
  `[storage]`, `[desktop_actions]`.
- Migrator v2→v3: backup `config.toml.v2.bak.<timestamp>` sebelum simpan
  pertama; nilai tak dikenal → default aman; `animated_bg` → tidak diam-diam
  mengaktifkan efek.
- Matriks rilis: CI Wayland + X11; upgrade terinstal 2.0.1 → 3.0.0 tanpa
  kehilangan config; rollback terdokumentasi; soak 8 jam; screenshots per
  width/scale; docs sinkron (README/TECHNICAL/TESTING/CHANGELOG/RELEASE_CHECKLIST/
  migration) di commit yang sama; dua varian `.deb`; SHA-256; publikasi GitHub.

**Gate Tahap 0:** `v3.0.0` terpasang dan dikonfirmasi manual di mesin pemilik;
upgrade 2.0.1→3.0.0 lolos; branch `major/v3` di GitHub berisi tag `v3.0.0`.

---
## 4. Tahap 1 — TopMonitoring 4.0

### Fase A — Solid Foundation (refactor modular + lib/bin + schema v3 + typed metrics)

#### A.1 · Pecah `main.rs` (5.818 → < 1.200 baris)

Ekstraksi bertahap, **satu commit per langkah dengan build hijau**:

| Urutan | Modul baru | Isi dari `main.rs` |
|---|---|---|
| 1 | `src/docking.rs` | `X11DockGeometry`, `calculate_x11_dock_geometry`, `apply_x11_dock`, `configure_x11`, `configure_wayland`, `monitor_geometry` |
| 2 | `src/fsio.rs` | Helper murni yang dipakai `backend.rs` (`human_bytes`, `format_disk_*`, `read_cpu_power`, `disk_temps`, `DiskSummary`, dst.) — perbaiki dependensi terbalik backend→main |
| 3 | `src/history.rs` | `HistorySummary`, `summarize_history`, `append_history_values_async`, `history_log_path` |
| 4 | `src/ui/{processes,sensors,hwmon}.rs` | Process manager, sensor viewer, hwmon picker, `ProcessSnapshotRow`, `HwSensor`, `HwmonPick` |
| 5 | `src/ui/settings/{mod,search,catalog}.rs` | `open_settings`, `SettingsContext`, `ModuleCatalog*`, `search_text_matches`, `settings_page_search_target` |
| 6 | `src/ui/bar.rs` | `Slot`, `SlotWidget`, `AdaptiveEntry`, `OverflowUi`, `BarZones`, slot builder, button quick actions |
| Sisa | `src/main.rs` | Decl mod, `main()`, orkestrasi `build_bar`, polling timer GTK, threading glue |

Target struktur akhir:

```text
src/
├── main.rs        (~900 baris glue)
├── lib.rs         (pub mod: backend, runtime, command, config, layout, model,
│                   docking, history, fsio, persistence, control, ui)
├── docking.rs · history.rs · fsio.rs
├── persistence.rs          (Fase B)
├── control/{dbus,cli}.rs   (Fase C)
└── ui/
    ├── mod.rs · bar.rs · effects.rs · theme_studio.rs
    ├── settings/{mod,search,catalog}.rs
    ├── processes.rs · sensors.rs · hwmon.rs
    └── dashboard.rs        (Fase B)
```

Syarat tak boleh dilanggar: **refactor murni, tanpa perubahan perilaku**; semua
44+ test tetap hijau di setiap commit ekstraksi.

#### A.2 · Pisahkan lib vs bin

- `src/lib.rs` = crate `topmonitoring` (lib) berisi semua modul; `src/main.rs` =
  bin tipis.
- Integrasi test di `tests/` (mis. `tests/config_migration.rs`, `tests/layout.rs`)
  menyusul unit test dalam modul.
- Dua varian fitur `wayland`/no-Wayland tetap bekerja persis seperti sekarang.

#### A.3 · Schema v3 lengkap + kerangka Profiles

- `src/config/schema_v3.rs`: `LayoutConf`, `EffectsConf`, `StorageConf`,
  `DesktopActionsConf`, `AlertsConf` (Fase D), `Profile`.
- `src/config/migration.rs`: migrator v2→v3 + backup + unit test round-trip
  (config 2.0.1 asli → v3 → simpan → baca); clamp nilai invalid; profil default
  `default`.
- Settings Overview menampilkan versi schema + lokasi backup terakhir.

#### A.4 · Typed MetricValue

- `enum MetricValue { Percent, Fraction, Bytes, Rate, Temp, Power, Frequency, Count, Text, Toggle }`
  menggantikan `MetricSample.value: String`.
- Diff/threshold/alert menjadi deterministik berbasis angka, bukan string.
- Custom module tetap `Text(String)`; semua test yang menyentuh
  `MetricSample::new(...)` dimigrasikan bersama.

**Gate Fase A:** `main.rs` < 1.200 baris; tanpa `use super::` terbalik dari
`backend.rs` ke crate root; 44+ test lama lulus tanpa perubahan perilaku;
`cargo test` + clippy dua matriks fitur hijau.

### Fase B — Persistence SQLite + dashboard terpadu

- `rusqlite` (fitur `bundled` agar `.deb` tidak bergantung sqlite sistem).
- Tabel `samples(timestamp, cpu, mem, temp, net_down, net_up, disk_used, ...)`
  dan `alerts(id, ts, rule, value, message)`; worker menulis batch WAL; retensi
  hari+MB dengan pruning; migrasi best-effort `history.csv` lama → SQLite.
- Ekspor CSV/JSON dipertahankan.
- Dashboard (second window on-demand): tab Overview, Metrics (1 jam/1 hari/1
  minggu), Storage, Network, Processes (searchable + kill berkonfirmasi), Sensors,
  History/Export. Graf tetap `DrawingArea` native — tidak menambah dependency
  chart baru.
- Proses/sensor memakai list model snapshot-backed, bukan rebuild 200 ms.

**Gate Fase B:** insert/query/prune/rotasi + migrasi CSV diuji; soak 8–24 jam
tanpa pertumbuhan memori/thread/fd; manual matrix dengan bar + Settings +
dashboard terbuka bersamaan tanpa snapshot korup.

---
## 5. Fase lanjutan v4 (setelah iterasi pertama)

### Fase C — Multi-bar, profiles, control plane

- Multi-bar: satu bar per monitor, konfigurasi per-bar, EWMH strut/
  layer-shell per instance dalam satu proses GTK.
- Profiles: `gaming`, `battery-saver`, `minimal`, custom; beralih via UI/CLI/
  D-Bus; swap config domain secara transaksional (pola `migrate_and_validate`
  yang sudah ada) tanpa spawn ulang proses.
- D-Bus `io.github.fakedevbagus.TopMonitoring` via `zbus`: `GetMetric`,
  `GetSnapshotJSON`, `ToggleVisibility`, `SetProfile`, `SetGeometry`.
- CLI: `topmonitoring control status|metric|profile|toggle --json`.

### Fase D — Alerting engine

- Rule: `metric + threshold + condition + cooldown + action
  (notify-send | trusted command | D-Bus signal | history)`.
- Dedupe & cooldown menggantikan `ALERTS` thread-local; hormati
  `reduce_motion`/mode tenang/battery policy.
- Reuse `MetricLevel::Higher/Lower` + warn/crit yang sudah ada.

### Fase E — Backend expansion

- SMART/NVMe health (optional `smartctl`/`nvme`, capability-gated).
- Battery health, charge rate, power-profile read.
- Snap package updates (menambah APT/DNF/Pacman/Flatpak).
- GPU engine detail (utilization per engine / VRAM) untuk NVIDIA + AMD hwmon.
- Semua optional & degrade graceful seperti pola provider saat ini.

### Fase F — Developer ecosystem + rilis v4.0.0

- Endpoint lokal read-only (JSON-RPC atau Prometheus text exposition) di Unix
  socket; dokumentasi + contoh `curl --unix-socket`. Bukan webview/Electron.
- Hardening: matriks CI penuh, install/upgrade/rollback, soak 24 jam, screenshot
  visual per width/scale, docs sinkron, SHA-256, publikasi GitHub.

**Gate Tahap 1 penuh:** seluruh acceptance criteria §7.

## 6. Strategi Git

- Branch kerja utama: `major/v3` (berisi baseline + Tahap 0 sampai rilis v3.0.0).
- Branch kerja v4: `major/v4` (di-fork dari tag `v3.0.0` saat rilis).
- Branch per fix/feat kecil (`fix/...`, `feat/...`), merge ke branch utama
  dengan review; jangan gabung perubahan yang tidak saling terkait dalam satu
  commit.
- `origin` = `https://github.com/fakedevbagus/TopMonitoring-linux.git`.
  Perhatikan: `origin/main` saat ini berisi kode legacy v1.3.0. **Jangan
  menimpa `main` dengan force-push.** Kerja rilis masuk lewat tag + merge biasa.
- Dokumen rencana mengikuti pola yang sudah ada: `AUDIT_MAJOR_UPGRADE_PLAN.md`
  (v2) dan `MAJOR_UPGRADE_V3_PLAN.md` (v3); dokumen ini adalah
  `MAJOR_UPGRADE_V4_PLAN.md` (v4).

## 7. Acceptance criteria v4

- [ ] Tidak ada provider I/O di GTK thread (dijaga gate statis).
- [ ] `main.rs` tanpa pecahan: glue saja, semua fungsionalitas di modul.
- [ ] Schema v3 dimigrasi dengan backup dan rollback terdokumentasi.
- [ ] Efek visual default Off, hormati reduce-motion, berhenti saat hidden.
- [ ] D-Bus query ≤ 10 ms; switch profil ≤ 300 ms tanpa restart proses.
- [ ] SQLite write < 1% CPU idle; dashboard terbuka berjam-jam tanpa stale UI.
- [ ] Alert tidak spam (cooldown), tidak menjalankan command untrusted.
- [ ] Provider baru semua capability-gated dan diagnosanya redacted.
- [ ] Soak 24 jam tanpa pertumbuhan memori/thread/process/fd.
- [ ] Versi `v4.0.0` terpasang dari `.deb` dan dikonfirmasi manual di mesin
      pemilik (X11 no-Wayland + Wayland jika tersedia).
- [ ] README/TECHNICAL/TESTING/CHANGELOG/RELEASE_CHECKLIST/migration sinkron.

## 8. Non-goals dan guardrails

- ✗ Tidak mengganti GTK4 dengan web/Electron/webview.
- ✗ Tidak drop dukungan X11/no-Wayland.
- ✗ Tidak membangun fitur baru di atas alpha yang belum rilis (Tahap 0 dulu).
- ✗ Tidak ada plugin/shader/command eksekusi arbitrer yang bisa diunduh.
- ✗ Tidak menulis ke endpoint lokal tanpa pembatasan (read-only, bounded).
- ✓ Satu proses GTK, satu set worker; profil = swap config domain.
- ✓ Setiap perubahan perilaku/config menyertakan perubahan docs di commit yang
      sama.

## 9. Estimasi iterasi pertama

| Step | Lingkup | Estimasi (paruh waktu) |
|---|---:|---:|
| 0.1 | Gate alpha.5 → alpha.6 | 0,5–1 hari |
| 0.2–0.3 | Effect engine + hapus animated_bg | 1–1,5 hari |
| 0.4 | Theme Studio | 0,5–1 hari |
| 0.5 | Layout editor + a11y + toast | 1–2 hari |
| 0.6 | Schema v3 + hardening + rilis 3.0.0 | 1–2 hari |
| A.1–A.3 | Pecah main.rs + lib/bin + schema | 2–3 hari |
| A.4 | Typed MetricValue | 1–1,5 hari |
| B.1–B.3 | SQLite + dashboard | 2–3,5 hari |
| **Total** | | **± 3–4 minggu** |

Fase C–F menambah ± 4–6 minggu setelah iterasi pertama selesai.
---
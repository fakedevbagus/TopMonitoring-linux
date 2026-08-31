# Changelog

All notable changes to TopMonitoring are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Theme Studio controls on the Settings Appearance page: label font weight
  (100–900), module border strength (0.0–0.6), and native GTK color pickers
  next to every CSS color entry (Background, Accent, Module surface, Warning,
  Critical). Entries remain the advanced fallback for rgba() strings with
  alpha; picker and entry stay in sync.
- Live bar preview: style changes re-render the real bar CSS immediately
  before saving, plus a compact-density simulation toggle and an explicit
  Reset (discard) / Save pair.
- New config tokens `font_weight` and `border_strength` (serde defaults, both
  clamped at validation) included in theme presets.

### Fixed
- A `RefCell already borrowed` panic when opening Settings: the five color
  values were read as temporaries inside one array-literal statement, so the
  shared borrow guards stayed alive while `picker.set_rgba()` synchronously
  fired the notify handler's `borrow_mut()`. Values are now read into locals
  first; the same hardening was applied to the weight/border controls.

### Fixed
- Settings no longer emits recurring `Gtk-CRITICAL` "minimum height ... for
  width of 1048576 ... Expect overlapping widgets" warnings every refresh
  tick. Both Settings stacks (page stack and module catalog stack) are now
  non-homogeneous; the old behavior measured every page at two widths each
  frame and mixed incompatible minimums. Verified 0 warnings across all five
  pages (previously 26–69 per 14s window).

### Added
- Diagnostic environment hooks for headless verification:
  `TOPMONITORING_AUTO_SETTINGS=1` auto-opens the Settings window after
  startup and `TOPMONITORING_SETTINGS_PAGE=<name>` forces the initial page
  (see `docs/TESTING.md` §12). Both are scheduled for removal before 3.0.0 GA.
- Frame-clock effect engine (`src/effects.rs`, Step 0.2): five effect kinds
  (`orbit-glow`, `ember-orbit`, `aurora-sweep`, `critical-beacon`,
  `static-fallback`) drawn on a non-interactive GTK overlay layer above the
  bar. All parameters are clamped, the phase advance respects the fps cap
  (default 30), `reduce_motion` forces the static fallback, and a discharging
  battery auto-disables animation. Disabled by default; new flat config keys
  `effects_enabled/effects_kind/effects_fps_cap/effects_intensity/
  effects_trail/effects_thickness/effects_speed` with serde defaults keep v2
  configs parsing. Eight new unit tests (52 total).

### Changed
- The legacy per-second `animated_bg` hue rotation was removed
  (`load_from_string` inside the polling loop is now rejected by
  `scripts/check_source.py`). Effect visuals render exclusively through the
  new engine; configs with `animated_bg = true` simply keep the static theme.

## [3.0.0-alpha.5] - 2026-08-16

### Added
- Persistent global Settings search across pages, controls, all 38 built-ins,
  aliases, custom module names, labels, and commands, with Ctrl+F/Esc flow.
- A compact module catalog with Enabled/Disabled, Built-in/Custom, and category
  filters, stable selection, explicit empty states, and one lazy detail editor.
- Responsive Settings navigation and catalog panes that switch layout at narrow
  window widths, plus five pure catalog/search regression tests.

### Changed
- Built-in and custom module editors share one catalog instead of eagerly
  constructing every full card when Settings opens.
- The permanent More control is clickable even when nothing overflows and shows
  an explanatory empty state instead of appearing broken.
- Alpha.4 r14 terminal, smoke, adaptive-width, overflow, and prior regression
  gates are recorded as passed; alpha.5 is the complete Phase 3B candidate.

## [3.0.0-alpha.4] - 2026-08-16

### Added
- A pure deterministic pixel-budget allocator with normal, compact, tiny, and
  overflow tiers plus canonical width profiles for all built-in modules.
- A permanent **More** popover that represents every overflowed module and
  continues showing its live value while the main widget is hidden.
- Width-tiered disk text: root plus fullest volume at normal width, the fullest
  selected volume at compact width, and a bounded percentage badge at tiny width.
- Nine Phase 3A tests covering allocation order, tier transitions, deterministic
  overflow, fixed-width behavior, registry profile bounds, disk tiers, reserved
  control budget, and owned snapshot tier values.

### Changed
- Module priority now controls survival under width pressure instead of only
  sorting. Settings and overflow controls always keep a reserved width budget.
- GTK reacts to actual allocation width changes without provider I/O, rebuilds,
  or duplicate graph samples; a tier change explicitly invalidates only the
  affected renderer representation.
- Existing **Adaptive compact** controls now describe real adaptive tiers and
  each module can opt out of compact/tiny rendering while retaining overflow.
- Alpha.3 r13 formatting, 30 tests, strict Clippy, release build, smoke, and
  manual Phase 2B responsiveness checks are recorded as passed.

## [3.0.0-alpha.3] - 2026-08-15

### Added
- An owned, immutable `CoreSnapshot` with metric samples, graph values,
  generation, collection duration, freshness deadline, and a redacted
  diagnostic summary.
- A dedicated `topmonitoring-core-collector` worker for sysinfo, `/sys`,
  `/proc`, NVML, GPU, disk, network, battery, and sensor-backed bar metrics.
- Independent workers for process rows, the hardware-sensor window, and the
  hwmon custom-module picker, with GTK receiving only owned results.
- Unit coverage for snapshot/graph diffs, diagnostics redaction, collector
  configuration, external-field diffs, and owned process-row sorting.

### Changed
- GTK reconciliation now compares snapshot generations and per-module values;
  unchanged labels, ranges, tooltips, and graph histories are left untouched.
- Runtime diagnostics combine core-collector and external-provider health, and
  network-interface choices come from the latest core snapshot instead of a
  synchronous Settings scan.
- Source hygiene now rejects native provider I/O in the GTK polling callback or
  metric renderer. Process and sensor handoff queues are bounded.
- The alpha.2 terminal, smoke, and manual provider-isolation gate is recorded as
  passed; alpha.3 is the Phase 2B source-test candidate.

### Fixed
- Fresh source trees can now run `smoke` or `manual` directly: both modes build
  the no-Wayland release binary that their launcher path actually executes.
- Rust 1.97 strict Clippy findings are resolved: redundant string dereferences
  were removed, the sensor producer uses its suggested `while let` form, and a
  typed `SettingsContext` replaces the eight-argument Settings entry point.
- Revision-specific archive directories and `SOURCE_CANDIDATE` output prevent
  an older alpha.3 extraction from being mistaken for the current candidate.

## [3.0.0-alpha.2] - 2026-08-15

### Added
- Provider health records with available, unavailable, in-flight, stale, and
  error states plus attempt/success time, duration, failure count, lane, and
  generation metadata.
- A privacy-safe Runtime diagnostics view that omits SSIDs, media metadata,
  usernames, paths, and command output.
- A typed `ModuleDescriptor` registry for all 38 built-in modules, including
  canonical names, categories, search aliases, and capability identities.
- Unit coverage for provider freshness/error transitions, lane classification,
  redacted diagnostics, descriptor uniqueness, registry coverage, and aliases.

### Changed
- Fast audio/brightness/media/connectivity polling and slow package/release
  jobs now run on separate workers; desktop actions retain their independent
  priority worker.
- Built-in Settings cards and search are generated from the shared descriptor
  registry instead of duplicated alias metadata.
- Phase 1 terminal, smoke, and manual X11 regressions are recorded as passed;
  the remaining Phase 2 work is core snapshot offload and render reconciliation.

## [3.0.0-alpha.1] - 2026-08-14

### Added
- Dedicated high-priority desktop-action worker and structured action feedback.
- Desktop-aware lock capability chain for Cinnamon, GNOME, Xfce, MATE, XDG, and logind.
- Search fields for built-in and custom modules, including HDD/storage aliases.
- Pure regression tests for negative X11 geometry, compact disk output, search, and lock ordering.

### Changed
- Disk text now keeps root plus the fullest secondary volume; full details remain in the tooltip.
- X11 dock geometry validates and clamps signed monitor coordinates before EWMH CARDINAL conversion.
- Canonical Rust formatting is synchronized with the source that passed the no-Wayland terminal gate: 17 tests, Clippy with warnings denied, release build, and source/documentation checks.

## [2.0.1] - 2026-08-14

### Fixed
- Debian upgrades now declare `Conflicts` and `Replaces` for the legacy
  `topmonitoring-no-wayland` package, preventing shared-file installation
  failures while preserving the user configuration directory.
- Packaging and release-document gates now ignore unrelated parent Git
  repositories, validate no-Wayland dependencies and migration relationships,
  and remove stale package artifacts before building.

## [2.0.0] - 2026-08-14

### Added
- Three-zone bar layout with priority ordering, compact labels, per-module
  colors, and display templates.
- Calm Telemetry design tokens, appearance presets, density controls,
  reduced-motion mode, and separate user CSS provider.
- Navigation-based Settings with live preview and transactional
  Save/Discard/Reset behavior.
- Dedicated runtime worker and thread-safe snapshots for external providers.
- Per-command interval, timeout, output limit, trust state, click action,
  compact mode, on-demand Run now refresh, duplication helpers, and a global
  custom-command concurrency cap.
- Safe config import, schema versioning/migration, validation, atomic writes,
  and restrictive config permissions.
- Functional lock, screenshot, and double-confirm poweroff actions.
- Configurable history-file rotation plus a non-blocking chart/statistics
  dashboard with CSV export and confirmed clearing; AppStream metadata, two CI
  feature matrices, release packaging workflow, source-hygiene checks, and tests
  for config, commands, version parsing, package counts, and X11 geometry.
- Repeatable no-Wayland terminal/smoke/manual test workflow, isolated test state,
  documentation/version gate, and package creation blocked behind explicit manual
  confirmation.

### Changed
- Application ID is now `io.github.fakedevbagus.TopMonitoring`.
- Network and disk rates are normalized by real elapsed time.
- CPU frequency reports the average across logical CPUs.
- Network graph and lifetime totals honor the selected interface.
- Package counts aggregate supported, available backends (APT, DNF, Pacman
  `checkupdates`, Flatpak) rather than assuming one package manager.
- `.deb` metadata is shared correctly; the no-Wayland variant overrides only
  dependencies.
- Installer and uninstaller now handle binary, desktop entry, AppStream data,
  both icon sizes, and desktop/icon caches symmetrically.

### Fixed
- Rustfmt drift and strict Clippy gate failures discovered by the first real
  Ubuntu no-Wayland terminal validation.
- GTK main-loop stalls caused by synchronous external commands.
- Pipe deadlock and unbounded output risks in custom commands.
- Races between delayed auto-dim callbacks.
- Slider feedback while dragging and unchecked setter failures.
- X11 CARDINAL underflow with signed monitor coordinates.
- Config import closing/revert behavior and stale update repository URL.
- Bluetooth private-use glyph rendering and incomplete quick-action rendering.

### Security
- Imported commands and click actions are disabled until explicitly trusted.
- Custom commands run in process groups with TERM/KILL timeout handling,
  bounded output, and a four-command global concurrency limit.
- Dynamic CSS colors and generated class identifiers are sanitized.

## [1.3.0]

### Added
- Audio/brightness controls, Bluetooth/VPN, process manager, CSV history,
  update checker, and `SIGUSR1` visibility toggle.

## [1.2.0]

### Added
- Quick-action configuration, Wi-Fi, package updates, media, and removable
  disk reporting.

## [1.1.1]

### Fixed
- Generated CSS, critical-state animation, and icon safe-zone issues.

## [1.1.0]

### Added
- Threshold editor, details, presets, graphs, auto-dim, and async custom
  command execution.

## [1.0.0]

### Added
- Initial X11/Wayland dock, metrics, sensors, customization, and packaging.

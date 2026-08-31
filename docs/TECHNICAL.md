# TopMonitoring — Technical Reference

## 1. Architecture

TopMonitoring is a Rust 2021 GTK4 application with seven source modules:

| Module | Responsibility |
|---|---|
| `main.rs` | GTK lifecycle, snapshot reconciliation, Settings, and X11/Wayland docking |
| `backend.rs` | Native core collector, immutable metric/graph snapshots, freshness, and diff helpers |
| `layout.rs` | Pure width profiles, tier decisions, pixel budgeting, and deterministic overflow allocation |
| `config.rs` | v2 schema, migration, validation, atomic persistence, CSS generation |
| `model.rs` | Canonical descriptors, categories, aliases, and capabilities for built-in modules |
| `runtime.rs` | Fast/slow provider schedulers, priority action worker, health metadata, and cached snapshots |
| `command.rs` | Bounded shell/program execution with timeout and process-group cleanup |

The GTK cadence clones `CoreSnapshot` and `ExternalSnapshot`; it never refreshes
sysinfo, sysfs, `/proc`, network/storage, or NVML providers. `CoreCollector`
owns that native state on `topmonitoring-core-collector`, while
`RuntimeServices` owns fast and slow external-provider workers plus the
high-priority action worker. GTK compares generations and individual samples,
mutating only affected widgets. Custom modules use capped background executions
and mpsc results.

## 2. Runtime model

Default provider cadences are deliberately different:

- audio and brightness: 2 seconds
- Wi-Fi, Bluetooth, and media: 5 seconds
- package updates: 20 minutes
- release check: once at startup when enabled, or on request

A provider update records its lane, available/unavailable/in-flight/error
state, last attempt and success, duration, failure count, stale deadline, and
generation. The effective state becomes stale when a formerly successful
sample exceeds that deadline. Settings renders only a redacted diagnostic
report; it never includes SSIDs, media titles/artists, usernames, paths, or
command output.

A failed optional backend produces `None`/`n/a` without stopping the bar.
Setters verify process exit status and use a fallback where safe. Package
updates are aggregated across APT, DNF, Pacman `checkupdates`, and Flatpak when
present. Runtime shutdown is requested when its last service owner is dropped.

Custom-command policy clamps timeout to 100–60,000 ms and captured output to
256–65,536 bytes. Reader threads drain both pipes even after reaching the cap,
preventing child-process deadlock. Four custom commands may execute globally;
timed-out Unix commands receive TERM then KILL as a process group. Settings can
queue an immediate rerun of any active custom module without waiting for its
regular cadence.

## 3. Configuration

Path: `~/.config/topmonitoring/config.toml`.

The schema uses `schema_version = 2` and `#[serde(default)]`, so old files can
be parsed without a separate brittle conversion format. `migrate_and_validate`
clamps geometry, timing, colors, command limits, zones, and priorities, de-
duplicates metric IDs, and appends newly introduced built-ins.

Saving writes `config.toml.tmp`, syncs it, applies mode `0600`, atomically
renames it, then best-effort syncs the containing directory.

Important global fields:

| Field | Meaning |
|---|---|
| `height`, `margin_top`, `position`, `monitor` | Native dock geometry |
| `theme`, `custom_bg`, `font_*` | Base appearance |
| `accent_color`, `surface_color`, `warning_color`, `critical_color` | Semantic tokens |
| `module_gap`, `module_padding`, `module_radius`, `bar_opacity` | Density/surface tokens |
| `interval_ms` | In-process refresh cadence (250–10,000 ms) |
| `reduce_motion`, `blink_critical`, `adaptive_compact` | Accessibility/density behavior |
| `history_*` | CSV enablement, cadence, and rotation size |
| `qa_*` | Optional quick actions |

Each built-in metric supports `enabled`, `label`, `zone`, `priority`,
`compact`, `format`, `warn`, `crit`, per-module colors, and an explicitly
trusted click command. Display templates replace `{label}` and `{value}`.

Each custom module additionally supports `command`, `trusted`, `interval_ms`,
`timeout_ms`, `max_output_bytes`, `tooltip`, and `click_command`.

## 4. Import trust boundary

`Config::from_import` always runs validation followed by command sanitization.
Built-in click commands become untrusted. A custom module containing a polling
or click command becomes both disabled and untrusted. The Settings import flow
saves this sanitized state as the new transaction baseline before closing.

Treat configuration files as executable-capable data even though they use
TOML. Do not bypass this boundary in future import paths.

## 5. Rendering and CSS

`CenterBox` contains explicit start, center, and end GTK boxes. Enabled entries
are sorted by descending priority per type and appended to their zone. Every
widget receives `.module` and a sanitized `.module-<id>` class.

Safe generated CSS and advanced user CSS use separate `CssProvider` objects.
Color fields are sanitized both during migration and when building live-preview
CSS. User CSS strips imports and remote HTTP(S) URLs before loading. Invalid
user CSS therefore cannot remove the generated base theme.

Critical blink is a state overlay and is disabled by reduced-motion mode.
Color still communicates warn/critical state when animation is off.

## 6. Metrics correctness

- CPU frequency is averaged across all logical CPUs.
- Network and disk deltas are divided by measured elapsed seconds.
- Interface filtering is shared by network text, graph, and lifetime totals.
- Disk usage excludes known virtual filesystems and reports mounted storage.
- GPU selection is explicit; NVIDIA uses NVML and AMD/Intel use best-effort
  sysfs paths.
- Audio/brightness periodic sync is suppressed during direct slider dragging.

## 7. Docking

Wayland builds use `gtk4-layer-shell`, anchors, monitor selection, and an
exclusive zone. The optional feature can be removed with
`--no-default-features`.

X11 builds set dock type, struts, sticky/above/taskbar/pager state, and exact
geometry using `x11rb`. Strut calculation keeps signed intermediate monitor
coordinates and clamps only before writing unsigned CARDINAL properties.

## 8. Settings transaction

Only one Settings window can exist. Controls mutate an in-memory config and
apply the relevant style/layout/geometry/interval domain immediately. Save
validates, atomically persists, and replaces the rollback snapshot. Closing or
Discard restores that snapshot. Reset saves defaults before closing.

Navigation pages are Overview, Bars & layout, Appearance, Built-in modules,
Custom modules, and Data & tools. A persistent footer exposes status and
transaction actions.

## 9. History

When enabled, the main refresh takes CPU, memory, and temperature values at the
configured interval, then queues file work on a background thread. The CSV
header and rows remain compatible with 1.x. At `history_max_mb`, the active
file rotates to `history.csv.1` before the next row. The history dashboard
loads and parses the file on a worker thread, renders the latest 180 samples,
shows whole-file min/average/max statistics, exports CSV, and clears current plus
rotated history only after confirmation.

## 10. Packaging

The canonical application ID is `io.github.fakedevbagus.TopMonitoring` across
GTK, desktop entry, icon names, AppStream metadata, `.deb` assets, and
autostart.

```bash
cargo build --release --locked --all-features
cargo build --release --locked --no-default-features
cargo deb --locked
cargo deb --locked --no-default-features --variant no-wayland
```

The base Debian metadata describes the normal Wayland-capable package; the
variant overrides only dependencies. The unified `topmonitoring` package
conflicts with and replaces the legacy `topmonitoring-no-wayland` package. The
no-Wayland packaging gate verifies those control fields and rejects a generated
package that depends on `libgtk4-layer-shell0`. Dirty-tree enforcement is active
only when `.git` belongs to the project root, so an extracted source directory
inside an unrelated parent repository remains valid. `install.sh` and
`uninstall.sh` mirror binary, desktop entry, metadata, and icon operations under
`PREFIX`.

## 11. Validation

The required authoritative checks are:

```bash
cargo fmt --check
cargo test --locked --all-features
cargo test --locked --no-default-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo clippy --locked --all-targets --no-default-features -- -D warnings
bash -n install.sh uninstall.sh
python3 scripts/check_source.py
```

CI runs both feature modes and release builds. Unit coverage includes config
migration/import sanitization, CSS identifiers, output truncation/timeouts,
version comparison, package counting, signed X11 geometry, provider health
transitions, scheduler-lane classification, diagnostic redaction, and module
registry completeness.

## 12. Release checklist

1. Update version in `Cargo.toml`, lockfile, changelog, and AppStream metadata.
2. Run the full validation matrix on Ubuntu 24.04 or CI.
3. Exercise Settings save/discard/import and custom-command trust manually.
4. Test native docking on one Wayland compositor and one EWMH X11 WM.
5. Inspect `.deb` metadata and contents with `dpkg-deb`.
6. Tag the validated release; the release workflow uploads the package.

See the audit plan for prioritized future work beyond 2.0.

## 13. TopMonitoring 3 alpha.1 reliability layer

Version `3.0.0-alpha.1` separates direct desktop actions from the slower
provider scheduler. The `topmonitoring-actions` worker handles Lock,
screenshot, poweroff, volume, brightness, and `xdg-open`; package and release
checks remain on `topmonitoring-providers`. A quick action publishes a request
ID, pending state, completion state, selected provider, message, and duration
to the shared snapshot so its bar button can provide immediate feedback.

Lock backend ordering is selected from `XDG_CURRENT_DESKTOP`, with native
Cinnamon, GNOME, KDE, Xfce, and MATE candidates before freedesktop D-Bus,
display-manager, XDG, and systemd-logind fallbacks. Logind success is explicitly
reported as unverified because exit status alone does not prove the desktop
displayed a lock screen.

X11 struts now come from a pure signed-geometry function. Monitor dimensions,
coordinates, offset, and bar height are validated and clamped before converting
EWMH properties to unsigned values. The function is unit-tested with a monitor
left of and above the primary origin.

Disk bar text no longer concatenates every mount. It selects root and the
fullest non-root volume, bounds each display name, and leaves the complete mount,
filesystem, free-space, usage, and temperature breakdown in the tooltip.
Settings performs view-only search across built-in IDs, labels, aliases, and
custom-module names/commands; search never mutates configuration.

## 14. TopMonitoring 3 alpha.2 provider lanes and registry

Version `3.0.0-alpha.2` replaces the single external-provider scheduler with
`topmonitoring-fast-providers` and `topmonitoring-slow-providers`. Audio,
brightness, media, Wi-Fi, and Bluetooth use the fast lane. Package-manager and
release checks use the slow lane. `topmonitoring-actions` remains independent,
so a package timeout cannot occupy the same queue as Lock or screenshot.

`ProviderHealth` is stored alongside the external values but contains only
operational metadata. Each collection marks itself in-flight before work and
publishes the final status afterward. Last-success time survives a failed
attempt, enabling deterministic stale/error transitions and useful diagnostics
without retaining sensitive provider values.

`model.rs` defines one `ModuleDescriptor` for every built-in metric. Settings
uses the descriptor name, category, capability, and aliases for card metadata
and search. Tests reject duplicate IDs and missing descriptors, preventing the
search vocabulary and module catalog from drifting apart.

Alpha.2 is Phase 2A, not the completed backend gate. Phase 2B must move native
hardware/filesystem collection into an immutable core snapshot and make GTK
reconcile only changed generations before adaptive layout or visual effects
begin.

## 15. TopMonitoring 3 alpha.3 immutable core snapshots

Version `3.0.0-alpha.3` implements Phase 2B. `NativeCollector` exclusively owns
`System`, `Components`, `Networks`, `Disks`, and optional NVML handles on the
`topmonitoring-core-collector` thread. Each collection creates a new owned
`CoreSnapshot`; GTK never receives provider objects, borrowed sysinfo data, or
filesystem handles.

A core snapshot contains `MetricSample` values and tooltips, threshold metadata,
graph scalars, selectable interface names, history scalars, generation, wall
clock, duration, and a cadence-derived stale deadline. Its diagnostics expose
only health metadata and counts—never hostname, interface names, mount paths, or
metric values. Collector reconfiguration is typed and wakes collection after an
interval, interface, or GPU selection change.

The GTK timer independently compares core and external generations. A text or
range widget is updated only when its backing field changed. Graph histories
append and redraw only when the corresponding graph value changed, preventing
multiple identical samples when GTK wakes faster than collection. Clock/date
and bounded custom-command channel handling retain their own cadence.

The process manager (`topmonitoring-processes`), hardware-sensor viewer
(`topmonitoring-sensors`), and hwmon picker (`topmonitoring-hwmon-picker`) also
collect away from GTK. Process/sensor result queues hold at most one pending
snapshot; closing a window drops its receiver and causes the worker to exit.
`SettingsContext` groups the GTK window, active slots, collectors, apply
actions, and singleton Settings slot instead of widening the Settings function
signature each time another service is introduced.
The source-hygiene gate additionally scans the main GTK reconciliation and
renderer regions for native refresh or filesystem-I/O regressions.
Smoke and manual modes explicitly compile the release no-Wayland target before
launching `target/release/topmonitoring`, so either mode also works in a fresh
extracted source tree. Each candidate also carries and prints a
`SOURCE_CANDIDATE` revision marker.

Alpha.3 passed the Ubuntu terminal, smoke, and manual Phase 2B gate. That
evidence authorizes the adaptive-layout work in alpha.4; Ember Orbit remains a
later Phase 4 feature.

## 16. TopMonitoring 3 alpha.4 adaptive allocation

Version `3.0.0-alpha.4` implements Phase 3A in `layout.rs`. Each enabled module
contributes a stable key, priority, original order, and `WidthProfile`. The pure
allocator starts at normal width, downgrades lower-priority items through
compact and tiny tiers, then overflows the lowest-priority eligible items until
the modeled width plus gaps fits the available pixel budget. Equal-priority
choices are deterministic.

The GTK layer reserves space for More, Settings, and configured quick actions.
A 250 ms allocation observer performs no provider I/O and runs the allocator
only when the actual window width changes. It toggles existing widget visibility,
CSS tier classes, label limits, graph widths, and slider widths; it does not
rebuild providers or append history. Tier dirtiness is tracked separately from
snapshot changes so text is reformatted without injecting duplicate graph
samples.

Overflow rows reuse an owned value label shared with each active slot. Hidden
modules therefore keep receiving snapshot or bounded command updates and remain
discoverable in the More popover. Expanding the available width restores their
original widgets. The More control remains allocated even with zero hidden
items, which prevents the Settings control budget from changing recursively.

`MetricSample` can carry optional compact/tiny values. Storage currently uses
this path for explicit `root + fullest`, `fullest`, and percentage-only tiers;
other metrics retain their normal value and rely on bounded label width. The
module registry exposes each descriptor's canonical width profile, and Settings
uses existing priority/compact controls with updated survival semantics.

Alpha.4 passed its complete Phase 3A gate. Alpha.5 implements the remaining
searchable catalog, lazy editor, keyboard activation, responsive navigation,
and empty states; visual effects remain blocked pending its manual gate.

## 17. TopMonitoring 3 alpha.5 responsive Settings catalog

Settings now keeps one global `SearchEntry` above the page stack. Search routes
known control vocabulary to Overview, Bars & layout, Appearance, or Data &
tools; module aliases and current custom-command metadata route to Modules.
Ctrl+F focuses search and Esc clears it.

The Modules page creates compact `ListBoxRow` summaries for every built-in and
custom entry but constructs only the selected editor. State, kind, and category
filters compose with multi-term search. Selection keys survive temporary filter
exclusion, custom mutations rebuild deterministically, and empty catalog/detail
states explain recovery. Refresh closures are cleared on window close to break
widget/controller ownership cycles.

Below 760 px the page sidebar becomes a compact dropdown. Below 900 px the
catalog `Paned` changes from horizontal to vertical. These observers only react
to threshold changes and perform no provider work. The More popover is now
permanently actionable; with zero hidden modules it displays an informational
empty state.

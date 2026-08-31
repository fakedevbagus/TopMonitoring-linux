# TopMonitoring 3 — Adaptive Visual Engine Major Upgrade Plan

**Status:** approved by the project owner on 2026-08-14; Phase 0/1, Phase 2, and Phase 3A passed; Phase 3B is implemented on `major/v3`
**Latest gate:** `3.0.0-alpha.4` r14 adaptive-layout matrix passed; `3.0.0-alpha.5` r15 searchable-Settings gate is pending
**Audit baseline:** TopMonitoring 2.0.1, commit `9c8be47`  
**Audit date:** 2026-08-14  
**Primary target:** Ubuntu/Linux desktop, with the no-Wayland X11 package remaining a first-class release

---

## 1. Executive decision

The backend is **not yet technically exhausted**. TopMonitoring already has broad
metric coverage, but the latest real-device test exposed reliability and layout
problems that must be fixed before spending most development effort on visual
effects.

The recommended TopMonitoring 3 allocation is:

- **30% backend stabilization and architecture**: action reliability, collector
  scheduling, typed snapshots, capability diagnostics, and regression tests;
- **70% frontend and visual customization**: genuinely adaptive layout,
  searchable Settings, a visual-effect engine, richer presets, and an easier
  module editor.

This is not a rewrite and does not replace GTK4 with a web frontend. The plan
keeps Rust + GTK4, preserves X11/no-Wayland support, migrates the existing v2
configuration, and leaves effects disabled by default until the user enables a
preset.

### Release recommendation

Use **TopMonitoring 3.0.0** because this plan changes the configuration schema,
module/render model, runtime scheduling, Settings information architecture, and
bar composition. A minor 2.x release would understate the migration scope.

---

## 2. Scope and audit method

The audit covered all current application code and release gates:

- `src/main.rs` — 4,255 lines;
- `src/config.rs` — 793 lines;
- `src/runtime.rs` — 720 lines;
- `src/command.rs` — 253 lines;
- 38 built-in metric definitions and their render paths;
- 12 current unit tests;
- X11 and Wayland positioning paths;
- Settings, history, process, sensor, package, and custom-command surfaces;
- packaging, CI, testing, migration, and release documentation;
- the repository screenshot and the user's installed 2.0.1 observations.

Static source/document gates pass on the audited baseline. Cargo validation for
2.0.1 already passed on the user's Ubuntu machine. The issues below are runtime,
architecture, responsiveness, and UX findings rather than a claim that the
current source fails to compile.

---

## 3. Findings that block the visual phase

### Priority legend

- **P0:** fix before any visual-effect implementation;
- **P1:** complete in the same major-upgrade cycle;
- **P2:** important hardening or polish after the core v3 experience works.

| ID | Priority | Finding | Evidence / root cause | Required outcome |
|---|---:|---|---|---|
| V3-001 | P0 | Lock button can appear to do nothing | `src/main.rs:982-988` only queues an action. `src/runtime.rs:131-186` has one serial worker, while startup package/update jobs can occupy it for roughly 34 seconds at configured timeouts. `src/runtime.rs:624-632` treats a successful `loginctl` exit as final even though logind only locks sessions that support its Lock signal. | Dedicated high-priority action lane, capability detection, visible pending/result state, verified/fallback lock strategy, and desktop matrix tests. |
| V3-002 | P0 | HDD text is clipped and not truly adaptive | `src/main.rs:1453-1512` concatenates every mount plus temperature into one label. `src/main.rs:902-903` caps all compact labels at 30 characters. Priority is currently used only for sorting, not for hiding or overflow. | Width-tiered disk formatter, selectable disk policy, measured layout budget, compact fallback, and full detail in tooltip/popover. |
| V3-003 | P0 | X11 negative-coordinate regression risk | `src/main.rs:592-593` casts signed monitor X coordinates directly to `u32`. A monitor left of the primary can underflow. The current test module does not cover X11 geometry although README claims signed underflow is avoided. | Pure signed geometry function, validated/clamped conversion, multi-monitor fixtures, and README claim backed by tests. |
| V3-004 | P0 | “Adaptive compact” is a static character cap | `adaptive_compact` changes `max_width_chars`, but no allocated-pixel measurement, compact renderer switch, priority eviction, or overflow popover exists. | Deterministic width allocator with normal/compact/tiny/overflow states. |
| V3-005 | P1 | Slow providers can starve quick actions | APT, DNF, Pacman, Flatpak, release checks, media, connectivity, and user actions share one worker. A slow package manager delays unrelated controls. | Separate action executor and provider scheduler; bounded parallelism and per-provider cancellation/deadlines. |
| V3-006 | P1 | Core collection still runs on the GTK thread | `src/main.rs:320-355` refreshes sysinfo, components, networks, disks, NVML paths, `/sys`, and `/proc` from a GTK timer. The UI comment overstates the non-blocking boundary. | Collector thread owns system state; GTK only reads immutable snapshots and updates widgets. |
| V3-007 | P1 | Settings has no module search | No `SearchEntry` or filter model exists. All 38 built-in cards are constructed fully expanded in `src/main.rs:3273-3472`. | Global Settings search plus module-specific filters, aliases, categories, result count, and keyboard navigation. |
| V3-008 | P1 | Existing animation path is unsuitable for rich effects | `animated_bg` exists in config and updates hue by recompiling the complete CSS provider from the core polling loop (`src/main.rs:379-385`), but the current Settings UI exposes no control for it. | Replace with a frame-clock-driven drawing layer. Never rebuild CSS every animation frame. |
| V3-009 | P1 | Quick-action result is practically invisible | Action status is only visible in the Settings runtime paragraph; the bar button has no busy, success, or failure feedback. | Inline state/toast, precise backend name, error detail, and a Settings “Test action” control. |
| V3-010 | P1 | Settings module pages do not scale | Built-in and custom cards expose every advanced field at once; custom cards are rebuilt eagerly. This becomes harder as visual controls grow. | Compact catalog rows, expandable editor, lazy detail panel, search, filters, and stable selection. |
| V3-011 | P1 | Documentation and screenshot are stale in visible areas | The checked-in screenshot shows the older Settings surface; `animated_bg` and current quick-action behavior are not documented accurately. | Replace screenshots after UI stabilization and keep feature claims tied to tested behavior. |
| V3-012 | P2 | Process and sensor tools do synchronous refresh/rebuild | Process and sensor windows rebuild widget lists every second on the UI thread. | Snapshot-backed list models, row reuse, search, empty/error states, and refresh diagnostics. |
| V3-013 | P2 | Test coverage is concentrated below the UI boundary | Current tests cover config, command limits, package/version parsing, and history parsing, but not action selection, overflow, disk formatting, Settings search, effects, or X11 geometry. | Extract pure algorithms and add unit/integration/visual regression coverage before release. |

---

## 4. Why the lock action fails

The user report is consistent with three independent weaknesses:

1. **Queue starvation.** At startup the runtime immediately scans package
   backends and may also check releases. Those serial jobs can hold the only
   worker before it receives the lock request.
2. **A zero exit status is not proof of a locked screen.** `loginctl
   lock-session` asks the desktop session to lock; the desktop session must
   support and react to logind's Lock signal. The systemd guidance explicitly
   requires the desktop session manager to implement that listener.
3. **Fallback selection stops too early.** If `loginctl` exits successfully but
   the desktop does not actually lock, `xdg-screensaver` is never attempted.
   The UI also gives no immediate indication of which backend ran.

References:

- [loginctl manual — lock-session works when the session supports it](https://www.man7.org/linux/man-pages/man1/loginctl.1.html)
- [systemd desktop-environment integration — session manager must handle Lock/Unlock](https://www.freedesktop.org/wiki/Software/systemd/writing-desktop-environments/)

### Planned lock pipeline

1. The click immediately changes the button to **Locking…** and submits a
   high-priority `DesktopAction::Lock` request.
2. Resolve the graphical session from `XDG_SESSION_ID`; otherwise query the
   active local session for the current UID/seat.
3. Build a capability-ranked candidate list:
   - logind D-Bus / `loginctl lock-session <session>`;
   - `org.freedesktop.ScreenSaver.Lock` when available;
   - desktop-specific providers discovered from `XDG_CURRENT_DESKTOP`;
   - `xdg-screensaver lock`;
   - optional user-configured lock command, explicitly trusted.
4. Return a structured result containing provider, exit/error, duration, and
   whether a lock hint could be observed.
5. If the first provider is unsupported or does not produce an observable lock
   hint in a short verification window, try the next safe candidate.
6. Show a compact result toast and retain the detailed trace in Diagnostics.

No privileged command, password capture, or shell interpolation is required.

---

## 5. HDD and adaptive-layout design

### Root cause

The disk module currently produces a variable-length string such as:

```text
DISK root 21%  efi 1%  WD BLUE 83%  34°C
```

It then applies the same 30-character compact cap used by ordinary metrics.
Ellipsizing can hide the most useful disk, and it cannot solve total bar
pressure when start, center, and end zones compete for width.

GTK documents that ellipsizing changes a label's requested width; it is a text
fallback, not a full overflow algorithm:

- [GTK Label ellipsize documentation](https://docs.gtk.org/gtk4/property.Label.ellipsize.html)

### Disk snapshot model

```rust
struct StorageVolumeSnapshot {
    stable_id: String,       // UUID when available, otherwise canonical mount
    display_name: String,
    mount_point: PathBuf,
    filesystem: String,
    total_bytes: u64,
    available_bytes: u64,
    used_percent: f64,
    temperature_c: Option<f64>,
    removable: bool,
    stale: bool,
}
```

### User-selectable display policy

- **Auto:** root plus the most-used non-root volume;
- **Selected volumes:** user chooses one or more stable volume IDs;
- **Worst only:** volume with the highest usage percentage;
- **Aggregate:** combined used/total capacity where meaningful;
- **Count badge:** compact status plus number of additional mounted volumes.

### Width tiers

| Tier | Example | Intended width |
|---|---|---:|
| Tiny | `DISK 83%` | 56–80 px |
| Compact | `WD BLUE 83%` | 90–130 px |
| Normal | `root 21% · WD 83%` | 140–220 px |
| Detail | All selected volumes and hottest temperature | popover/tooltip only |

The full list remains accessible by tooltip, middle-click detail popover, and a
Storage page. A long mount name must never make the settings button or another
higher-priority module unreachable.

### Real adaptive allocator

Each module declares:

```rust
struct WidthProfile {
    tiny_px: u16,
    compact_px: u16,
    normal_px: u16,
    priority: u8,
    can_overflow: bool,
}
```

On allocation or configuration change:

1. reserve start/end critical controls and the Settings button;
2. assign normal widths by priority;
3. downgrade eligible modules to compact, then tiny;
4. move remaining low-priority modules into an overflow popover;
5. never overlap zones and never clip the entire module without a discoverable
   overflow representation.

Priority will finally control survival under pressure instead of only changing
render order.

---

## 6. Searchable Settings and module catalog

### Information architecture

The Settings window becomes:

```text
Overview
Bars & monitors
Modules
Appearance & effects
Data providers
Rules & notifications
Diagnostics
About
```

A persistent search field sits above the sidebar.

### Search behavior

- `Ctrl+F` focuses search; `Esc` clears it;
- searches page titles, setting labels, module names, aliases, IDs, categories,
  and descriptions;
- aliases include `hdd`, `ssd`, `storage`, and `drive` for the disk module;
- filters: **All**, **Enabled**, **Disabled**, **Built-in**, **Custom**, and
  category;
- results show the matching path, for example
  `Modules → Storage → Disk usage`;
- activating a result opens the correct page/editor and focuses the control;
- search never discards unsaved changes;
- an empty state explains which filters are active.

### Module registry

Hard-coded Settings cards and renderer metadata move behind one registry:

```rust
struct ModuleDescriptor {
    id: &'static str,
    name: &'static str,
    aliases: &'static [&'static str],
    category: ModuleCategory,
    description: &'static str,
    default_zone: Zone,
    width: WidthProfile,
    capabilities: ModuleCapabilities,
}
```

The same registry powers Settings search, default config, diagnostics,
accessibility names, overflow behavior, and documentation generation. This
prevents UI labels and renderer capabilities from drifting apart.

---

## 7. Visual direction: Adaptive Telemetry

### Design goals

- striking when effects are enabled, calm and readable by default;
- effects must never interfere with clicks, layout, text contrast, warning
  states, or reserved screen space;
- every visual option is configurable through Settings without requiring CSS;
- advanced CSS remains available as a separate override;
- the no-Wayland binary gets the same effects on X11;
- reduced-motion is a hard accessibility override, not a suggestion.

### Effect engine architecture

The bar becomes a `GtkOverlay` composition:

```text
GtkOverlay
├── EffectLayer (DrawingArea, cannot receive pointer input)
└── AdaptiveBarContent
    ├── Start zone
    ├── Center zone
    ├── End zone
    └── Overflow button + Settings
```

`EffectLayer` uses Cairo drawing and GTK's frame clock. GTK recommends
`add_tick_callback` and frame-clock time for continuous animation:

- [GTK Widget add_tick_callback](https://docs.gtk.org/gtk4/method.Widget.add_tick_callback.html)

The renderer queues only the effect layer for redraw. It must not regenerate
CSS, rebuild modules, poll metrics, or trigger layout every frame.

### Built-in effect presets

| Effect | Description | Motion fallback |
|---|---|---|
| Off | No animated layer; zero animation work | Off |
| Orbit Glow | One or more light points travel around the rounded bar perimeter | Static edge highlight |
| Ember Orbit | Orange/red luminous particles and a fading fire-like trail loop around the perimeter | Static warm gradient border |
| Aurora Sweep | Slow multi-color band sweeps horizontally | Static aurora tint |
| Energy Pulse | Accent halo expands subtly from a selected module or zone | Static halo |
| Spectrum Chase | Controlled multi-color perimeter chase | Static segmented border |
| Critical Beacon | Activates only while selected metrics are critical | Solid critical outline |

**Ember Orbit** directly covers the requested “cahaya api berjalan berputar
mengelilingi bar” concept. It is a bounded procedural trail, not an external GIF
or untrusted shader.

### Effect controls

- enabled and effect type;
- primary/secondary/trail colors;
- intensity and opacity;
- path thickness and glow radius;
- speed in loops per second;
- direction;
- trail length;
- particle density with a hard upper bound;
- frame cap: 24, 30, or 60 FPS;
- activation: always, hover, critical-only, or selected metric source;
- optional reaction source: CPU, temperature, network, or audio level;
- battery-saver policy;
- reduced-motion fallback.

### Rendering constraints

- draw inside the existing bar allocation so X11 struts and Wayland exclusive
  zones do not change;
- effect layer is non-interactive and cannot steal pointer events;
- clamp particle count, glow samples, alpha, speed, and frame rate during config
  migration and import;
- stop tick callbacks while the window is hidden/unmapped;
- reuse buffers/state; no allocation proportional to frame count;
- effects default to **Off** after migration and in safe mode.

---

## 8. Appearance and layout customization

### Theme Studio

Replace raw text fields as the primary workflow with:

- native color buttons plus validated CSS-value fallback;
- typography family, size, weight, and numeric style;
- bar background, opacity, blur-compatible tint, border, and shadow controls;
- module surface, border, corner, padding, gap, hover, warning, and critical
  tokens;
- live previews at 1366, 1920, and narrow simulated widths;
- light, dark, and high-contrast base modes;
- undo to last applied style and **Restore safe visual defaults**.

### Layout editor

- compact module catalog on the left;
- start/center/end drop zones;
- keyboard-accessible move controls in addition to drag-and-drop;
- normal/compact/tiny preview for each module;
- per-module visibility, priority, width mode, text template, colors, click
  action, and detail behavior;
- overflow preview and explanation of why an item moved there;
- profile preset saves layout + appearance + effects, not executable commands.

### Preset safety

- visual-only preset import is immediately previewable;
- commands, click actions, and scripts stay outside visual presets;
- imported executable behavior remains disabled and untrusted;
- every preset has a schema version and bounded values.

---

## 9. Backend work before “backend complete” status

Backend development should be considered mature for this cycle only when all
items below pass:

1. GTK main thread performs widget work only; provider I/O lives outside it.
2. Desktop actions cannot be delayed by package/update scans.
3. Provider status includes available, unavailable, stale, error, and last
   successful update.
4. Lock, screenshot, power, volume, brightness, and open-path actions return
   structured results and visible feedback.
5. X11 signed geometry has fixtures for left/right/above/below monitors.
6. Disk and network devices use stable identities where available.
7. Shutdown checks active inhibitors before final confirmation where supported.
8. Runtime diagnostics can be copied without exposing SSIDs, media titles,
   command output, or usernames by default.
9. No provider can create unbounded output, processes, tasks, or history.
10. A multi-hour soak test shows no growing worker/process/timer count.

### Backend features intentionally deferred

The backend is still extensible, but these features should not delay v3 visual
work:

- SMART/NVMe health through optional `smartctl`/`nvme` providers;
- battery health, charge rate, and power-profile controls;
- process search/detail/terminate with permission-aware confirmation;
- rule-based alerts and notification cooldown editor;
- CLI/D-Bus profile switching and diagnostics export;
- richer GPU engine metrics;
- Snap package updates.

They can enter later 3.x releases after the stabilization gate, based on actual
user demand.

---

## 10. Target source architecture

The current 4,255-line `main.rs` should be split incrementally without a
big-bang rewrite:

```text
src/
├── main.rs
├── app.rs
├── config/
│   ├── mod.rs
│   ├── schema_v3.rs
│   └── migration.rs
├── backend/
│   ├── mod.rs
│   ├── scheduler.rs
│   ├── action_executor.rs
│   ├── capability.rs
│   ├── snapshot.rs
│   └── providers/
│       ├── system.rs
│       ├── storage.rs
│       ├── network.rs
│       ├── audio.rs
│       ├── desktop.rs
│       └── packages.rs
├── model/
│   ├── module_registry.rs
│   ├── layout.rs
│   └── effects.rs
└── ui/
    ├── bar.rs
    ├── adaptive_layout.rs
    ├── effect_layer.rs
    ├── overflow.rs
    ├── settings/
    │   ├── mod.rs
    │   ├── search.rs
    │   ├── modules.rs
    │   └── theme_studio.rs
    ├── history.rs
    ├── processes.rs
    └── sensors.rs
```

### Data flow

```text
Providers ──> typed immutable RuntimeSnapshot ──> GTK render diff
Actions   ──> high-priority ActionResult       ──> button/toast/diagnostics
Config v3 ──> validated view model             ──> layout + style + effects
FrameClock ────────────────────────────────────> effect redraw only
```

GTK widgets never cross threads. Backend snapshots contain plain owned data.
The UI applies only changed values to avoid rebuilding unchanged widget trees.

---

## 11. Configuration schema v3

### New concepts

```toml
schema_version = 3

[layout]
adaptive = true
overflow = "priority-popover"
reserve_settings_button = true

[effects]
enabled = false
kind = "off"
primary = "#FF8A3D"
secondary = "#FFD36A"
speed = 0.22
intensity = 0.65
trail = 0.28
thickness = 2.0
fps_cap = 30
activation = "always"
react_to = "none"
reduce_motion_fallback = "static"

[storage]
mode = "auto"
selected = []
show_temperature = true

[desktop_actions]
lock_provider = "auto"
lock_command = ""
lock_command_trusted = false
```

Per-module layout gains explicit width mode and overflow behavior. Existing
zone, priority, compact, template, colors, and trust fields are preserved.

### Migration guarantees

- make a timestamped v2 backup before first schema-3 save;
- preserve all enabled modules, zones, priorities, colors, templates, custom
  modules, and command trust;
- map `animated_bg = true` to a disabled preview suggestion rather than silently
  enabling a new continuous effect;
- default effect to Off;
- convert disk behavior to Auto while retaining full detail in its popover;
- unknown effect/provider values become safe defaults;
- rollback instructions remain documented and tested with a copied profile.

---

## 12. Implementation roadmap

Durations are engineering estimates, not release promises. Each phase has a
separate terminal gate, manual test, documentation update, and source archive.

### Phase 0 — Reproduction and regression harness (2–3 days)

- capture desktop/session diagnostics for the lock failure;
- add pure X11 geometry fixtures, including negative coordinates;
- add disk formatter fixtures for long labels and many mounts;
- add scheduler/action-order tests;
- define width test fixtures for 1024, 1280, 1366, 1920, and scaled displays;
- record a current v2 screenshot for before/after comparison.

**Gate:** each reported bug has a failing automated fixture or an explicit
manual reproduction checklist before its fix.

### Phase 1 — Reliability hotfix layer (3–5 days)

- introduce high-priority desktop action executor;
- implement lock capability chain and visible action result;
- restore signed-safe X11 geometry calculation;
- implement disk tiny/compact/normal formatting;
- add a temporary Settings module search before the full registry migration.

**Gate:** lock works or reports an actionable provider error; HDD remains
visible on 1366 px with default modules; no negative-coordinate underflow.

### Phase 2 — Backend snapshot and module registry (5–7 days)

- move sysinfo, `/sys`, `/proc`, NVML, storage, and network collection off GTK;
- split slow package jobs from action work;
- add provider freshness/error metadata;
- create `ModuleDescriptor` registry;
- replace full per-tick widget mutation with snapshot diffs.

**Gate:** no provider I/O on GTK thread; lock latency remains bounded while all
package backends time out.

**Phase 2A implemented in `3.0.0-alpha.2`:** fast external providers are
isolated from package/release jobs and from desktop actions; scheduled providers
publish freshness/error/in-flight metadata; Settings shows redacted diagnostics;
and all 38 built-ins use a tested `ModuleDescriptor` registry for canonical
names, categories, aliases, and capability identities.

**Phase 2B implemented in `3.0.0-alpha.3`:** one background core collector owns
`sysinfo`, `/sys`, `/proc`, NVML, storage, network, battery, GPU, and native bar
metrics; GTK consumes immutable generations and reconciles only changed samples.
Process rows, the sensor viewer, and the hwmon picker also scan on workers, and a
static source gate prevents provider I/O from returning to the GTK renderer.

**Phase 2 gate passed with `3.0.0-alpha.3` r13:** formatting, 30 tests, strict
Clippy, locked no-Wayland build, smoke, and the manual responsiveness/freshness
matrix all passed. Phase 3 work is now authorized.

### Phase 3 — Adaptive layout and searchable Settings (5–7 days)

- implement pixel-budget allocator and overflow popover;
- add per-module width profiles and explicit disk policies;
- convert modules to searchable catalog + lazy editor;
- add global Settings search, aliases, filters, and keyboard flow;
- add responsive Settings layouts and empty/error states.

**Gate:** no overlap or inaccessible Settings button at supported widths/scales;
all 38 built-ins are discoverable through search.

**Phase 3A implemented in `3.0.0-alpha.4`:** deterministic pixel budgeting,
registry-backed normal/compact/tiny profiles, priority overflow, a permanent More
popover with live values, explicit disk width tiers, and resize-safe renderer
invalidation. The candidate adds nine regression tests.

**Phase 3A gate passed with `3.0.0-alpha.4` r14. Phase 3B is implemented in
`3.0.0-alpha.5`:** global search, composable filters, a compact unified catalog,
lazy editors, Ctrl+F/Esc, responsive navigation/panes, and explicit empty states.
More is now clickable even with zero overflow.

**Still required for the Phase 3 gate:** alpha.5 must pass formatting, 44 tests,
strict Clippy, locked build, smoke, and the complete Settings/manual regression
matrix. Visual effects remain blocked until that evidence passes.

### Phase 4 — Visual-effect engine and Theme Studio (7–10 days)

- add `GtkOverlay` effect layer and frame-clock lifecycle;
- implement Off, Orbit Glow, Ember Orbit, Aurora Sweep, and Critical Beacon;
- expose bounded controls and live preview;
- integrate reduced-motion, battery policy, safe mode, and visual presets;
- remove the CSS-recompile animation path.

**Gate:** effects do not change geometry, consume pointer events, reduce text
contrast below target, or run while disabled/hidden.

### Phase 5 — Layout editor, polish, and accessibility (5–7 days)

- zone editor with drag and keyboard controls;
- profile-style visual presets;
- action toasts and richer detail popovers;
- focus order, accessible names, 44 px Settings targets, high contrast;
- update history/process/sensor windows to shared design components.

**Gate:** complete keyboard flow and reduced-motion flow; no advanced CSS is
required for ordinary customization.

### Phase 6 — Hardening and release (4–6 days)

- full no-Wayland and Wayland matrices;
- installed upgrade from 2.0.1, config migration, rollback, autostart;
- visual regression captures and current screenshots;
- CPU/memory/frame-time soak tests;
- package, AppStream, README, changelog, technical, migration, testing, security,
  and release-checklist synchronization.

**Gate:** all v3 acceptance criteria below pass before `.deb` publication.

---

## 13. Test strategy

### Unit tests

- action candidate ordering and desktop capability detection;
- high-priority action scheduling while slow providers are blocked;
- lock result/error normalization;
- signed X11 geometry for all monitor quadrants;
- storage filtering, stable IDs, sorting, and width-tier text;
- adaptive allocator downgrade/overflow determinism;
- search normalization, aliases, category/filter combinations;
- config v2 → v3 migration and invalid-value clamping;
- effect path position, speed, trail bounds, and reduced-motion fallback;
- snapshot freshness and stale/error transitions.

### GTK/integration tests

- widget tree contains permanent Settings and overflow controls;
- search result activates and focuses the expected editor;
- Save, Discard, close, import, reset, and live preview remain transactional;
- effect layer cannot receive pointer input;
- hidden/unmapped window stops animation work;
- no-Wayland build contains no layer-shell dependency.

### Manual desktop matrix

| Environment | Display | Required checks |
|---|---|---|
| Ubuntu GNOME | X11 | docking, lock, search, disk overflow, effects |
| Linux Mint Cinnamon | X11 | lock fallback, panel coexistence, multi-disk names |
| Xfce | X11 | lock provider, struts, screenshot action |
| KDE Plasma | X11 or Wayland | lock provider, scaling, visual effects |
| GNOME/KDE compositor | Wayland full build | layer-shell geometry and effects |
| Wayland session | no-Wayland build | documented floating fallback only |

### Performance budgets

Measured on a declared reference machine with the same module set:

- effect Off: no persistent frame callback and no measurable animation CPU;
- Ember Orbit at 30 FPS: effect render p95 under 3 ms and no missed input;
- backend collection never blocks a GTK frame;
- quick action accepted within 100 ms and dispatched within 250 ms when the
  system is responsive;
- package timeout cannot delay a desktop action;
- no monotonic memory, thread, child-process, or timer growth in an 8-hour soak;
- Settings search responds within 50 ms for built-in plus 200 custom modules.

---

## 14. Visual and accessibility acceptance criteria

- no overlapping or horizontally overflowing bar controls;
- HDD remains visible in at least Tiny mode when enabled and not explicitly sent
  to overflow;
- Settings button is always reachable;
- every hidden module is represented in the overflow popover;
- effect layer remains inside the bar allocation;
- text and meaningful controls meet WCAG AA contrast targets;
- warning/critical state never depends on motion or color alone;
- Reduce motion stops all continuous motion and uses the configured static
  fallback;
- keyboard users can search, select, edit, move, save, discard, and reset;
- all Settings controls have accessible names and visible focus;
- effect presets look intentional at 100%, 125%, 150%, and 200% scale;
- no pseudo-text images, external animation assets, or fake status information.

---

## 15. Release acceptance criteria

TopMonitoring 3 is release-ready only when:

- [ ] V3-001 through V3-004 are closed with regression evidence;
- [ ] lock works on the target desktop or shows a precise actionable failure;
- [ ] disk formatting and overflow pass all width fixtures;
- [ ] no synchronous provider I/O remains on GTK's main loop;
- [ ] quick actions are isolated from slow provider scheduling;
- [ ] Settings search covers built-in and custom modules;
- [ ] effects default Off, obey reduced motion, and stop while hidden;
- [ ] no-Wayland X11 and full Wayland feature matrices pass fmt/test/Clippy/build;
- [ ] package install, upgrade, autostart, migration, rollback, and uninstall pass;
- [ ] manual X11 docking and visual tests pass on the user's machine;
- [ ] visual screenshots are inspected at supported widths/scales;
- [ ] documentation is updated in the same commit as behavior/config changes;
- [ ] source archive and `.deb` have published SHA-256 values;
- [ ] GitHub publication happens only after installed-package confirmation.

---

## 16. Non-goals and guardrails

- no arbitrary downloadable shader/plugin execution in 3.0;
- no user-supplied effect code;
- no unbounded particles, blur passes, frame rates, logs, or subprocesses;
- no Electron/webview rewrite;
- no dropping X11/no-Wayland support;
- no visual effect enabled by migration without explicit user choice;
- no feature expansion that bypasses the P0 regression gate;
- no `.deb` created merely because compilation passes;
- no implementation begins until this plan is approved.

---

## 17. Proposed branch and commit sequence

```text
major/v3
fix/actions-lock-capability
fix/x11-signed-geometry
fix/storage-responsive-renderer
refactor/runtime-snapshot-scheduler
refactor/module-registry
feat/settings-search
feat/adaptive-overflow
feat/effect-layer
feat/ember-orbit
feat/theme-studio
feat/layout-editor
test/v3-regression-matrix
docs/v3-release
```

Keep changes reviewable. Do not combine the worker rewrite, adaptive allocator,
and effect renderer into one commit.

---

## 18. Approved implementation scope

Approved scope:

1. approve **TopMonitoring 3.0.0 — Adaptive Visual Engine**;
2. require P0 reliability fixes before visual effects;
3. use **Ember Orbit** as the signature visual preset;
4. keep effects Off by default and cap the default animated preset at 30 FPS;
5. implement searchable Settings and real adaptive overflow in the first v3
   release, not as later polish;
6. defer optional backend expansion until the v3 stabilization gate passes;
7. retain terminal-first testing and documentation synchronization for every
   update.

Implementation starts with Phase 0 and Phase 1 only. A new source candidate is
produced after those gates pass; the visual engine is not layered on top of
unresolved lock, disk, X11, or scheduling defects.

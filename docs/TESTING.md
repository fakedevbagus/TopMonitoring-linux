# Testing TopMonitoring Updates

This workflow tests a new **no-Wayland** build directly from the source tree.
It does not replace the currently installed package and uses an isolated XDG
configuration, cache, and data directory.

## 1. Prerequisites (Ubuntu/Debian)

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libgtk-4-dev \
  libsensors-dev lm-sensors python3
rustup toolchain install stable --component clippy rustfmt
cargo install cargo-deb --locked   # needed only when packaging
```

A no-Wayland build does not require `libgtk4-layer-shell-dev`.

## 2. Extract or update the source

```bash
unzip TopMonitoring-linux-v3.0.0-alpha.5-source-r16.zip
cd TopMonitoring-linux-v3.0.0-alpha.5-r16
cat SOURCE_CANDIDATE
```

The marker must print `3.0.0-alpha.5-r16`. Do not extract this archive over an
older alpha directory; the revision-specific top-level directory prevents stale
scripts or source files from being reused accidentally.

For the last stable source instead, use the 2.0.1 r7 archive documented in its
release notes. Never mix files from the stable and alpha source trees.

When testing a Git checkout, use a dedicated branch and make sure its working
tree is clean before packaging.

## 3. Terminal quality gate

```bash
./scripts/test-no-wayland.sh check
```

This must pass, in order:

1. `cargo fmt --check`
2. unit tests with `--no-default-features`
3. Clippy with warnings denied
4. locked no-Wayland release build
5. repository, shell, and release-document checks

Do not create a `.deb` while any step fails. The gate is intentionally
fail-fast: fix the first reported phase, rerun the complete command, and retain
the new log as evidence. A rustfmt diff means formatting failed; run
`cargo fmt --all` before rerunning the gate.

## 4. GUI smoke test without replacing the installed package

Stop the running installed process, but do **not** uninstall it:

```bash
pkill -x topmonitoring || true
./scripts/test-no-wayland.sh smoke
```

The script runs `target/release/topmonitoring`, not `/usr/bin/topmonitoring` or
`/usr/local/bin/topmonitoring`. Its test config is stored under
`.test-state/no-wayland`, so the installed profile remains untouched. Logs are
written under `test-artifacts/no-wayland`.

Check the session first:

```bash
echo "$XDG_SESSION_TYPE"
```

Use an **X11 session** for the authoritative no-Wayland docking test. On a
Wayland session, this build can only use its floating fallback and cannot prove
that X11 struts reserve screen space.

## 5. Manual test

```bash
./scripts/test-no-wayland.sh manual
```

Keep the terminal open and verify:

- bar starts once and reserves the intended X11 edge;
- top/bottom position, monitor selection, height, and offset;
- Settings Save, Discard, Reset, close/reopen, and live preview;
- start/center/end zones, priority, compact mode, colors, and templates;
- CPU, RAM, temperature, disk, network, GPU, battery, and graphs;
- volume, mute, brightness, Wi-Fi, Bluetooth, media, and package updates;
- custom module timeout, output cap, trust, Run now, duplicate, and click action;
- imported commands remain disabled and untrusted;
- history recording, dashboard, CSV export, rotation, and confirmed clearing;
- process manager, hardware sensors, notifications, and quick actions;
- `pkill -SIGUSR1 topmonitoring` toggles visibility;
- no repeated errors, freezes, runaway CPU, or orphan processes in the log.

Press `Ctrl+C` after testing.

### Test migration using a copy of the installed config

Run this only after the clean-profile test passes:

```bash
./scripts/test-no-wayland.sh clean-state
TOPMONITORING_COPY_CONFIG=1 ./scripts/test-no-wayland.sh manual
```

The original `~/.config/topmonitoring/config.toml` is copied, never modified.

## 6. Package only after confirmation

```bash
./scripts/package-no-wayland.sh --confirmed-manual
```

The packaging script refuses a dirty Git tree, repeats the automated gate,
validates required documentation/version metadata, builds the no-Wayland
variant, inspects package contents, runs `lintian` when available, and creates
a SHA-256 file.

Inspect and install the candidate:

```bash
package=$(find target/debian -name '*.deb' -type f | head -n1)
dpkg-deb --info "$package"
dpkg-deb --contents "$package"
sudo apt install "./$package"
dpkg-query -W -f='${Version}\n' topmonitoring
```

Keep the previous known-good `.deb` until the candidate has passed installed
startup, autostart, upgrade, and uninstall/reinstall checks.

When upgrading a machine that still has `topmonitoring-no-wayland` 1.x, APT
should report removal/replacement of that legacy package instead of a shared-file
error. Verify the candidate control metadata before installation:

```bash
dpkg-deb -f "$package" Package Version Depends Conflicts Replaces
```

For the no-Wayland artifact, `Depends` must not contain
`libgtk4-layer-shell0`; both `Conflicts` and `Replaces` must include
`topmonitoring-no-wayland`.

## 7. Documentation gate for every update

Before GitHub publication, update the files listed in
[`RELEASE_CHECKLIST.md`](../RELEASE_CHECKLIST.md). In particular:

- every code change: `CHANGELOG.md` and relevant technical documentation;
- UI/behavior change: `README.md` and `docs/TECHNICAL.md`;
- config/schema change: `MIGRATION_V2.md`;
- release/version change: `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, README
  package examples, and AppStream metadata;
- screenshot whenever the visible interface changes materially.

## 8. TopMonitoring 3 alpha.2 provider-isolation gate — passed

Alpha.2 passed formatting, tests, strict Clippy, locked release build, smoke,
and the manual provider-isolation matrix. Fast/slow provider health,
diagnostics redaction, action responsiveness, module-registry search, and the
alpha.1 regressions are now the baseline for alpha.3.

## 9. TopMonitoring 3 alpha.3 core-snapshot gate — passed

Normalize the source first, then run the complete fail-fast gate:

```bash
cargo fmt --all
cargo fmt --all -- --check
./scripts/test-no-wayland.sh check
```

Before any additional test is introduced, alpha.3 contains **30 unit tests**:
the previous 24 plus core metric/graph diffs, redacted core diagnostics,
collector configuration clamping, external-field diff isolation, and owned
process-row sorting. Accept only an exact all-pass result with zero Clippy
warning and a successful locked no-Wayland release build.

Run the candidate without replacing the installed package:

```bash
pkill -x topmonitoring || true
./scripts/test-no-wayland.sh smoke
./scripts/test-no-wayland.sh manual
```

Verify all of the following on the isolated manual profile:

1. Open **Settings → Overview → Runtime**. A `core=` line must progress from
   unavailable to available and show increasing generation, bounded duration,
   a sensible stale deadline, metric count, and interface count. It must not
   expose hostname, interface names, mount paths, or metric values.
2. CPU, RAM, swap, temperature, fan, CPU power/Vcore, GPU metrics, disk/HDD,
   disk I/O, network rate/totals, VPN, battery, process count, uptime, load, and
   host/kernel/OS must still match the machine or degrade to `n/a` safely.
3. Watch CPU/RAM/network graphs for several collection periods. They must append
   once per changed core sample, remain smooth, and not accelerate when Settings
   or another auxiliary window is open.
4. Change the selected network interface and GPU index. The collector must
   reconfigure, rebuilt widgets must receive a complete first value, and no old
   interface/GPU sample may remain indefinitely.
5. Open/close **Process Manager**, **Hardware Sensors**, and **Add sensor from
   hwmon** repeatedly. Windows must remain responsive while scans run; closing
   them must not create continuing worker/thread growth.
6. Start package/release work and repeatedly use Lock, Capture, volume, and
   brightness while core collection is active. Actions must remain prompt and
   must not wait for package, `/sys`, `/proc`, disk, network, or NVML work.
7. Simulate unavailable optional backends where practical. The bar and Settings
   must remain responsive, stale/error/unavailable states must be truthful, and
   no retry loop may flood logs or processes.
8. Re-run alpha.1 Lock, compact HDD, Settings search, and signed negative-origin
   multi-monitor X11 regressions, plus all alpha.2 provider diagnostics checks.
9. Leave the candidate running for at least ten minutes. Watch for GTK freezes,
   panics, channel disconnect noise, duplicate graph samples, unbounded memory,
   or sustained idle CPU regression.

Alpha.3 r13 passed formatting, 30 tests, strict Clippy, locked release build,
smoke, and the complete manual Phase 2B matrix. Preserve that evidence as the
backend baseline.

## 10. TopMonitoring 3 alpha.4 adaptive-layout gate — passed

Run the fail-fast terminal gate first:

```bash
cargo fmt --all
cargo fmt --all -- --check
./scripts/test-no-wayland.sh check
```

Alpha.4 contains **39 unit tests**: the previous 30 plus six pure allocator
fixtures and tests for registry width bounds, disk normal/compact/tiny output,
reserved controls, and owned tier values. Accept only 39/39, zero Clippy
warning, and a successful locked no-Wayland release build.

Only after terminal PASS:

```bash
pkill -x topmonitoring || true
./scripts/test-no-wayland.sh smoke
TOPMONITORING_COPY_CONFIG=1 ./scripts/test-no-wayland.sh manual
```

Manual Phase 3A matrix:

1. Confirm the first log line is `Candidate: 3.0.0-alpha.4-r14` and Settings is
   always visible beside the permanent More control.
2. Enable enough low-priority built-ins and custom modules to exceed a 1366 px
   X11 bar. Labels must step through normal, compact, and tiny widths without
   overlap before low-priority items enter **More**.
3. Raise one module's priority to 100 and lower another to 0. After live apply,
   the priority-100 module must survive longer and the priority-0 module must be
   the earlier overflow candidate; equal-priority behavior must remain stable.
4. Open More and verify every hidden module appears once with a live value.
   Values must continue changing while hidden, and increasing available width
   must restore the original widgets without duplicate modules.
5. Test Disk/HDD with root plus a long secondary volume name. Normal should show
   root plus the fullest volume, compact should show the fullest bounded name,
   tiny should show `DISK nn%`, and the tooltip must retain full detail.
6. Disable **Adaptive layout tiers** for selected modules. Those modules must
   skip compact/tiny but may still move into More so Settings remains reachable.
7. Enable Lock, Capture, and Power quick actions together. Their reserved budget
   must not overlap More/Settings, and Lock/Capture responsiveness must retain
   the alpha.3 baseline during package and core work.
8. Watch CPU/RAM/network graphs while changing width pressure. A tier change may
   resize/redraw but must not append duplicate history samples or restart
   providers. Idle CPU and memory must remain stable for at least ten minutes.
9. Re-run alpha.1–alpha.3 Lock, X11 signed geometry, provider diagnostics,
   process/sensor windows, and Settings transaction/search regressions.

Alpha.4 r14 passed the complete terminal, smoke, width, overflow, and prior
regression matrix. Preserve it as the adaptive-layout baseline.

## 11. TopMonitoring 3 alpha.5 searchable-Settings gate

Run `cargo fmt --all`, `cargo fmt --all -- --check`, then
`./scripts/test-no-wayland.sh check` as a fail-fast sequence. Alpha.5 expects
**44 unit tests**, zero Clippy warning, and a locked release build.

Only after PASS, run smoke and manual modes. Verify: More opens both empty and
`More N` states; Ctrl+F/Esc; HDD/Wi-Fi aliases; all state/kind/category filters;
stable selection when a row is filtered out and restored; lazy built-in/custom
editing; add/duplicate/remove/run custom actions; explanatory no-result states;
sidebar-to-dropdown behavior below 760 px; horizontal-to-vertical catalog below
900 px; Save/Discard/Reset; and all alpha.1–alpha.4 regressions. Leave Settings
open ten minutes and repeatedly reopen it to check CPU, memory, and stale UI.

Do **not** create a `.deb` for `3.0.0-alpha.5`. Preserve logs and return any
rustfmt-modified source before Phase 4 begins.

## 12. Settings minimum-height warning fix gate — 2026-09-01

The manual alpha.5 session recorded 542 recurring Gtk-CRITICAL warnings:
`GtkBox reports a minimum height of 363, but minimum height for width of
1048576 is 395` plus `GtkPaned ... 140 ... 172` (see
`test-artifacts/no-wayland/manual-20260816-234513.log`).

Root cause: both Settings stacks (`GtkStack` for the five pages and
`GtkStack` for the module catalog list/empty states) were homogeneous. A
homogeneous stack measures every child at two different widths each frame,
so the reported minimum height mixed results from different pages and the
1-second refresh timer re-emitted the warning continuously. The widgets are
scrolled/filled, so a shared size is not required.

Fix: set `hhomogeneous(false)`/`vhomogeneous(false)` on both stacks and
document two diagnostic hooks: `TOPMONITORING_AUTO_SETTINGS=1` auto-opens
Settings ~2s after launch, and `TOPMONITORING_SETTINGS_PAGE=<name>` forces
the initial page for per-page bisection.

Verification (2026-09-01, release build, isolated config, 14s per page):
`overview=0 bars=0 appearance=0 modules=0 tools=0` Gtk-CRITICAL warnings,
compared with 26–69 per page before the fix. The full terminal gate re-passed
after the change: `cargo fmt --check`, 44/44 tests on `--all-features` and
`--no-default-features`, zero Clippy warnings on both feature matrices,
release builds, and `scripts/check_source.py`.

## 13. Effect engine gate — Step 0.2/0.3 — 2026-09-01

`src/effects.rs` implements the frame-clock effect engine with pure state
(`EffectKind`, `EffectParams`, `EffectState`) and a thin GTK executor
(non-interactive `DrawingArea` overlay via `GtkOverlay`). The bar now
composes `GtkOverlay > root` so the effect layer paints above modules
without touching CSS providers. The legacy `animated_bg` hue-recompile path
was removed and `scripts/check_source.py` now rejects `load_from_string`
inside the polling loop.

Verification (2026-09-01): 52/52 unit tests on both feature matrices, zero
Clippy warnings with `-D warnings` on both matrices, release build ok,
source hygiene ok. Headless smoke: 12 s runs with isolated config produced
0 Gtk-CRITICAL and 0 panics with effects disabled and with
`effects_kind = "aurora-sweep"` enabled. Battery policy (`discharging` via
`/sys/class/power_supply`) disables animation; `reduce_motion` forces
`static-fallback`; unknown `effects_kind` values validate back to `off`.

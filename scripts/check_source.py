#!/usr/bin/env python3
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
forbidden = {".crdownload", ".tmp", ".swp", ".bak"}
problems = []
for path in root.rglob("*"):
    if ".git" in path.parts or "target" in path.parts:
        continue
    if path.is_file() and any(path.name.endswith(suffix) for suffix in forbidden):
        problems.append(f"temporary file committed: {path.relative_to(root)}")
for required in [
    "SOURCE_CANDIDATE",
    "SECURITY.md",
    "README.md",
    "CHANGELOG.md",
    "topmonitoring.desktop",
    "data/io.github.fakedevbagus.TopMonitoring.metainfo.xml",
    "docs/TESTING.md",
    "RELEASE_CHECKLIST.md",
    "scripts/test-no-wayland.sh",
    "scripts/package-no-wayland.sh",
    "scripts/check_release_docs.py",
    "src/app.rs",
    "src/backend.rs",
    "src/lib.rs",
    "src/layout.rs",
]:
    path = root / required
    if not path.exists() or path.stat().st_size == 0:
        problems.append(f"missing or empty required file: {required}")

# Module-wide warning suppression hides stale code during refactors. Keep any
# exceptional lint allowance narrow and attached to the affected item instead.
for source_path in (root / "src").rglob("*.rs"):
    for line_number, line in enumerate(source_path.read_text().splitlines(), start=1):
        stripped = line.strip()
        if stripped.startswith("#![allow(") and any(
            lint in stripped for lint in ("unused", "dead_code")
        ):
            relative = source_path.relative_to(root)
            problems.append(
                f"module-wide warning suppression is forbidden: {relative}:{line_number}"
            )

main_source = (root / "src/main.rs").read_text()
app_source = (root / "src/app.rs").read_text()
orchestration_source = app_source if "fn build_bar(" in app_source else main_source
poll_marker = (
    "let source = glib::timeout_add_local(Duration::from_millis(step_ms), move || {"
)
poll_start = orchestration_source.find(poll_marker)
poll_end = orchestration_source.find(
    "*poll_source.borrow_mut() = Some(source);", poll_start
)
if poll_start < 0 or poll_end < 0:
    problems.append("could not locate the GTK snapshot reconciliation callback")
else:
    poll_source = orchestration_source[poll_start:poll_end]
    for token in [
        "refresh_cpu_all",
        "refresh_memory",
        ".refresh(true)",
        "System::new",
        "Networks::new",
        "Nvml::",
        "std::fs::",
        '"/sys/',
        '"/proc',
        # The effect engine redraws its own overlay layer; rebuilding a CSS
        # provider inside the polling loop is the removed animated_bg path
        # and must not come back (V3-008 / Step 0.3).
        "load_from_string",
    ]:
        if token in poll_source:
            problems.append(f"provider I/O leaked into GTK polling callback: {token}")

render_start = orchestration_source.find("fn update_metric(")
render_end = orchestration_source.find("fn install_autostart(", render_start)
# After Fase A.1 extraction, renderer lives in src/ui/bar.rs; accept either location
if render_start < 0 or render_end < 0:
    bar_render_start = (root / "src/ui/bar.rs").read_text().find("fn update_metric(")
    bar_render_end = (root / "src/ui/bar.rs").read_text().find("fn install_autostart(", bar_render_start)
    if bar_render_start < 0 or bar_render_end < 0:
        problems.append("could not locate snapshot-backed metric renderer")
    else:
        renderer = (root / "src/ui/bar.rs").read_text()[bar_render_start:bar_render_end]
        for token in ["std::fs::", "System::", "Networks::", "Nvml::", '"/sys/', '"/proc']:
            if token in renderer:
                problems.append(f"provider I/O leaked into metric renderer: {token}")
else:
    renderer = orchestration_source[render_start:render_end]
    for token in ["std::fs::", "System::", "Networks::", "Nvml::", '"/sys/', '"/proc']:
        if token in renderer:
            problems.append(f"provider I/O leaked into metric renderer: {token}")

test_script = (root / "scripts/test-no-wayland.sh").read_text()
launch_contract = (
    "cargo build --release --locked --no-default-features\n"
    'binary="$ROOT/target/release/topmonitoring"'
)
if launch_contract not in test_script:
    problems.append("GUI test modes do not build the release binary they launch")

layout_source = (root / "src/layout.rs").read_text()
for token in ["WidthProfile", "LayoutTier", "allocate_layout"]:
    if token not in layout_source:
        problems.append(f"adaptive layout module is missing {token}")
bar_source = (root / "src/ui/bar.rs").read_text()
settings_mod_source = (root / "src/ui/settings/mod.rs").read_text()
catalog_source = (root / "src/ui/settings/catalog.rs").read_text()
combined_bar_settings = orchestration_source + bar_source + settings_mod_source + catalog_source + (root / "src/ui/settings/search.rs").read_text() + (root / "src/ui/settings/layout.rs").read_text()
for token in ["OverflowUi", "apply_adaptive_layout", "overflow-button", "overflow-empty"]:
    if token not in combined_bar_settings and token not in (root / "src/config.rs").read_text():
        problems.append(f"adaptive overflow UI is missing {token}")
if "overflow.button.set_sensitive(false)" in combined_bar_settings:
    problems.append("permanent More control must remain clickable when overflow is empty")
for token in [
    "ModuleCatalogKey",
    "build_module_catalog_page",
    "module_catalog_matches",
    "settings_page_search_target",
    "EventControllerKey",
    "module-catalog-empty",
]:
    if token not in combined_bar_settings and token not in (root / "src/config.rs").read_text():
        problems.append(f"responsive Settings catalog is missing {token}")
# Bar renderer now lives in src/ui/bar.rs after Fase A.1 extraction
bar_renderer = (root / "src/ui/bar.rs").read_text()
for token in ["std::fs::", "System::", "Networks::", "Nvml::", '"/sys/', '"/proc']:
    if token in bar_renderer[max(0, bar_renderer.find("fn update_metric(")):bar_renderer.find("fn install_autostart(")]:
        problems.append(f"provider I/O leaked into metric renderer (bar.rs): {token}")
if problems:
    print("\n".join(problems), file=sys.stderr)
    raise SystemExit(1)
print("source hygiene: ok")

#![allow(unused, dead_code)]
//! Settings window — extracted from `main.rs` (Fase A.1).
pub mod catalog;
pub mod layout;
pub mod search;

pub(crate) use catalog::{build_module_catalog_page, ModuleCatalogKey, ModuleCatalogRecord};
pub(crate) use layout::{collect_layout_rows, render_module_layout};
pub(crate) use search::search_text_matches;

use crate::config::{
    build_css, build_custom_css, css_class_id, read_import, Config, CustomModule, ThemePreset,
};
use crate::history::history_log_path;
use crate::runtime;
use crate::runtime::RuntimeServices;
use crate::ui::bar::{
    add_module_classes, install_autostart, Active, ApplyActions, RefreshSlot, SettingsContext,
};
use crate::ui::settings::catalog::{
    append_catalog_detail_empty, module_catalog_matches, module_catalog_record,
    render_builtin_module_editor, render_custom_module_editor,
};
use crate::ui::settings::layout::{
    labeled_row, settings_page, settings_section, zone_dropdown, zone_name,
};
use crate::ui::settings::search::{
    search_text_matches as search_matches, settings_page_search_target,
};
use crate::ui::{open_history_dashboard, open_process_manager, open_sensors};
use gtk::gio::prelude::FileExt;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, DropDown, Entry, EventControllerKey, FileDialog, Label, ListBox,
    Orientation, Paned, ScrolledWindow, SearchEntry, SpinButton, Stack, StackSidebar, Switch,
    TextView, Window,
};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

pub(crate) fn open_settings(
    cfg: &Rc<RefCell<Config>>,
    is_wayland: bool,
    context: &SettingsContext,
) {
    let bar_window = &context.bar_window;
    let active = &context.active;
    let runtime = &context.runtime;
    let core = &context.core;
    let actions = &context.actions;
    let settings_slot = &context.settings_slot;
    if let Some(existing) = settings_slot.borrow().as_ref() {
        existing.present();
        return;
    }

    let window = Window::builder()
        .title("TopMonitoring 3 Alpha — Settings")
        .transient_for(bar_window)
        .default_width(980)
        .default_height(760)
        .build();
    *settings_slot.borrow_mut() = Some(window.clone());
    let original = Rc::new(RefCell::new(cfg.borrow().clone()));

    let shell = GtkBox::new(Orientation::Vertical, 0);
    shell.add_css_class("settings-shell");

    let settings_search_header = GtkBox::new(Orientation::Horizontal, 10);
    settings_search_header.add_css_class("settings-search-header");
    let compact_navigation = DropDown::from_strings(&[
        "Overview",
        "Bars & layout",
        "Appearance",
        "Modules",
        "Data & tools",
    ]);
    compact_navigation.add_css_class("compact-settings-nav");
    compact_navigation.set_visible(false);
    settings_search_header.append(&compact_navigation);
    let search_column = GtkBox::new(Orientation::Vertical, 2);
    search_column.set_hexpand(true);
    let global_search = SearchEntry::new();
    global_search.set_hexpand(true);
    global_search.set_placeholder_text(Some("Search all settings or modules…  Ctrl+F"));
    search_column.append(&global_search);
    let global_search_status = Label::new(Some(
        "Search pages, controls, built-ins, aliases, and custom commands.",
    ));
    global_search_status.add_css_class("settings-caption");
    global_search_status.set_xalign(0.0);
    global_search_status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    search_column.append(&global_search_status);
    settings_search_header.append(&search_column);
    shell.append(&settings_search_header);

    let body = GtkBox::new(Orientation::Horizontal, 0);
    body.set_vexpand(true);
    let stack = Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    // Pages live inside ScrolledWindows, so they do not need a shared size.
    // A homogeneous stack measures every page at two different widths each
    // frame, which triggers recurring "minimum height ... Expect overlapping
    // widgets" Gtk-CRITICAL warnings from the 1s refresh timer.
    stack.set_hhomogeneous(false);
    stack.set_vhomogeneous(false);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    let sidebar = StackSidebar::new();
    sidebar.set_stack(&stack);
    sidebar.add_css_class("settings-sidebar");
    body.append(&sidebar);
    body.append(&stack);
    shell.append(&body);

    let status = Label::new(Some("Changes preview live. Save to persist."));
    status.add_css_class("settings-caption");
    status.set_xalign(0.0);
    status.set_hexpand(true);
    let footer = GtkBox::new(Orientation::Horizontal, 8);
    footer.add_css_class("settings-footer");
    footer.append(&status);
    let reset = Button::with_label("Reset defaults");
    let discard = Button::with_label("Discard");
    let save = Button::with_label("Save changes");
    save.add_css_class("suggested-action");
    let quit = Button::with_label("Quit app");
    footer.append(&reset);
    footer.append(&discard);
    footer.append(&save);
    footer.append(&quit);
    shell.append(&footer);
    window.set_child(Some(&shell));

    // ----- Overview -----
    let (overview_scroll, overview) = settings_page(
        "Overview",
        "Runtime health, safe defaults, and a live visual preview.",
    );
    let preview_section = settings_section("Live preview", "The real topbar updates as you edit.");
    let preview = GtkBox::new(Orientation::Horizontal, 4);
    preview.add_css_class("topbar");
    preview.set_margin_top(8);
    preview.set_margin_bottom(8);
    for (id, text) in [
        ("clock", "12:34:56"),
        ("cpu", "CPU 24%"),
        ("memory", "RAM 6.2/16G"),
        ("network", "NET ↓1.4MB/s ↑82KB/s"),
    ] {
        let label = Label::new(Some(text));
        add_module_classes(&label, id);
        if id == "clock" {
            label.add_css_class("accent");
        }
        preview.append(&label);
    }
    preview_section.append(&preview);
    overview.append(&preview_section);

    let runtime_section = settings_section(
        "Runtime",
        "Fast telemetry, slow package checks, and direct actions use isolated workers.",
    );
    let version = Label::new(Some(&format!(
        "Version {} · config schema {}",
        env!("CARGO_PKG_VERSION"),
        cfg.borrow().schema_version
    )));
    version.set_xalign(0.0);
    runtime_section.append(&version);
    let backend = Label::new(Some(if is_wayland {
        "Display backend: Wayland layer-shell (when compositor supports it)"
    } else {
        "Display backend: X11 dock struts"
    }));
    backend.set_xalign(0.0);
    runtime_section.append(&backend);
    let runtime_state = Label::new(Some("Collecting redacted provider diagnostics…"));
    runtime_state.set_xalign(0.0);
    runtime_state.set_wrap(true);
    runtime_state.set_selectable(true);
    runtime_section.append(&runtime_state);
    let diagnostics_privacy = Label::new(Some(
        "Diagnostics intentionally omit SSIDs, media titles, usernames, paths, and command output.",
    ));
    diagnostics_privacy.add_css_class("settings-caption");
    diagnostics_privacy.set_xalign(0.0);
    diagnostics_privacy.set_wrap(true);
    runtime_section.append(&diagnostics_privacy);
    overview.append(&runtime_section);
    {
        let service = runtime.clone();
        let core_service = core.clone();
        let runtime_state = runtime_state.clone();
        let window2 = window.clone();
        glib::timeout_add_local(Duration::from_secs(1), move || {
            if !window2.is_visible() {
                return glib::ControlFlow::Break;
            }
            let external_snapshot = service.snapshot();
            let core_snapshot = core_service.snapshot();
            runtime_state.set_text(&format!(
                "{}\n{}",
                core_snapshot.sanitized_diagnostics(),
                external_snapshot.sanitized_diagnostics()
            ));
            glib::ControlFlow::Continue
        });
    }

    let behavior = settings_section("Behavior", "Accessibility and notification behavior.");
    let auto_dim = Switch::new();
    auto_dim.set_active(cfg.borrow().auto_hide);
    {
        let cfg = cfg.clone();
        let bar_window = bar_window.clone();
        auto_dim.connect_active_notify(move |widget| {
            cfg.borrow_mut().auto_hide = widget.is_active();
            if !widget.is_active() {
                bar_window.remove_css_class("dimmed");
            }
        });
    }
    labeled_row(&behavior, "Auto-dim when pointer leaves", &auto_dim);
    let notifications = Switch::new();
    notifications.set_active(cfg.borrow().notifications);
    {
        let cfg = cfg.clone();
        notifications.connect_active_notify(move |widget| {
            cfg.borrow_mut().notifications = widget.is_active();
        });
    }
    labeled_row(&behavior, "Critical notifications", &notifications);
    let reduce_motion = Switch::new();
    reduce_motion.set_active(cfg.borrow().reduce_motion);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        reduce_motion.connect_active_notify(move |widget| {
            cfg.borrow_mut().reduce_motion = widget.is_active();
            style();
        });
    }
    labeled_row(&behavior, "Reduce motion", &reduce_motion);
    let blink = Switch::new();
    blink.set_active(cfg.borrow().blink_critical);
    {
        let cfg = cfg.clone();
        blink.connect_active_notify(move |widget| {
            cfg.borrow_mut().blink_critical = widget.is_active();
        });
    }
    labeled_row(&behavior, "Blink critical values", &blink);
    overview.append(&behavior);
    stack.add_titled(&overview_scroll, Some("overview"), "Overview");

    // ----- Bars & layout -----
    let (bars_scroll, bars) = settings_page(
        "Bars & layout",
        "Control placement, density, monitor targeting, and quick actions.",
    );
    let placement = settings_section("Placement", "Changes apply without restarting.");
    let position = DropDown::from_strings(&["Top", "Bottom"]);
    position.set_selected(if cfg.borrow().position == "bottom" {
        1
    } else {
        0
    });
    {
        let cfg = cfg.clone();
        let geometry = actions.geometry.clone();
        position.connect_selected_notify(move |widget| {
            cfg.borrow_mut().position = if widget.selected() == 1 {
                "bottom"
            } else {
                "top"
            }
            .into();
            geometry();
        });
    }
    labeled_row(&placement, "Screen edge", &position);

    let mut monitor_labels = vec!["Default / primary".to_string()];
    if let Some(display) = gtk::gdk::Display::default() {
        let monitors = display.monitors();
        for index in 0..monitors.n_items() {
            if let Some(object) = monitors.item(index) {
                if let Ok(monitor) = object.downcast::<gtk::gdk::Monitor>() {
                    let geometry = monitor.geometry();
                    monitor_labels.push(format!(
                        "Monitor {index} · {}×{}",
                        geometry.width(),
                        geometry.height()
                    ));
                }
            }
        }
    }
    let monitor_refs: Vec<&str> = monitor_labels.iter().map(String::as_str).collect();
    let monitor = DropDown::from_strings(&monitor_refs);
    monitor.set_selected(if cfg.borrow().monitor < 0 {
        0
    } else {
        cfg.borrow().monitor as u32 + 1
    });
    {
        let cfg = cfg.clone();
        let geometry = actions.geometry.clone();
        monitor.connect_selected_notify(move |widget| {
            cfg.borrow_mut().monitor = if widget.selected() == 0 {
                -1
            } else {
                widget.selected() as i32 - 1
            };
            geometry();
        });
    }
    labeled_row(&placement, "Monitor", &monitor);
    let height = SpinButton::with_range(24.0, 64.0, 1.0);
    height.set_value(cfg.borrow().height as f64);
    {
        let cfg = cfg.clone();
        let geometry = actions.geometry.clone();
        height.connect_value_changed(move |widget| {
            cfg.borrow_mut().height = widget.value() as i32;
            geometry();
        });
    }
    labeled_row(&placement, "Bar height", &height);
    let offset = SpinButton::with_range(0.0, 500.0, 1.0);
    offset.set_value(cfg.borrow().margin_top as f64);
    {
        let cfg = cfg.clone();
        let geometry = actions.geometry.clone();
        offset.connect_value_changed(move |widget| {
            cfg.borrow_mut().margin_top = widget.value() as i32;
            geometry();
        });
    }
    labeled_row(&placement, "Edge offset", &offset);
    bars.append(&placement);

    let density = settings_section("Density", "Shared module spacing and refresh cadence.");
    let gap = SpinButton::with_range(0.0, 24.0, 1.0);
    gap.set_value(cfg.borrow().module_gap as f64);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        let layout = actions.layout.clone();
        gap.connect_value_changed(move |widget| {
            cfg.borrow_mut().module_gap = widget.value() as i32;
            style();
            layout();
        });
    }
    labeled_row(&density, "Gap", &gap);
    let padding = SpinButton::with_range(0.0, 18.0, 1.0);
    padding.set_value(cfg.borrow().module_padding as f64);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        padding.connect_value_changed(move |widget| {
            cfg.borrow_mut().module_padding = widget.value() as i32;
            style();
        });
    }
    labeled_row(&density, "Module padding", &padding);
    let radius = SpinButton::with_range(0.0, 18.0, 1.0);
    radius.set_value(cfg.borrow().module_radius as f64);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        radius.connect_value_changed(move |widget| {
            cfg.borrow_mut().module_radius = widget.value() as i32;
            style();
        });
    }
    labeled_row(&density, "Corner radius", &radius);
    let interval = SpinButton::with_range(250.0, 10_000.0, 250.0);
    interval.set_value(cfg.borrow().interval_ms as f64);
    {
        let cfg = cfg.clone();
        let apply_interval = actions.interval.clone();
        interval.connect_value_changed(move |widget| {
            cfg.borrow_mut().interval_ms = widget.value() as u64;
            apply_interval();
        });
    }
    labeled_row(&density, "Core refresh interval (ms)", &interval);
    let compact = Switch::new();
    compact.set_active(cfg.borrow().adaptive_compact);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        compact.connect_active_notify(move |widget| {
            cfg.borrow_mut().adaptive_compact = widget.is_active();
            layout();
        });
    }
    labeled_row(&density, "Adaptive layout tiers", &compact);
    let adaptive_note = Label::new(Some(
        "Normal → compact → tiny tiers are selected from the real monitor width. Low-priority modules move into the permanent More popover before Settings can be clipped.",
    ));
    adaptive_note.add_css_class("settings-caption");
    adaptive_note.set_xalign(0.0);
    adaptive_note.set_wrap(true);
    density.append(&adaptive_note);
    bars.append(&density);

    let quick = settings_section(
        "Quick actions",
        "Power requires a second click within four seconds.",
    );
    for (title, current, setter) in [
        ("Lock session", cfg.borrow().qa_lock, 0_u8),
        ("Screenshot", cfg.borrow().qa_screenshot, 1_u8),
        ("Power off", cfg.borrow().qa_shutdown, 2_u8),
    ] {
        let toggle = Switch::new();
        toggle.set_active(current);
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        toggle.connect_active_notify(move |widget| {
            match setter {
                0 => cfg.borrow_mut().qa_lock = widget.is_active(),
                1 => cfg.borrow_mut().qa_screenshot = widget.is_active(),
                _ => cfg.borrow_mut().qa_shutdown = widget.is_active(),
            }
            layout();
        });
        labeled_row(&quick, title, &toggle);
    }
    // ----- Module layout editor (Step 0.5) -----
    let layout_section = settings_section(
        "Module layout",
        "Enabled modules in bar order per zone. Arrows reorder within the same \
         zone, and the zone picker moves a module between Start/Center/End. The \
         tier preview runs the same allocator as the live bar at the simulated \
         width: Normal → Compact → Tiny as space shrinks, then → Move to More.",
    );
    let preview_row = GtkBox::new(Orientation::Horizontal, 8);
    let preview_caption = Label::new(Some("Simulated bar width (px)"));
    preview_caption.set_xalign(0.0);
    let width_spin = SpinButton::with_range(320.0, 7_680.0, 16.0);
    width_spin.set_value(1_366.0);
    let layout_list = GtkBox::new(Orientation::Vertical, 6);
    layout_list.add_css_class("module-layout-list");
    preview_row.append(&preview_caption);
    preview_row.append(&width_spin);
    layout_section.append(&preview_row);
    layout_section.append(&layout_list);
    let layout_note = Label::new(Some(
        "Modules you see here follow the same priority rule as the bar: within \
         each zone, equal-priority modules keep this manual order, while \
         different priorities still win/lose width first. Disabled modules are \
         hidden until enabled in the Modules catalog.",
    ));
    layout_note.add_css_class("settings-caption");
    layout_note.set_wrap(true);
    layout_note.set_xalign(0.0);
    layout_section.append(&layout_note);
    bars.append(&layout_section);

    {
        let cfg = cfg.clone();
        let actions = actions.clone();
        let list = layout_list.clone();
        width_spin.connect_value_changed(move |spin| {
            render_module_layout(&list, &cfg, &actions, spin.value() as i32);
        });
    }
    render_module_layout(&layout_list, cfg, actions, width_spin.value() as i32);

    bars.append(&quick);
    stack.add_titled(&bars_scroll, Some("bars"), "Bars & layout");

    // ----- Appearance -----
    let (appearance_scroll, appearance) = settings_page(
        "Appearance",
        "Use design controls for common changes; CSS remains an advanced override.",
    );
    let theme_section = settings_section("Theme", "Calm Telemetry uses semantic color tokens.");
    let theme = DropDown::from_strings(&["Dark", "Light"]);
    theme.set_selected(if cfg.borrow().theme == "light" { 1 } else { 0 });
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        theme.connect_selected_notify(move |widget| {
            cfg.borrow_mut().theme = if widget.selected() == 1 {
                "light"
            } else {
                "dark"
            }
            .into();
            style();
        });
    }
    labeled_row(&theme_section, "Color scheme", &theme);
    let font = Entry::new();
    font.set_text(&cfg.borrow().font_family);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        font.connect_changed(move |widget| {
            cfg.borrow_mut().font_family = widget.text().to_string();
            style();
        });
    }
    labeled_row(&theme_section, "Font family", &font);
    let font_size = SpinButton::with_range(8.0, 24.0, 1.0);
    font_size.set_value(cfg.borrow().font_size as f64);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        font_size.connect_value_changed(move |widget| {
            cfg.borrow_mut().font_size = widget.value() as i32;
            style();
        });
    }
    labeled_row(&theme_section, "Font size", &font_size);
    let opacity = SpinButton::with_range(0.35, 1.0, 0.05);
    opacity.set_digits(2);
    opacity.set_value(cfg.borrow().bar_opacity);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        opacity.connect_value_changed(move |widget| {
            cfg.borrow_mut().bar_opacity = widget.value();
            style();
        });
    }
    labeled_row(&theme_section, "Bar opacity", &opacity);
    let weight_labels = [
        "100 Thin",
        "200 ExtraLight",
        "300 Light",
        "400 Regular",
        "500 Medium",
        "600 SemiBold",
        "700 Bold",
        "800 ExtraBold",
        "900 Black",
    ];
    let font_weight = DropDown::from_strings(&weight_labels);
    // Borrow ends before the setter runs (same re-entrancy rule as colors).
    let weight_current = cfg.borrow().font_weight.clamp(100, 900);
    font_weight.set_selected(((weight_current / 100) - 1) as u32);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        font_weight.connect_selected_notify(move |widget| {
            cfg.borrow_mut().font_weight = 100 * (widget.selected() as i32 + 1);
            style();
        });
    }
    labeled_row(&theme_section, "Label font weight", &font_weight);
    let border_strength = SpinButton::with_range(0.0, 0.6, 0.02);
    border_strength.set_digits(2);
    let border_current = cfg.borrow().border_strength;
    border_strength.set_value(border_current);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        border_strength.connect_value_changed(move |widget| {
            cfg.borrow_mut().border_strength = widget.value();
            style();
        });
    }
    labeled_row(&theme_section, "Module border strength", &border_strength);
    appearance.append(&theme_section);

    let colors = settings_section("Colors", "CSS color values are validated before use.");
    // Read current values as separate statements: a temporary `cfg.borrow()`
    // inside an array literal stays alive until the whole statement ends, and
    // `picker.set_rgba()` below fires the notify handler synchronously, whose
    // `borrow_mut()` would then collide (RefCell already borrowed).
    let background_value = cfg.borrow().custom_bg.clone();
    let accent_value = cfg.borrow().accent_color.clone();
    let surface_value = cfg.borrow().surface_color.clone();
    let warning_value = cfg.borrow().warning_color.clone();
    let critical_value = cfg.borrow().critical_color.clone();
    for (title, value, field) in [
        ("Background", background_value, 0_u8),
        ("Accent", accent_value, 1_u8),
        ("Module surface", surface_value, 2_u8),
        ("Warning", warning_value, 3_u8),
        ("Critical", critical_value, 4_u8),
    ] {
        let entry = Entry::new();
        entry.set_text(&value);
        entry.set_placeholder_text(Some(if field == 0 {
            "empty = theme default"
        } else {
            "#RRGGBB or rgba(…)"
        }));
        // Native color picker next to the text entry (Step 0.4). The entry
        // stays as the advanced fallback for rgba() strings with alpha.
        let picker = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
        {
            let cfg = cfg.clone();
            let style = actions.style.clone();
            let entry = entry.clone();
            picker.connect_rgba_notify(move |button| {
                let rgba = button.rgba();
                let token = match rgba.alpha() {
                    alpha if (alpha - 1.0).abs() < f32::EPSILON => format!(
                        "rgb({:.0}, {:.0}, {:.0})",
                        rgba.red() * 255.0,
                        rgba.green() * 255.0,
                        rgba.blue() * 255.0,
                    ),
                    alpha => format!(
                        "rgba({:.0}, {:.0}, {:.0}, {alpha:.2})",
                        rgba.red() * 255.0,
                        rgba.green() * 255.0,
                        rgba.blue() * 255.0,
                    ),
                };
                match field {
                    0 => cfg.borrow_mut().custom_bg = token.clone(),
                    1 => cfg.borrow_mut().accent_color = token.clone(),
                    2 => cfg.borrow_mut().surface_color = token.clone(),
                    3 => cfg.borrow_mut().warning_color = token.clone(),
                    _ => cfg.borrow_mut().critical_color = token.clone(),
                }
                // Reuse the validated entry path so both stay in sync.
                entry.set_text(&token);
                style();
            });
        }
        if let Ok(rgba) = gtk::gdk::RGBA::parse(&value) {
            picker.set_rgba(&rgba);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        entry.set_hexpand(true);
        row.append(&entry);
        row.append(&picker);
        let cfg = cfg.clone();
        let style = actions.style.clone();
        entry.connect_changed(move |widget| {
            let value = widget.text().to_string();
            match field {
                0 => cfg.borrow_mut().custom_bg = value,
                1 => cfg.borrow_mut().accent_color = value,
                2 => cfg.borrow_mut().surface_color = value,
                3 => cfg.borrow_mut().warning_color = value,
                _ => cfg.borrow_mut().critical_color = value,
            }
            style();
        });
        labeled_row(&colors, title, &row);
    }
    appearance.append(&colors);

    let preset_section = settings_section(
        "Presets",
        "Presets now include spacing and semantic color tokens.",
    );
    let presets_snapshot = cfg.borrow().presets.clone();
    let preset_names: Vec<&str> = presets_snapshot
        .iter()
        .map(|preset| preset.name.as_str())
        .collect();
    let preset_picker = DropDown::from_strings(&preset_names);
    let apply_preset = Button::with_label("Apply selected preset");
    {
        let cfg = cfg.clone();
        let actions = actions.clone();
        let picker = preset_picker.clone();
        let presets = presets_snapshot.clone();
        apply_preset.connect_clicked(move |_| {
            let Some(preset) = presets.get(picker.selected() as usize) else {
                return;
            };
            let mut config = cfg.borrow_mut();
            config.theme = preset.theme.clone();
            config.custom_bg = preset.custom_bg.clone();
            config.custom_css = preset.custom_css.clone();
            config.font_family = preset.font_family.clone();
            config.font_size = preset.font_size;
            config.accent_color = preset.accent_color.clone();
            config.surface_color = preset.surface_color.clone();
            config.module_gap = preset.module_gap;
            config.module_padding = preset.module_padding;
            config.module_radius = preset.module_radius;
            config.font_weight = preset.font_weight;
            config.border_strength = preset.border_strength;
            drop(config);
            (actions.style)();
            (actions.layout)();
        });
    }
    labeled_row(&preset_section, "Preset", &preset_picker);
    preset_section.append(&apply_preset);
    let preset_name = Entry::new();
    preset_name.set_placeholder_text(Some("New preset name"));
    let save_preset = Button::with_label("Save current appearance as preset");
    {
        let cfg = cfg.clone();
        let preset_name = preset_name.clone();
        let status = status.clone();
        save_preset.connect_clicked(move |_| {
            let name = preset_name.text().trim().to_string();
            let name = if name.is_empty() {
                format!("Preset {}", cfg.borrow().presets.len() + 1)
            } else {
                name
            };
            let config = cfg.borrow();
            let preset = ThemePreset {
                name: name.clone(),
                theme: config.theme.clone(),
                custom_bg: config.custom_bg.clone(),
                custom_css: config.custom_css.clone(),
                font_family: config.font_family.clone(),
                font_size: config.font_size,
                accent_color: config.accent_color.clone(),
                surface_color: config.surface_color.clone(),
                module_gap: config.module_gap,
                module_padding: config.module_padding,
                module_radius: config.module_radius,
                font_weight: config.font_weight,
                border_strength: config.border_strength,
            };
            drop(config);
            cfg.borrow_mut().presets.push(preset);
            preset_name.set_text("");
            status.set_text(&format!(
                "Preset “{name}” added; reopen Settings to select it."
            ));
        });
    }
    preset_section.append(&preset_name);
    preset_section.append(&save_preset);
    appearance.append(&preset_section);

    // ----- Live preview (Step 0.4) -----
    // Rendered by the same global provider that styles the bar, so every
    // token above is reflected immediately without opening the bar window.
    let preview_section = settings_section(
        "Live preview",
        "Rendered with the same CSS provider as the live bar.",
    );
    let preview = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    preview.add_css_class("topbar");
    let preview_zone = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    preview_zone.add_css_class("zone");
    for (text, extra) in [("CPU 12%", "accent"), ("MEM 41%", "crit")] {
        let module = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        module.add_css_class("module");
        let label = Label::new(Some(text));
        label.add_css_class(extra);
        module.append(&label);
        preview_zone.append(&module);
    }
    preview.append(&preview_zone);
    preview_section.append(&preview);
    appearance.append(&preview_section);

    // ----- Undo & safe defaults (Step 0.4) -----
    // A snapshot of every visual token taken when Settings opened. Advanced
    // custom CSS is deliberately untouched by both actions.
    let visual_snapshot = cfg.borrow().clone();
    let undo_section = settings_section(
        "Undo & defaults",
        "Undo reverts the visual tokens below to the values from when Settings opened. Custom CSS is never modified.",
    );
    let undo_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let undo_button = Button::with_label("Undo since opening Settings");
    {
        let cfg = cfg.clone();
        let actions = actions.clone();
        let snapshot = visual_snapshot.clone();
        let slot = Rc::clone(&context.settings_slot);
        undo_button.connect_clicked(move |_| {
            {
                let mut config = cfg.borrow_mut();
                config.theme = snapshot.theme.clone();
                config.custom_bg = snapshot.custom_bg.clone();
                config.font_family = snapshot.font_family.clone();
                config.font_size = snapshot.font_size;
                config.bar_opacity = snapshot.bar_opacity;
                config.accent_color = snapshot.accent_color.clone();
                config.warning_color = snapshot.warning_color.clone();
                config.critical_color = snapshot.critical_color.clone();
                config.surface_color = snapshot.surface_color.clone();
                config.module_gap = snapshot.module_gap;
                config.module_padding = snapshot.module_padding;
                config.module_radius = snapshot.module_radius;
                config.font_weight = snapshot.font_weight;
                config.border_strength = snapshot.border_strength;
            }
            (actions.style)();
            (actions.layout)();
            if let Some(window) = slot.borrow().as_ref() {
                window.close();
            }
        });
    }
    undo_row.append(&undo_button);
    let restore_button = Button::with_label("Restore safe visual defaults");
    {
        let cfg = cfg.clone();
        let actions = actions.clone();
        let slot = Rc::clone(&context.settings_slot);
        restore_button.connect_clicked(move |_| {
            {
                let defaults = Config::default();
                let mut config = cfg.borrow_mut();
                config.theme = defaults.theme.clone();
                config.custom_bg = defaults.custom_bg.clone();
                config.font_family = defaults.font_family.clone();
                config.font_size = defaults.font_size;
                config.bar_opacity = defaults.bar_opacity;
                config.accent_color = defaults.accent_color.clone();
                config.warning_color = defaults.warning_color.clone();
                config.critical_color = defaults.critical_color.clone();
                config.surface_color = defaults.surface_color.clone();
                config.module_gap = defaults.module_gap;
                config.module_padding = defaults.module_padding;
                config.module_radius = defaults.module_radius;
                config.font_weight = defaults.font_weight;
                config.border_strength = defaults.border_strength;
            }
            (actions.style)();
            (actions.layout)();
            if let Some(window) = slot.borrow().as_ref() {
                window.close();
            }
        });
    }
    undo_row.append(&restore_button);
    undo_section.append(&undo_row);
    appearance.append(&undo_section);

    let css_section = settings_section(
        "Advanced CSS",
        "Loaded separately from the safe base theme. Target the .topbar selector.",
    );
    let css = TextView::new();
    css.set_monospace(true);
    css.buffer().set_text(&cfg.borrow().custom_css);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        css.buffer().connect_changed(move |buffer| {
            let (start, end) = buffer.bounds();
            cfg.borrow_mut().custom_css = buffer.text(&start, &end, false).to_string();
            style();
        });
    }
    css_section.append(
        &ScrolledWindow::builder()
            .child(&css)
            .min_content_height(130)
            .build(),
    );
    appearance.append(&css_section);
    stack.add_titled(&appearance_scroll, Some("appearance"), "Appearance");

    // ----- Searchable module catalog -----
    let module_catalog = build_module_catalog_page(
        cfg,
        actions,
        active,
        &status,
        &global_search,
        &global_search_status,
        &stack,
    );
    stack.add_titled(&module_catalog.root, Some("modules"), "Modules");

    // ----- Data & tools -----
    let (tools_scroll, tools) = settings_page(
        "Data & tools",
        "Select data sources, history behavior, updates, and configuration files.",
    );
    let sources = settings_section(
        "Sources",
        "Choose stable device identities where available.",
    );
    let mut interface_labels = vec!["All interfaces".to_string()];
    interface_labels.extend(core.snapshot().interface_names);
    let interface_refs: Vec<&str> = interface_labels.iter().map(String::as_str).collect();
    let interface = DropDown::from_strings(&interface_refs);
    let selected_interface = if cfg.borrow().net_iface.is_empty() {
        0
    } else {
        interface_labels
            .iter()
            .position(|name| name == &cfg.borrow().net_iface)
            .unwrap_or(0)
    };
    interface.set_selected(selected_interface as u32);
    {
        let cfg = cfg.clone();
        let names = interface_labels.clone();
        let layout = actions.layout.clone();
        interface.connect_selected_notify(move |widget| {
            let index = widget.selected() as usize;
            cfg.borrow_mut().net_iface = if index == 0 {
                String::new()
            } else {
                names.get(index).cloned().unwrap_or_default()
            };
            layout();
        });
    }
    labeled_row(&sources, "Network interface", &interface);
    let gpu = SpinButton::with_range(0.0, 31.0, 1.0);
    gpu.set_value(cfg.borrow().gpu_index as f64);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        gpu.connect_value_changed(move |widget| {
            cfg.borrow_mut().gpu_index = widget.value() as usize;
            layout();
        });
    }
    labeled_row(&sources, "GPU index", &gpu);
    tools.append(&sources);

    let history = settings_section(
        "History",
        "CSV remains compatible; writes happen off the UI thread.",
    );
    let history_enabled = Switch::new();
    history_enabled.set_active(cfg.borrow().history_enabled);
    {
        let cfg = cfg.clone();
        history_enabled.connect_active_notify(move |widget| {
            cfg.borrow_mut().history_enabled = widget.is_active();
        });
    }
    labeled_row(&history, "Record history", &history_enabled);
    let history_interval = SpinButton::with_range(5.0, 86_400.0, 5.0);
    history_interval.set_value(cfg.borrow().history_interval_secs as f64);
    {
        let cfg = cfg.clone();
        history_interval.connect_value_changed(move |widget| {
            cfg.borrow_mut().history_interval_secs = widget.value() as u64;
        });
    }
    labeled_row(&history, "Write interval (seconds)", &history_interval);
    let history_size = SpinButton::with_range(1.0, 1_024.0, 1.0);
    history_size.set_value(cfg.borrow().history_max_mb as f64);
    {
        let cfg = cfg.clone();
        history_size.connect_value_changed(move |widget| {
            cfg.borrow_mut().history_max_mb = widget.value() as u32;
        });
    }
    labeled_row(&history, "Rotate at size (MB)", &history_size);
    let history_actions = GtkBox::new(Orientation::Horizontal, 8);
    let dashboard = Button::with_label("View dashboard");
    {
        let service = runtime.clone();
        dashboard.connect_clicked(move |_| open_history_dashboard(service.clone()));
    }
    let open_history = Button::with_label("Open history folder");
    {
        let service = runtime.clone();
        open_history.connect_clicked(move |_| {
            if let Some(directory) = history_log_path().parent() {
                service.open_path(directory.to_path_buf());
            }
        });
    }
    let export_history = Button::with_label("Export CSV");
    {
        let settings_window = window.clone();
        let status = status.clone();
        export_history.connect_clicked(move |_| {
            let dialog = FileDialog::builder()
                .title("Export TopMonitoring history")
                .initial_name("topmonitoring-history.csv")
                .build();
            let status = status.clone();
            dialog.save(
                Some(&settings_window),
                gtk::gio::Cancellable::NONE,
                move |result| {
                    let destination = result.map_err(|error| error.to_string()).and_then(|file| {
                        file.path()
                            .ok_or_else(|| "No local path selected".to_string())
                    });
                    match destination.and_then(|destination| {
                        std::fs::copy(history_log_path(), destination)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    }) {
                        Ok(_) => status.set_text("History exported."),
                        Err(error) => status.set_text(&format!("History export failed: {error}")),
                    }
                },
            );
        });
    }
    history_actions.append(&dashboard);
    history_actions.append(&open_history);
    history_actions.append(&export_history);
    history.append(&history_actions);
    tools.append(&history);

    let diagnostics = settings_section(
        "Diagnostics",
        "Detailed views run independently from the bar renderer.",
    );
    let process_manager = Button::with_label("Open Process Manager");
    process_manager.connect_clicked(|_| open_process_manager());
    let sensors = Button::with_label("Open Hardware Sensors");
    sensors.connect_clicked(|_| open_sensors());
    diagnostics.append(&process_manager);
    diagnostics.append(&sensors);
    tools.append(&diagnostics);

    let updates = settings_section(
        "Updates",
        "Checks GitHub releases; never installs with root privileges.",
    );
    let auto_updates = Switch::new();
    auto_updates.set_active(cfg.borrow().check_updates);
    {
        let cfg = cfg.clone();
        auto_updates.connect_active_notify(move |widget| {
            cfg.borrow_mut().check_updates = widget.is_active();
        });
    }
    labeled_row(&updates, "Check once at startup", &auto_updates);
    let update_row = GtkBox::new(Orientation::Horizontal, 8);
    let check_update = Button::with_label("Check now");
    let open_release = Button::with_label("Open releases");
    let update_result = Label::new(None);
    update_result.set_wrap(true);
    {
        let service = runtime.clone();
        check_update.connect_clicked(move |_| service.check_updates());
    }
    {
        let service = runtime.clone();
        open_release.connect_clicked(move |_| {
            service.open_path(PathBuf::from(runtime::release_page()));
        });
    }
    {
        let service = runtime.clone();
        let result = update_result.clone();
        let window2 = window.clone();
        glib::timeout_add_local(Duration::from_millis(500), move || {
            if !window2.is_visible() {
                return glib::ControlFlow::Break;
            }
            if let Some(message) = service.snapshot().update_status {
                result.set_text(&message);
            }
            glib::ControlFlow::Continue
        });
    }
    update_row.append(&check_update);
    update_row.append(&open_release);
    update_row.append(&update_result);
    updates.append(&update_row);
    tools.append(&updates);

    let files = settings_section(
        "Configuration",
        "Imported executable behavior is disabled until trusted.",
    );
    let file_row = GtkBox::new(Orientation::Horizontal, 8);
    let export = Button::with_label("Export configuration");
    let import = Button::with_label("Import safely");
    let autostart = Button::with_label("Enable autostart");
    file_row.append(&export);
    file_row.append(&import);
    file_row.append(&autostart);
    files.append(&file_row);
    {
        let cfg = cfg.clone();
        let window2 = window.clone();
        let status = status.clone();
        export.connect_clicked(move |_| {
            let dialog = FileDialog::builder()
                .title("Export TopMonitoring configuration")
                .initial_name("topmonitoring-v2.toml")
                .build();
            let cfg = cfg.clone();
            let status = status.clone();
            dialog.save(
                Some(&window2),
                gtk::gio::Cancellable::NONE,
                move |result| match result
                    .map_err(|error| error.to_string())
                    .and_then(|file| file.path().ok_or_else(|| "No local path selected".into()))
                    .and_then(|path| {
                        cfg.borrow().to_toml().and_then(|raw| {
                            std::fs::write(&path, raw).map_err(|error| error.to_string())
                        })
                    }) {
                    Ok(_) => status.set_text("Configuration exported."),
                    Err(error) => status.set_text(&format!("Export failed: {error}")),
                },
            );
        });
    }
    {
        let cfg = cfg.clone();
        let original = original.clone();
        let actions = actions.clone();
        let window2 = window.clone();
        let status = status.clone();
        import.connect_clicked(move |_| {
            let dialog = FileDialog::builder().title("Import TopMonitoring configuration").build();
            let cfg = cfg.clone();
            let original = original.clone();
            let actions = actions.clone();
            let window3 = window2.clone();
            let status = status.clone();
            dialog.open(Some(&window2), gtk::gio::Cancellable::NONE, move |result| {
                let imported = result
                    .map_err(|error| error.to_string())
                    .and_then(|file| file.path().ok_or_else(|| "No local path selected".into()))
                    .and_then(|path| read_import(&path));
                match imported {
                    Ok(imported) => {
                        *cfg.borrow_mut() = imported;
                        match cfg.borrow().save() {
                            Ok(_) => {
                                *original.borrow_mut() = cfg.borrow().clone();
                                (actions.all)();
                                status.set_text("Imported safely. External commands are disabled until trusted.");
                                window3.close();
                            }
                            Err(error) => status.set_text(&format!("Import could not be saved: {error}")),
                        }
                    }
                    Err(error) => status.set_text(&format!("Import failed: {error}")),
                }
            });
        });
    }
    {
        let status = status.clone();
        autostart.connect_clicked(move |_| {
            status.set_text(if install_autostart() {
                "Autostart enabled."
            } else {
                "Could not enable autostart."
            });
        });
    }
    tools.append(&files);
    stack.add_titled(&tools_scroll, Some("tools"), "Data & tools");

    // Responsive navigation and keyboard search flow.
    {
        let stack = stack.clone();
        compact_navigation.connect_selected_notify(move |widget| {
            let page = match widget.selected() {
                0 => "overview",
                1 => "bars",
                2 => "appearance",
                3 => "modules",
                _ => "tools",
            };
            stack.set_visible_child_name(page);
        });
    }
    {
        let navigation = compact_navigation.clone();
        stack.connect_visible_child_name_notify(move |widget| {
            let visible_name = widget.visible_child_name().map(|name| name.to_string());
            let selected = match visible_name.as_deref() {
                Some("bars") => 1,
                Some("appearance") => 2,
                Some("modules") => 3,
                Some("tools") => 4,
                _ => 0,
            };
            if navigation.selected() != selected {
                navigation.set_selected(selected);
            }
        });
    }
    {
        let global_search = global_search.clone();
        let keyboard = EventControllerKey::new();
        keyboard.connect_key_pressed(move |_, key, _, modifiers| {
            if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) && key == gtk::gdk::Key::f {
                global_search.grab_focus();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Escape && !global_search.text().is_empty() {
                global_search.set_text("");
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keyboard);
    }
    {
        let window2 = window.clone();
        let sidebar = sidebar.clone();
        let navigation = compact_navigation.clone();
        let catalog_paned = module_catalog.paned.clone();
        let responsive_state = Rc::new(Cell::new(None::<(bool, bool)>));
        glib::timeout_add_local(Duration::from_millis(250), move || {
            if !window2.is_visible() {
                return glib::ControlFlow::Break;
            }
            let width = window2.width();
            let compact_shell = width < 760;
            let stacked_catalog = width < 900;
            let state = (compact_shell, stacked_catalog);
            if responsive_state.replace(Some(state)) != Some(state) {
                sidebar.set_visible(!compact_shell);
                navigation.set_visible(compact_shell);
                catalog_paned.set_orientation(if stacked_catalog {
                    Orientation::Vertical
                } else {
                    Orientation::Horizontal
                });
                catalog_paned.set_position(if stacked_catalog { 240 } else { 310 });
            }
            glib::ControlFlow::Continue
        });
    }

    // Footer actions and transactional close behavior.
    {
        let cfg = cfg.clone();
        let original = original.clone();
        let status = status.clone();
        let actions = actions.clone();
        save.connect_clicked(move |_| {
            cfg.borrow_mut().migrate_and_validate();
            match cfg.borrow().save() {
                Ok(_) => {
                    *original.borrow_mut() = cfg.borrow().clone();
                    (actions.all)();
                    status.set_text("Saved successfully.");
                }
                Err(error) => status.set_text(&format!("Save failed: {error}")),
            }
        });
    }
    {
        let window2 = window.clone();
        discard.connect_clicked(move |_| window2.close());
    }
    {
        let cfg = cfg.clone();
        let original = original.clone();
        let actions = actions.clone();
        let window2 = window.clone();
        let status = status.clone();
        reset.connect_clicked(move |_| {
            *cfg.borrow_mut() = Config::default();
            match cfg.borrow().save() {
                Ok(_) => {
                    *original.borrow_mut() = cfg.borrow().clone();
                    (actions.all)();
                    window2.close();
                }
                Err(error) => status.set_text(&format!("Reset failed: {error}")),
            }
        });
    }
    {
        let bar_window = bar_window.clone();
        quit.connect_clicked(move |_| {
            if let Some(application) = bar_window.application() {
                application.quit();
            }
        });
    }
    {
        let cfg = cfg.clone();
        let original = original.clone();
        let actions = actions.clone();
        let settings_slot = settings_slot.clone();
        let catalog_refresh_slots = module_catalog.refresh_slots.clone();
        window.connect_close_request(move |_| {
            *cfg.borrow_mut() = original.borrow().clone();
            (actions.all)();
            for refresh_slot in &catalog_refresh_slots {
                *refresh_slot.borrow_mut() = None;
            }
            *settings_slot.borrow_mut() = None;
            glib::Propagation::Proceed
        });
    }

    window.present();
}

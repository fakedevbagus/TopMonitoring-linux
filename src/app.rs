#![allow(unused, dead_code)]
use crate::{config, effects};

use crate::backend::{
    CoreCollectionConfig, CoreCollector, CoreSnapshot, MetricLevel, MetricSample,
};
use crate::command::{run_program_limited, run_shell_limited, CommandPolicy, CommandResult};
use crate::config::{
    build_css, build_custom_css, css_class_id, default_prefix, read_import, Config, CustomModule,
    ThemePreset,
};
#[cfg(feature = "wayland")]
use crate::docking::configure_wayland;
use crate::docking::{configure_x11, monitor_geometry};
use crate::fsio::human_rate;
use crate::history::{append_history_values_async, history_log_path};
use crate::layout::{
    allocate_layout, custom_width_profile, module_width_profile, LayoutItem, LayoutTier,
    WidthProfile,
};
use crate::model::{module_descriptor, module_registry, module_search_text};
use crate::ui::bar::{
    add_module_classes, apply_adaptive_layout, connect_action_feedback, external_metric_changed,
    make_action_toast, make_adaptive_entry, module_width_budget, rebuild_bar_metrics,
    update_metric, update_slot, ActionToast, Active, AdaptiveEntries, ApplyActions, BarZones,
    OverflowUi, RefreshSlot, SettingsContext, Slot, SlotWidget,
};
use crate::ui::settings::catalog::{
    build_module_catalog_page, module_catalog_matches, module_catalog_record, CatalogKind,
    ModuleCatalogKey, ModuleCatalogRecord,
};
use crate::ui::settings::layout::{
    collect_layout_rows, labeled_row, layout_preview_items, layout_row_swap, render_module_layout,
    settings_page, settings_section, zone_dropdown, zone_name, zone_tier_summary, LayoutRow,
    LayoutRowTarget,
};
use crate::ui::settings::open_settings;
use crate::ui::settings::search::{
    request_custom_module_run, search_text_matches, settings_page_search_target,
    unique_custom_module_name,
};
use crate::ui::{
    make_range, open_history_dashboard, open_hwmon_picker, open_process_manager, open_sensors,
};
use gdk4_x11::prelude::*;
use gtk::gio::prelude::FileExt;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, CenterBox, CssProvider, DrawingArea,
    DropDown, Entry, EventControllerKey, EventControllerMotion, FileDialog, GestureClick, Label,
    ListBox, ListBoxRow, Orientation, Paned, Scale, ScrolledWindow, SearchEntry, Separator,
    SpinButton, Stack, StackSidebar, Switch, TextView, Window,
};
#[cfg(feature = "wayland")]
use gtk4_layer_shell::LayerShell;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::runtime::{ExternalSnapshot, QuickAction, RuntimeServices};

pub fn run() {
    let app = Application::builder()
        .application_id("io.github.fakedevbagus.TopMonitoring")
        .build();

    // TopMonitoring is single-instance: launching it again while it is
    // already running just re-activates the existing bar instead of
    // creating a second, overlapping one.
    let built = Rc::new(Cell::new(false));
    app.connect_activate(move |app| {
        if built.get() {
            return;
        }
        built.set(true);
        build_bar(app);
    });
    app.run();
}

fn build_bar(app: &Application) {
    let cfg = Rc::new(RefCell::new(Config::load()));
    let runtime = RuntimeServices::start(cfg.borrow().check_updates);
    let core = {
        let config = cfg.borrow();
        CoreCollector::start(CoreCollectionConfig::new(
            config.interval_ms,
            config.net_iface.clone(),
            config.gpu_index as u32,
        ))
    };
    let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok()
        && std::env::var("GDK_BACKEND")
            .map(|backend| backend != "x11")
            .unwrap_or(true);

    // Safe base CSS and user CSS live in separate providers. A malformed user
    // override can no longer remove the readable base theme.
    let provider = Rc::new(CssProvider::new());
    let custom_provider = Rc::new(CssProvider::new());
    provider.load_from_string(&build_css(&cfg.borrow(), None));
    custom_provider.load_from_string(&build_custom_css(&cfg.borrow()));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &*provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        gtk::style_context_add_provider_for_display(
            &display,
            &*custom_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .title("TopMonitoring")
        .build();
    window.add_css_class("topbar");
    window.set_default_height(cfg.borrow().height);

    if is_wayland {
        #[cfg(feature = "wayland")]
        {
            window.init_layer_shell();
            configure_wayland(&window, &cfg.borrow());
        }
        #[cfg(not(feature = "wayland"))]
        {
            eprintln!(
                "[TopMonitoring] Wayland detected, but this build has no layer-shell support. \
                 The bar will use an explicit floating fallback and cannot reserve screen space."
            );
            window.set_decorated(false);
            window.set_resizable(false);
        }
    } else {
        window.set_decorated(false);
        window.set_resizable(false);
        let cfg2 = cfg.clone();
        window.connect_realize(move |widget| configure_x11(widget, &cfg2.borrow()));
    }

    // Auto-dim uses one shared cancellable timer. Re-entering the bar always
    // cancels a pending fade, fixing the v1 leave/enter race.
    {
        let motion = EventControllerMotion::new();
        let dim_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let win_enter = window.clone();
        let enter_timer = dim_timer.clone();
        motion.connect_enter(move |_, _, _| {
            if let Some(source) = enter_timer.borrow_mut().take() {
                source.remove();
            }
            win_enter.remove_css_class("dimmed");
        });
        let win_leave = window.clone();
        let cfg_leave = cfg.clone();
        let leave_timer = dim_timer.clone();
        motion.connect_leave(move |_| {
            if let Some(source) = leave_timer.borrow_mut().take() {
                source.remove();
            }
            if !cfg_leave.borrow().auto_hide {
                win_leave.remove_css_class("dimmed");
                return;
            }
            let win2 = win_leave.clone();
            let cfg2 = cfg_leave.clone();
            let timer2 = leave_timer.clone();
            let source = glib::timeout_add_local(Duration::from_millis(1_200), move || {
                if cfg2.borrow().auto_hide {
                    win2.add_css_class("dimmed");
                }
                *timer2.borrow_mut() = None;
                glib::ControlFlow::Break
            });
            *leave_timer.borrow_mut() = Some(source);
        });
        window.add_controller(motion);
    }

    // V2 layout: independent start/center/end zones. The settings button stays
    // outside user modules so it remains reachable even with a dense layout.
    let root = CenterBox::new();
    let zones = BarZones::new(cfg.borrow().module_gap);
    zones.start.set_halign(gtk::Align::Start);
    zones.center.set_halign(gtk::Align::Center);
    zones.end.set_halign(gtk::Align::End);
    root.set_start_widget(Some(&zones.start));
    root.set_center_widget(Some(&zones.center));
    let end_shell = GtkBox::new(Orientation::Horizontal, 4);
    end_shell.append(&zones.end);
    let overflow_ui = OverflowUi::new();
    end_shell.append(&overflow_ui.button);
    end_shell.append(&Separator::new(Orientation::Vertical));
    let settings_btn = Button::with_label("⚙");
    settings_btn.set_tooltip_text(Some("Open TopMonitoring settings"));
    settings_btn.set_accessible_role(gtk::AccessibleRole::Button);
    settings_btn.set_margin_end(6);
    end_shell.append(&settings_btn);
    root.set_end_widget(Some(&end_shell));
    // The bar content lives in an overlay so the non-interactive effect layer
    // can paint above every module without intercepting pointer input.
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&root));
    let effect_engine = effects::EffectEngine::attach(
        &overlay,
        effects::EffectParams::from_config(&cfg.borrow()),
        cfg.borrow().reduce_motion,
    );
    window.set_child(Some(&overlay));

    // V3-009: a transient toast layer for action feedback (pending → success /
    // failure with backend name and error detail). Non-interactive label that
    // auto-hides, so it never steals keyboard or pointer focus.
    let action_toast = make_action_toast(&overlay);

    let active: Active = Rc::new(RefCell::new(Vec::new()));
    let adaptive_entries: AdaptiveEntries = Rc::new(RefCell::new(Vec::new()));
    rebuild_bar_metrics(
        &zones,
        &active,
        &adaptive_entries,
        &cfg.borrow(),
        &runtime,
        &action_toast,
    );
    let initial_layout_width = monitor_geometry(cfg.borrow().monitor)
        .map(|(_, _, width, _)| width)
        .unwrap_or(1_366);
    let layout_width = Rc::new(Cell::new(initial_layout_width));
    apply_adaptive_layout(
        &adaptive_entries,
        &overflow_ui,
        initial_layout_width,
        &cfg.borrow(),
    );

    // GTK only reconciles immutable snapshots. Native hardware/filesystem I/O
    // is owned by CoreCollector; subprocess providers use RuntimeServices.
    let poll_source: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let history_accum_ms = Rc::new(Cell::new(0_u64));
    let last_core_snapshot: Rc<RefCell<Option<CoreSnapshot>>> = Rc::new(RefCell::new(None));
    let last_external_snapshot: Rc<RefCell<Option<ExternalSnapshot>>> = Rc::new(RefCell::new(None));

    let respawn: Rc<dyn Fn(u64)> = {
        let (active, cfg, poll_source, core) = (
            active.clone(),
            cfg.clone(),
            poll_source.clone(),
            core.clone(),
        );
        let effect_engine = effect_engine.clone();
        let history_accum_ms = history_accum_ms.clone();
        let last_core_snapshot = last_core_snapshot.clone();
        let last_external_snapshot = last_external_snapshot.clone();
        let runtime = runtime.clone();
        Rc::new(move |interval_ms: u64| {
            if let Some(old) = poll_source.borrow_mut().take() {
                old.remove();
            }
            {
                let config = cfg.borrow();
                core.configure(CoreCollectionConfig::new(
                    interval_ms,
                    config.net_iface.clone(),
                    config.gpu_index as u32,
                ));
            }
            let (active, cfg, core) = (active.clone(), cfg.clone(), core.clone());
            let effect_engine = effect_engine.clone();
            let history_accum_ms = history_accum_ms.clone();
            let last_core_snapshot = last_core_snapshot.clone();
            let last_external_snapshot = last_external_snapshot.clone();
            let runtime = runtime.clone();
            let step_ms = interval_ms.clamp(250, 10_000);
            let source = glib::timeout_add_local(Duration::from_millis(step_ms), move || {
                let core_snapshot = core.snapshot();
                let external = runtime.snapshot();
                let config = cfg.borrow();
                {
                    let previous_core = last_core_snapshot.borrow();
                    let previous_external = last_external_snapshot.borrow();
                    let core_generation_changed = previous_core
                        .as_ref()
                        .map(|previous| previous.generation != core_snapshot.generation)
                        .unwrap_or(true);
                    let external_generation_changed = previous_external
                        .as_ref()
                        .map(|previous| previous.generation != external.generation)
                        .unwrap_or(true);
                    for slot in active.borrow().iter() {
                        let layout_changed = slot.layout.dirty.replace(false);
                        let core_changed = core_generation_changed
                            && if matches!(
                                slot.id.as_str(),
                                "cpu_graph" | "ram_graph" | "net_graph"
                            ) {
                                core_snapshot.graph_changed(&slot.id, previous_core.as_ref())
                            } else {
                                core_snapshot.metric_changed(&slot.id, previous_core.as_ref())
                            };
                        let external_changed = external_generation_changed
                            && external_metric_changed(
                                &slot.id,
                                &external,
                                previous_external.as_ref(),
                            );
                        update_slot(
                            slot,
                            &core_snapshot,
                            &external,
                            config.notifications,
                            core_changed,
                            external_changed,
                            layout_changed,
                        );
                    }
                }
                if config.history_enabled && core_snapshot.generation > 0 {
                    let accumulated = history_accum_ms.get().saturating_add(step_ms);
                    let target = config
                        .history_interval_secs
                        .saturating_mul(1_000)
                        .max(1_000);
                    if accumulated >= target {
                        append_history_values_async(
                            core_snapshot.cpu_percent,
                            core_snapshot.memory_percent,
                            core_snapshot.temperature_celsius,
                            config.history_max_mb,
                        );
                        history_accum_ms.set(0);
                    } else {
                        history_accum_ms.set(accumulated);
                    }
                } else {
                    history_accum_ms.set(0);
                }
                // Keep the effect engine in sync with the live config. The
                // engine itself never rebuilds CSS; it only redraws its own
                // overlay layer through the frame clock.
                effect_engine.update_params(
                    effects::EffectParams::from_config(&config),
                    config.reduce_motion,
                );
                drop(config);
                *last_core_snapshot.borrow_mut() = Some(core_snapshot);
                *last_external_snapshot.borrow_mut() = Some(external);
                glib::ControlFlow::Continue
            });
            *poll_source.borrow_mut() = Some(source);
        })
    };
    respawn(cfg.borrow().interval_ms);

    // GTK does not expose provider work here; this cadence only responds to an
    // actual allocation-width change and applies the pure layout decisions.
    {
        let window = window.clone();
        let cfg = cfg.clone();
        let adaptive_entries = adaptive_entries.clone();
        let overflow_ui = overflow_ui.clone();
        let layout_width = layout_width.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let width = window.width();
            if width > 1 && width != layout_width.get() {
                layout_width.set(width);
                apply_adaptive_layout(&adaptive_entries, &overflow_ui, width, &cfg.borrow());
            }
            glib::ControlFlow::Continue
        });
    }

    // Critical animation is optional and automatically disabled with reduced
    // motion. State remains distinguishable through color and tooltip text.
    {
        let active = active.clone();
        let cfg = cfg.clone();
        let phase = Rc::new(Cell::new(false));
        glib::timeout_add_local(Duration::from_millis(500), move || {
            let animate = cfg.borrow().blink_critical && !cfg.borrow().reduce_motion;
            let off = phase.get();
            phase.set(!off);
            for slot in active.borrow().iter() {
                if let SlotWidget::Text(label) = &slot.widget {
                    if animate && label.has_css_class("crit") && off {
                        label.add_css_class("blink-off");
                    } else {
                        label.remove_css_class("blink-off");
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    let apply_style: Rc<dyn Fn()> = {
        let (cfg, provider, custom_provider) =
            (cfg.clone(), provider.clone(), custom_provider.clone());
        Rc::new(move || {
            let config = cfg.borrow();
            provider.load_from_string(&build_css(&config, None));
            custom_provider.load_from_string(&build_custom_css(&config));
        })
    };
    let apply_layout: Rc<dyn Fn()> = {
        let (
            cfg,
            zones,
            active,
            adaptive_entries,
            overflow_ui,
            layout_width,
            runtime,
            core,
            last_core,
            last_external,
            action_toast,
        ) = (
            cfg.clone(),
            zones.clone(),
            active.clone(),
            adaptive_entries.clone(),
            overflow_ui.clone(),
            layout_width.clone(),
            runtime.clone(),
            core.clone(),
            last_core_snapshot.clone(),
            last_external_snapshot.clone(),
            action_toast.clone(),
        );
        Rc::new(move || {
            let config = cfg.borrow();
            core.configure(CoreCollectionConfig::new(
                config.interval_ms,
                config.net_iface.clone(),
                config.gpu_index as u32,
            ));
            rebuild_bar_metrics(
                &zones,
                &active,
                &adaptive_entries,
                &config,
                &runtime,
                &action_toast,
            );
            apply_adaptive_layout(&adaptive_entries, &overflow_ui, layout_width.get(), &config);
            *last_core.borrow_mut() = None;
            *last_external.borrow_mut() = None;
        })
    };
    let apply_geometry: Rc<dyn Fn()> = {
        let (cfg, window) = (cfg.clone(), window.clone());
        Rc::new(move || {
            let config = cfg.borrow();
            window.set_default_height(config.height);
            if is_wayland {
                #[cfg(feature = "wayland")]
                configure_wayland(&window, &config);
            } else {
                configure_x11(&window, &config);
            }
        })
    };
    let apply_interval: Rc<dyn Fn()> = {
        let (cfg, respawn) = (cfg.clone(), respawn.clone());
        Rc::new(move || respawn(cfg.borrow().interval_ms))
    };
    let apply_all: Rc<dyn Fn()> = {
        let (style, layout, geometry, interval) = (
            apply_style.clone(),
            apply_layout.clone(),
            apply_geometry.clone(),
            apply_interval.clone(),
        );
        Rc::new(move || {
            style();
            layout();
            geometry();
            interval();
        })
    };
    let actions = ApplyActions {
        style: apply_style,
        layout: apply_layout,
        geometry: apply_geometry,
        interval: apply_interval,
        all: apply_all,
    };

    let settings_slot: Rc<RefCell<Option<Window>>> = Rc::new(RefCell::new(None));
    let settings_context = SettingsContext {
        bar_window: window.clone(),
        active: active.clone(),
        runtime: runtime.clone(),
        core: core.clone(),
        actions: actions.clone(),
        settings_slot,
    };
    let open = {
        let cfg = cfg.clone();
        let settings_context = settings_context.clone();
        Rc::new(move || open_settings(&cfg, is_wayland, &settings_context))
    };
    {
        let open = open.clone();
        settings_btn.connect_clicked(move |_| open());
    }
    {
        let gesture = GestureClick::new();
        gesture.set_button(3);
        let open = open.clone();
        gesture.connect_pressed(move |_, _, _, _| open());
        window.add_controller(gesture);
    }

    // SIGUSR1 is 10 on the supported Linux architectures in the release
    // matrix. The CLI/DBus control surface planned for a later minor release
    // can replace this compatibility shortcut.
    {
        let window = window.clone();
        glib::source::unix_signal_add_local(10, move || {
            window.set_visible(!window.is_visible());
            glib::ControlFlow::Continue
        });
    }

    window.present();
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use crate::docking::calculate_x11_dock_geometry;
    use crate::fsio::{format_disk_bar, format_disk_compact, format_disk_tiny, DiskSummary};
    use crate::history::summarize_history;

    #[test]
    fn history_summary_ignores_invalid_rows_and_calculates_stats() {
        let raw = "timestamp,cpu_percent,mem_percent,temp_c\n2026-08-14 10:00:00,10,20,40\ninvalid\n2026-08-14 10:01:00,30,40,60\n";
        let summary = summarize_history(raw);
        assert_eq!(summary.sample_count, 2);
        assert_eq!(summary.cpu_average, 20.0);
        assert_eq!(summary.memory_maximum, 40.0);
        assert_eq!(summary.temperature_maximum, 60.0);
    }

    #[test]
    fn x11_geometry_clamps_negative_monitor_coordinates() {
        let geometry =
            calculate_x11_dock_geometry(3840, 1080, Some((-1920, -20, 1920, 1080)), 0, 32, false)
                .expect("valid geometry");
        assert_eq!(geometry.x, 0);
        assert!(geometry.strut_partial.iter().all(|value| *value <= 3840));
        assert!(geometry.strut_partial[9] < 3840);
    }

    #[test]
    fn disk_bar_keeps_root_and_fullest_secondary_volume() {
        let entry = |name: &str, mount: &str, used_percent: f64| DiskSummary {
            name: name.into(),
            mount: mount.into(),
            file_system: "ext4".into(),
            used_percent,
            available_gb: 10.0,
            total_gb: 100.0,
        };
        let entries = vec![
            entry("root", "/", 21.0),
            entry("ARCHIVE", "/media/archive", 44.0),
            entry("WD BLUE BACKUP DRIVE", "/media/backup", 83.0),
        ];
        assert_eq!(format_disk_bar(&entries), "root 21% · WD BLUE… 83%");
        assert_eq!(format_disk_compact(&entries), "WD BLUE BAC… 83%");
        assert_eq!(format_disk_tiny(&entries), "83%");
    }

    #[test]
    fn module_budget_reserves_permanent_and_quick_controls() {
        let mut config = Config::default();
        assert_eq!(module_width_budget(1_366, &config), 1_250);
        config.qa_lock = true;
        config.qa_screenshot = true;
        config.qa_shutdown = true;
        assert_eq!(module_width_budget(1_366, &config), 1_016);
    }

    #[test]
    fn module_search_uses_storage_aliases_and_multiple_terms() {
        let search_text = module_search_text("disk", "DISK", "DISK");
        assert!(search_text_matches(&search_text, "hdd storage"));
        assert!(!search_text_matches(&search_text, "wifi"));
    }

    #[test]
    fn catalog_filter_matches_alias_and_kind() {
        let record = ModuleCatalogRecord {
            title: "Disk usage".into(),
            subtitle: "Built-in · Storage".into(),
            search_text: module_search_text("disk", "DISK", "DISK"),
            category: "Storage".into(),
            kind: CatalogKind::BuiltIn,
            enabled: true,
        };
        assert!(module_catalog_matches(&record, "hdd storage", 0, 1, None));
        assert!(!module_catalog_matches(&record, "hdd", 0, 2, None));
    }

    #[test]
    fn catalog_filter_combines_state_and_category() {
        let record = ModuleCatalogRecord {
            title: "Temperature".into(),
            subtitle: "Built-in · Thermal".into(),
            search_text: "temperature thermal sensor".into(),
            category: "Thermal".into(),
            kind: CatalogKind::BuiltIn,
            enabled: false,
        };
        assert!(module_catalog_matches(
            &record,
            "thermal",
            2,
            0,
            Some("Thermal")
        ));
        assert!(!module_catalog_matches(
            &record,
            "thermal",
            1,
            0,
            Some("Thermal")
        ));
        assert!(!module_catalog_matches(
            &record,
            "thermal",
            2,
            0,
            Some("Storage")
        ));
    }

    #[test]
    fn global_settings_search_routes_common_controls() {
        assert_eq!(
            settings_page_search_target("runtime provider"),
            Some("overview")
        );
        assert_eq!(settings_page_search_target("monitor edge"), Some("bars"));
        assert_eq!(
            settings_page_search_target("theme color"),
            Some("appearance")
        );
        assert_eq!(settings_page_search_target("history csv"), Some("tools"));
    }

    #[test]
    fn global_settings_search_rejects_unrelated_terms() {
        assert_eq!(
            settings_page_search_target("nonexistent quantum banana"),
            None
        );
    }

    #[test]
    fn custom_catalog_record_uses_live_configuration() {
        let mut config = Config::default();
        let custom = CustomModule {
            name: "Build queue".into(),
            label: "CI".into(),
            command: "printf ready".into(),
            enabled: true,
            ..CustomModule::default()
        };
        config.custom_modules.push(custom);
        let record = module_catalog_record(&config, &ModuleCatalogKey::Custom(0))
            .expect("custom catalog record");
        assert_eq!(record.title, "Build queue");
        assert!(record.enabled);
        assert!(record.search_text.contains("printf ready"));
    }

    #[test]
    fn external_diff_ignores_health_only_generation_changes() {
        let previous = ExternalSnapshot::default();
        let mut current = previous.clone();
        current.generation = 9;
        assert!(!external_metric_changed("wifi", &current, Some(&previous)));
        current.wifi = Some(("test-network".into(), -40));
        assert!(external_metric_changed("wifi", &current, Some(&previous)));
    }
}
#[cfg(test)]
mod layout_editor_tests {
    use super::*;

    fn metric(id: &str, enabled: bool, zone: &str, priority: i32) -> config::MetricConf {
        config::MetricConf {
            id: id.into(),
            label: String::new(),
            enabled,
            command: String::new(),
            command_trusted: true,
            warn: 0.0,
            crit: 0.0,
            zone: zone.into(),
            priority,
            compact: true,
            format: String::new(),
            fg_color: String::new(),
            bg_color: String::new(),
        }
    }

    fn custom(name: &str, enabled: bool, zone: &str, priority: i32) -> CustomModule {
        CustomModule {
            name: name.into(),
            label: String::new(),
            format: String::new(),
            command: "echo hi".into(),
            enabled,
            trusted: true,
            zone: zone.into(),
            priority,
            compact: true,
            interval_ms: 1_000,
            timeout_ms: 5_000,
            max_output_bytes: 4_096,
            tooltip: String::new(),
            click_command: String::new(),
            fg_color: String::new(),
            bg_color: String::new(),
        }
    }

    #[test]
    fn collect_rows_orders_builtins_then_customs_by_priority() {
        let config = Config {
            metrics: vec![
                metric("clock", true, "end", 50),
                metric("memory", false, "center", 90),
                metric("cpu", true, "start", 80),
            ],
            custom_modules: vec![
                custom("low", true, "start", 10),
                custom("high", true, "end", 60),
            ],
            ..Config::default()
        };
        let rows = collect_layout_rows(&config);
        let keys: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
        assert_eq!(
            keys,
            ["builtin:cpu", "builtin:clock", "custom:high", "custom:low"]
        );
        // Built-ins (sorted by priority) always precede customs.
        assert!(keys[0].starts_with("builtin:") && keys[1].starts_with("builtin:"));
        assert!(keys[2].starts_with("custom:") && keys[3].starts_with("custom:"));
    }

    #[test]
    fn swap_only_reaches_same_zone_neighbour() {
        let mut config = Config {
            metrics: vec![
                metric("cpu", true, "start", 50),
                metric("ram", true, "start", 50),
                metric("disk", true, "end", 50),
            ],
            ..Config::default()
        };
        // Move cpu down: it must leap over disk (different zone) to land on ram.
        assert!(layout_row_swap(
            &mut config,
            &LayoutRowTarget::BuiltIn(0),
            1
        ));
        assert_eq!(config.metrics[0].id, "ram");
        assert_eq!(config.metrics[1].id, "cpu");
        assert_eq!(config.metrics[2].id, "disk");
        // Disk has no same-zone neighbour; swap must be a no-op.
        assert!(!layout_row_swap(
            &mut config,
            &LayoutRowTarget::BuiltIn(2),
            -1
        ));
        assert_eq!(config.metrics[2].id, "disk");
    }

    #[test]
    fn swap_moves_custom_modules_within_zone() {
        let mut config = Config {
            custom_modules: vec![
                custom("a", true, "center", 50),
                custom("b", true, "center", 50),
            ],
            ..Config::default()
        };
        assert!(layout_row_swap(&mut config, &LayoutRowTarget::Custom(0), 1));
        assert_eq!(config.custom_modules[0].name, "b");
        assert_eq!(config.custom_modules[1].name, "a");
    }

    #[test]
    fn preview_items_honor_adaptive_compact_off() {
        let mut config = Config {
            metrics: vec![metric("cpu", true, "start", 50)],
            ..Config::default()
        };
        config.adaptive_compact = false;
        let items = layout_preview_items(&config);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].profile.normal_px, items[0].profile.tiny_px);
        config.adaptive_compact = true;
        let items = layout_preview_items(&config);
        assert!(items[0].profile.tiny_px < items[0].profile.normal_px);
    }

    #[test]
    fn tier_summary_labels_every_state() {
        assert!(zone_tier_summary(LayoutTier::Normal, 104).contains("Normal"));
        assert!(zone_tier_summary(LayoutTier::Compact, 78).contains("Compact"));
        assert!(zone_tier_summary(LayoutTier::Tiny, 42).contains("Tiny"));
        assert_eq!(
            zone_tier_summary(LayoutTier::Overflow, 0),
            "→ More (hidden)"
        );
    }

    #[test]
    fn preview_decisions_match_allocator_at_1366() {
        let config = Config {
            metrics: vec![
                metric("cpu", true, "start", 90),
                metric("memory", true, "start", 80),
                metric("disk", true, "center", 10),
            ],
            ..Config::default()
        };
        let items = layout_preview_items(&config);
        let budget = module_width_budget(1_366, &config);
        let decisions = allocate_layout(&items, budget, config.module_gap.max(0) as u16);
        assert_eq!(decisions.len(), 3);
        assert!(decisions
            .iter()
            .all(|decision| decision.tier != LayoutTier::Overflow));
    }
}

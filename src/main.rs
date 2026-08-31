mod backend;
mod command;
mod config;
mod layout;
mod model;
mod runtime;

use backend::{CoreCollectionConfig, CoreCollector, CoreSnapshot, MetricLevel, MetricSample};
use command::{run_program_limited, run_shell_limited, CommandPolicy, CommandResult};
use config::{
    build_css, build_custom_css, css_class_id, default_prefix, read_import, Config, CustomModule,
    ThemePreset,
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
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use layout::{
    allocate_layout, custom_width_profile, module_width_profile, LayoutItem, LayoutTier,
    WidthProfile,
};
use model::{module_descriptor, module_registry, module_search_text};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use sysinfo::{ProcessesToUpdate, System};

use runtime::{ExternalSnapshot, QuickAction, RuntimeServices};

/// The visual representation of one bar slot: either a text label or a
/// small history graph.
enum SlotWidget {
    Text(Label),
    Graph {
        area: DrawingArea,
        hist: Rc<RefCell<VecDeque<f64>>>,
    },
    /// A draggable slider metric (Volume, Brightness). `suppress` is set
    /// while the periodic poll syncs the value from the OS, so that sync
    /// doesn't re-trigger the drag handler and fight the user's input.
    Range {
        scale: Scale,
        suppress: Rc<Cell<bool>>,
        icon: Option<Button>,
    },
}

/// A single item shown on the bar, built from a `MetricConf`/`CustomModule`.
struct Slot {
    id: String,
    prefix: String,
    /// Optional display template with `{label}` and `{value}`.
    format: String,
    /// Non-empty only for custom (shell-command) modules.
    source_cmd: String,
    warn: f64,
    crit: f64,
    widget: SlotWidget,
    layout: SlotLayoutHandle,
    last_value: RefCell<Option<String>>,
    /// Channel used to receive command output from a background thread,
    /// so a slow custom-module command never blocks the UI thread.
    cmd_tx: Option<Sender<CommandResult>>,
    cmd_rx: Option<Receiver<CommandResult>>,
    /// True while a background command for this slot is still running,
    /// preventing threads from piling up if a command is slow.
    busy: Option<Rc<Cell<bool>>>,
    /// On-demand run request, kept pending while the previous run is busy.
    force_run: Option<Rc<Cell<bool>>>,
    interval_ms: u64,
    timeout_ms: u64,
    max_output_bytes: usize,
    trusted: bool,
    last_started: RefCell<Option<Instant>>,
}

#[derive(Clone)]
struct SlotLayoutHandle {
    tier: Rc<Cell<LayoutTier>>,
    dirty: Rc<Cell<bool>>,
    overflow_value: Label,
}

struct AdaptiveEntry {
    key: String,
    display_name: String,
    priority: i32,
    order: usize,
    profile: WidthProfile,
    root: gtk::Widget,
    text: Option<Label>,
    graph: Option<DrawingArea>,
    scale: Option<Scale>,
    overflow_row: GtkBox,
    handle: SlotLayoutHandle,
}

type AdaptiveEntries = Rc<RefCell<Vec<AdaptiveEntry>>>;

#[derive(Clone)]
struct OverflowUi {
    button: gtk::MenuButton,
    content: GtkBox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CatalogKind {
    BuiltIn,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ModuleCatalogKey {
    BuiltIn(String),
    Custom(usize),
}

struct ModuleCatalogRecord {
    title: String,
    subtitle: String,
    search_text: String,
    category: String,
    kind: CatalogKind,
    enabled: bool,
}

struct ModuleCatalogRow {
    key: ModuleCatalogKey,
    row: ListBoxRow,
    title: Label,
    subtitle: Label,
    state: Label,
}

struct ModuleCatalogUi {
    root: GtkBox,
    paned: Paned,
    refresh_slots: Vec<RefreshSlot>,
}

type Active = Rc<RefCell<Vec<Slot>>>;
type RefreshFn = Box<dyn Fn()>;
type RefreshSlot = Rc<RefCell<Option<RefreshFn>>>;

#[derive(Clone)]
struct BarZones {
    start: GtkBox,
    center: GtkBox,
    end: GtkBox,
}

impl BarZones {
    fn new(spacing: i32) -> Self {
        let start = GtkBox::new(Orientation::Horizontal, spacing);
        let center = GtkBox::new(Orientation::Horizontal, spacing);
        let end = GtkBox::new(Orientation::Horizontal, spacing);
        for zone in [&start, &center, &end] {
            zone.add_css_class("zone");
            zone.set_valign(gtk::Align::Center);
        }
        Self { start, center, end }
    }

    fn clear(&self) {
        for zone in [&self.start, &self.center, &self.end] {
            while let Some(child) = zone.first_child() {
                zone.remove(&child);
            }
        }
    }

    fn set_spacing(&self, spacing: i32) {
        self.start.set_spacing(spacing);
        self.center.set_spacing(spacing);
        self.end.set_spacing(spacing);
    }

    fn append(&self, zone: &str, widget: &impl IsA<gtk::Widget>) {
        match zone {
            "start" => self.start.append(widget),
            "end" => self.end.append(widget),
            _ => self.center.append(widget),
        }
    }
}

impl OverflowUi {
    fn new() -> Self {
        let content = GtkBox::new(Orientation::Vertical, 6);
        content.set_margin_top(10);
        content.set_margin_bottom(10);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let scroll = ScrolledWindow::builder()
            .min_content_width(300)
            .max_content_height(420)
            .child(&content)
            .build();
        let popover = gtk::Popover::new();
        popover.set_child(Some(&scroll));
        let button = gtk::MenuButton::new();
        button.set_label("More");
        button.set_tooltip_text(Some("Open overflow status"));
        button.set_popover(Some(&popover));
        button.add_css_class("overflow-button");
        Self { button, content }
    }
}

#[allow(clippy::too_many_arguments)]
fn make_adaptive_entry(
    key: String,
    display_name: String,
    priority: i32,
    order: usize,
    profile: WidthProfile,
    allow_compact: bool,
    root: gtk::Widget,
    text: Option<Label>,
    graph: Option<DrawingArea>,
    scale: Option<Scale>,
) -> (SlotLayoutHandle, AdaptiveEntry) {
    let handle = SlotLayoutHandle {
        tier: Rc::new(Cell::new(LayoutTier::Normal)),
        dirty: Rc::new(Cell::new(true)),
        overflow_value: Label::new(Some("Waiting…")),
    };
    handle.overflow_value.set_xalign(1.0);
    handle.overflow_value.set_hexpand(true);
    handle
        .overflow_value
        .set_ellipsize(gtk::pango::EllipsizeMode::End);
    handle.overflow_value.set_max_width_chars(36);

    let overflow_row = GtkBox::new(Orientation::Horizontal, 12);
    overflow_row.add_css_class("overflow-row");
    let title = Label::new(Some(&display_name));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    overflow_row.append(&title);
    overflow_row.append(&handle.overflow_value);

    let profile = if allow_compact {
        profile
    } else {
        profile.fixed_normal()
    };
    let entry = AdaptiveEntry {
        key,
        display_name,
        priority,
        order,
        profile,
        root,
        text,
        graph,
        scale,
        overflow_row,
        handle: handle.clone(),
    };
    (handle, entry)
}

fn module_width_budget(window_width: i32, cfg: &Config) -> u32 {
    let quick_actions = [cfg.qa_lock, cfg.qa_screenshot, cfg.qa_shutdown]
        .into_iter()
        .filter(|enabled| *enabled)
        .count() as u32;
    let reserved = 116_u32.saturating_add(quick_actions.saturating_mul(78));
    (window_width.max(0) as u32).saturating_sub(reserved)
}

fn apply_adaptive_layout(
    entries: &AdaptiveEntries,
    overflow: &OverflowUi,
    window_width: i32,
    cfg: &Config,
) {
    let entries = entries.borrow();
    let items: Vec<LayoutItem> = entries
        .iter()
        .map(|entry| LayoutItem {
            key: entry.key.clone(),
            priority: entry.priority,
            order: entry.order,
            profile: entry.profile,
        })
        .collect();
    let budget = module_width_budget(window_width, cfg);
    let gap = cfg.module_gap.max(0) as u16;
    let decisions = allocate_layout(&items, budget, gap);

    while let Some(child) = overflow.content.first_child() {
        overflow.content.remove(&child);
    }
    let mut hidden_names = Vec::new();
    let mut modeled_width = 0_u32;
    for (entry, decision) in entries.iter().zip(&decisions) {
        debug_assert_eq!(entry.key.as_str(), decision.key.as_str());
        if entry.handle.tier.replace(decision.tier) != decision.tier {
            entry.handle.dirty.set(true);
        }
        for class in [
            "layout-normal",
            "layout-compact",
            "layout-tiny",
            "layout-overflow",
        ] {
            entry.root.remove_css_class(class);
        }
        entry.root.add_css_class(decision.tier.css_class());
        entry
            .root
            .set_visible(decision.tier != LayoutTier::Overflow);
        modeled_width = modeled_width.saturating_add(u32::from(decision.width_px));

        if let Some(label) = &entry.text {
            label.set_max_width_chars(match decision.tier {
                LayoutTier::Normal | LayoutTier::Overflow => 64,
                LayoutTier::Compact => 28,
                LayoutTier::Tiny => 12,
            });
        }
        if let Some(graph) = &entry.graph {
            graph.set_content_width(match decision.tier {
                LayoutTier::Normal | LayoutTier::Overflow => 64,
                LayoutTier::Compact => 50,
                LayoutTier::Tiny => 36,
            });
        }
        if let Some(scale) = &entry.scale {
            scale.set_size_request(
                match decision.tier {
                    LayoutTier::Normal | LayoutTier::Overflow => 80,
                    LayoutTier::Compact => 58,
                    LayoutTier::Tiny => 38,
                },
                -1,
            );
        }
        if decision.tier == LayoutTier::Overflow {
            hidden_names.push(entry.display_name.clone());
            overflow.content.append(&entry.overflow_row);
        }
    }

    let hidden = hidden_names.len();
    if hidden == 0 {
        let empty = Label::new(Some(
            "Everything fits on the bar. Lower-priority modules will appear here when space becomes limited.",
        ));
        empty.add_css_class("overflow-empty");
        empty.set_wrap(true);
        empty.set_xalign(0.0);
        overflow.content.append(&empty);
        overflow.button.set_label("More");
        overflow.button.set_sensitive(true);
        overflow
            .button
            .set_tooltip_text(Some("Open overflow status"));
    } else {
        overflow.button.set_label(&format!("More {hidden}"));
        overflow.button.set_sensitive(true);
        overflow.button.set_tooltip_text(Some(&format!(
            "Hidden at {window_width}px (budget {budget}px, modeled {modeled_width}px): {}",
            hidden_names.join(", ")
        )));
    }
}

#[derive(Clone)]
struct ApplyActions {
    style: Rc<dyn Fn()>,
    layout: Rc<dyn Fn()>,
    geometry: Rc<dyn Fn()>,
    interval: Rc<dyn Fn()>,
    all: Rc<dyn Fn()>,
}

#[derive(Clone)]
struct SettingsContext {
    bar_window: ApplicationWindow,
    active: Active,
    runtime: RuntimeServices,
    core: CoreCollector,
    actions: ApplyActions,
    settings_slot: Rc<RefCell<Option<Window>>>,
}

thread_local! {
    static DISK_PREV: RefCell<HashMap<String, (u64, u64, Instant)>> = RefCell::new(HashMap::new());
    static RAPL_PREV: RefCell<(u64, Instant)> = RefCell::new((0, Instant::now()));
    static ALERTS: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
}

fn main() {
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
    window.set_child(Some(&root));

    let active: Active = Rc::new(RefCell::new(Vec::new()));
    let adaptive_entries: AdaptiveEntries = Rc::new(RefCell::new(Vec::new()));
    rebuild_bar_metrics(&zones, &active, &adaptive_entries, &cfg.borrow(), &runtime);
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
    let tick = Rc::new(Cell::new(0_u32));
    let poll_source: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let history_accum_ms = Rc::new(Cell::new(0_u64));
    let last_core_snapshot: Rc<RefCell<Option<CoreSnapshot>>> = Rc::new(RefCell::new(None));
    let last_external_snapshot: Rc<RefCell<Option<ExternalSnapshot>>> = Rc::new(RefCell::new(None));

    let respawn: Rc<dyn Fn(u64)> = {
        let (active, cfg, provider, tick, poll_source, core) = (
            active.clone(),
            cfg.clone(),
            provider.clone(),
            tick.clone(),
            poll_source.clone(),
            core.clone(),
        );
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
            let (active, cfg, provider, tick, core) = (
                active.clone(),
                cfg.clone(),
                provider.clone(),
                tick.clone(),
                core.clone(),
            );
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
                if config.animated_bg && !config.reduce_motion {
                    let phase = tick.get();
                    tick.set(phase.wrapping_add(1));
                    provider.load_from_string(&build_css(
                        &config,
                        Some((phase as f64 * step_ms as f64 / 1_000.0 * 18.0) % 360.0),
                    ));
                }
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
        );
        Rc::new(move || {
            let config = cfg.borrow();
            core.configure(CoreCollectionConfig::new(
                config.interval_ms,
                config.net_iface.clone(),
                config.gpu_index as u32,
            ));
            rebuild_bar_metrics(&zones, &active, &adaptive_entries, &config, &runtime);
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

// ---------- Positioning: Wayland layer-shell & X11 struts ----------

#[cfg(feature = "wayland")]
fn configure_wayland(window: &ApplicationWindow, cfg: &Config) {
    let bottom = cfg.position == "bottom";
    window.set_layer(Layer::Top);
    window.set_anchor(Edge::Top, !bottom);
    window.set_anchor(Edge::Bottom, bottom);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, if bottom { 0 } else { cfg.margin_top });
    window.set_margin(Edge::Bottom, if bottom { cfg.margin_top } else { 0 });
    // The exclusive zone is what actually reserves screen space and
    // forces other windows (including fullscreen ones) to shrink around it.
    window.set_exclusive_zone(cfg.height + cfg.margin_top);
    if cfg.monitor >= 0 {
        if let Some(display) = gtk::gdk::Display::default() {
            if let Some(obj) = display.monitors().item(cfg.monitor as u32) {
                if let Ok(mon) = obj.downcast::<gtk::gdk::Monitor>() {
                    window.set_monitor(&mon);
                }
            }
        }
    }
}

fn monitor_geometry(idx: i32) -> Option<(i32, i32, i32, i32)> {
    let display = gtk::gdk::Display::default()?;
    let monitors = display.monitors();
    let obj = monitors.item(if idx >= 0 { idx as u32 } else { 0 })?;
    let mon = obj.downcast::<gtk::gdk::Monitor>().ok()?;
    let g = mon.geometry();
    Some((g.x(), g.y(), g.width(), g.height()))
}

fn configure_x11(window: &ApplicationWindow, cfg: &Config) {
    if let Some(surface) = window.surface() {
        if let Ok(x11) = surface.downcast::<gdk4_x11::X11Surface>() {
            let geo = monitor_geometry(cfg.monitor);
            match apply_x11_dock(
                x11.xid() as u32,
                geo,
                cfg.margin_top as u32,
                cfg.height as u32,
                cfg.position == "bottom",
            ) {
                Ok(w) => window.set_size_request(w as i32, cfg.height),
                Err(e) => eprintln!("[TopMonitoring] Failed to apply X11 dock hints: {e}"),
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct X11DockGeometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    strut_partial: [u32; 12],
    strut: [u32; 4],
}

fn calculate_x11_dock_geometry(
    screen_width: i32,
    screen_height: i32,
    monitor: Option<(i32, i32, i32, i32)>,
    offset: u32,
    bar_height: u32,
    bottom: bool,
) -> Option<X11DockGeometry> {
    if screen_width <= 0 || screen_height <= 0 || bar_height == 0 {
        return None;
    }
    let (monitor_x, monitor_y, monitor_width, monitor_height) =
        monitor.unwrap_or((0, 0, screen_width, screen_height));
    if monitor_width <= 0 || monitor_height <= 0 {
        return None;
    }

    let width = monitor_width.clamp(1, screen_width) as u32;
    let height = (bar_height as i32).clamp(1, screen_height) as u32;
    let width_i = width as i32;
    let height_i = height as i32;
    let x = monitor_x.clamp(0, screen_width - width_i);
    let x_start = x as u32;
    let x_end = (x + width_i - 1).clamp(0, screen_width - 1) as u32;
    let monitor_top = monitor_y.clamp(0, screen_height);
    let monitor_bottom = monitor_y
        .saturating_add(monitor_height)
        .clamp(0, screen_height);
    let offset_i = (offset.min(screen_height as u32)) as i32;

    let (y, reserve) = if bottom {
        let outside = (screen_height - monitor_bottom).max(0);
        let reserve = outside
            .saturating_add(offset_i)
            .saturating_add(height_i)
            .clamp(0, screen_height) as u32;
        let y = monitor_bottom
            .saturating_sub(offset_i)
            .saturating_sub(height_i)
            .clamp(0, screen_height - height_i);
        (y, reserve)
    } else {
        let reserve = monitor_top
            .saturating_add(offset_i)
            .saturating_add(height_i)
            .clamp(0, screen_height) as u32;
        let y = monitor_top
            .saturating_add(offset_i)
            .clamp(0, screen_height - height_i);
        (y, reserve)
    };

    let (strut_partial, strut) = if bottom {
        (
            [0, 0, 0, reserve, 0, 0, 0, 0, 0, 0, x_start, x_end],
            [0, 0, 0, reserve],
        )
    } else {
        (
            [0, 0, reserve, 0, 0, 0, 0, 0, x_start, x_end, 0, 0],
            [0, 0, reserve, 0],
        )
    };
    Some(X11DockGeometry {
        x,
        y,
        width,
        height,
        strut_partial,
        strut,
    })
}

/// Turns a plain X11 window into a dock: sets `_NET_WM_WINDOW_TYPE_DOCK`,
/// reserves space via `_NET_WM_STRUT_PARTIAL`/`_NET_WM_STRUT`, keeps it on
/// all workspaces, and pins its exact geometry. Returns the applied width.
fn apply_x11_dock(
    xid: u32,
    geo: Option<(i32, i32, i32, i32)>,
    offset: u32,
    bar_height: u32,
    bottom: bool,
) -> Result<u32, Box<dyn std::error::Error>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ConfigureWindowAux, ConnectionExt as XProto, PropMode,
    };
    use x11rb::wrapper::ConnectionExt as Wrapper;

    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let sw = screen.width_in_pixels as i32;
    let sh = screen.height_in_pixels as i32;
    let geometry = calculate_x11_dock_geometry(sw, sh, geo, offset, bar_height, bottom)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid X11 dock geometry",
            )
        })?;

    let atom = |name: &[u8]| -> Result<u32, Box<dyn std::error::Error>> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    };

    let wtype = atom(b"_NET_WM_WINDOW_TYPE")?;
    let dock = atom(b"_NET_WM_WINDOW_TYPE_DOCK")?;
    conn.change_property32(PropMode::REPLACE, xid, wtype, AtomEnum::ATOM, &[dock])?;

    let strut_p = atom(b"_NET_WM_STRUT_PARTIAL")?;
    let strut = atom(b"_NET_WM_STRUT")?;
    conn.change_property32(
        PropMode::REPLACE,
        xid,
        strut_p,
        AtomEnum::CARDINAL,
        &geometry.strut_partial,
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        xid,
        strut,
        AtomEnum::CARDINAL,
        &geometry.strut,
    )?;

    let desktop = atom(b"_NET_WM_DESKTOP")?;
    conn.change_property32(
        PropMode::REPLACE,
        xid,
        desktop,
        AtomEnum::CARDINAL,
        &[0xFFFF_FFFF],
    )?;

    let state = atom(b"_NET_WM_STATE")?;
    let sticky = atom(b"_NET_WM_STATE_STICKY")?;
    let above = atom(b"_NET_WM_STATE_ABOVE")?;
    let skip_tb = atom(b"_NET_WM_STATE_SKIP_TASKBAR")?;
    let skip_pg = atom(b"_NET_WM_STATE_SKIP_PAGER")?;
    conn.change_property32(
        PropMode::REPLACE,
        xid,
        state,
        AtomEnum::ATOM,
        &[sticky, above, skip_tb, skip_pg],
    )?;

    conn.configure_window(
        xid,
        &ConfigureWindowAux::new()
            .x(geometry.x)
            .y(geometry.y)
            .width(geometry.width)
            .height(geometry.height),
    )?;
    conn.flush()?;
    Ok(geometry.width)
}

// ---------- Bar widgets ----------

/// Runs `cmd` in a shell when `w` is left-clicked. No-op if `cmd` is empty.
fn attach_click(w: &impl IsA<gtk::Widget>, command: &str, trusted: bool) {
    if command.trim().is_empty() {
        return;
    }
    if !trusted {
        w.set_tooltip_text(Some("Imported click command is blocked until trusted"));
        return;
    }
    let gesture = GestureClick::new();
    gesture.set_button(1);
    let command = command.to_string();
    gesture.connect_pressed(move |_, _, _, _| {
        let command = command.clone();
        std::thread::spawn(move || {
            let _ = run_shell_limited(
                &command,
                CommandPolicy {
                    timeout: Duration::from_secs(10),
                    max_output_bytes: 4 * 1024,
                },
            );
        });
    });
    w.add_controller(gesture);
}

/// Middle-click shows a popover with whatever is currently in the widget's
/// tooltip, giving a richer "detail view" without duplicating any logic.
fn attach_detail_popover<W: IsA<gtk::Widget> + Clone + 'static>(w: &W) {
    let gesture = GestureClick::new();
    gesture.set_button(2);
    let w2 = w.clone();
    gesture.connect_pressed(move |_, _, _, _| {
        let text = w2
            .tooltip_text()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "No details available".to_string());
        let popover = gtk::Popover::new();
        popover.set_parent(&w2);
        let lbl = Label::new(Some(&text));
        lbl.set_margin_top(10);
        lbl.set_margin_bottom(10);
        lbl.set_margin_start(12);
        lbl.set_margin_end(12);
        lbl.set_xalign(0.0);
        popover.set_child(Some(&lbl));
        popover.connect_closed(|p| p.unparent());
        popover.popup();
    });
    w.add_controller(gesture);
}

/// Creates a small history sparkline. The line and its filled area shift
/// color (cyan -> amber -> red) as the latest value nears warning/critical
/// levels for percentage-based graphs (CPU/RAM); other graphs stay cyan.
fn make_graph(
    kind: &str,
    warn_override: f64,
    crit_override: f64,
) -> (DrawingArea, Rc<RefCell<VecDeque<f64>>>) {
    let area = DrawingArea::new();
    area.set_content_width(64);
    area.set_vexpand(true);
    let hist = Rc::new(RefCell::new(VecDeque::with_capacity(64)));
    let h2 = hist.clone();
    let kind = kind.to_string();
    area.set_draw_func(move |_, cr, w, h| {
        let hist = h2.borrow();
        if hist.len() < 2 {
            return;
        }
        let (maxv, default_warn, default_crit) = match kind.as_str() {
            "cpu_graph" => (100.0, 70.0, 90.0),
            "ram_graph" => (100.0, 80.0, 92.0),
            _ => (
                hist.iter().cloned().fold(1.0_f64, f64::max),
                f64::INFINITY,
                f64::INFINITY,
            ),
        };
        let warn = if warn_override > 0.0 {
            warn_override
        } else {
            default_warn
        };
        let crit = if crit_override > 0.0 {
            crit_override
        } else {
            default_crit
        };
        let last = *hist.back().unwrap();
        let (r, g, b) = if last >= crit {
            (1.0, 0.25, 0.25)
        } else if last >= warn {
            (1.0, 0.69, 0.13)
        } else {
            (0.0, 0.82, 1.0)
        };

        let n = hist.len();
        let dx = w as f64 / (n - 1) as f64;
        let points: Vec<(f64, f64)> = hist
            .iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    i as f64 * dx,
                    h as f64 - (v / maxv).clamp(0.0, 1.0) * h as f64,
                )
            })
            .collect();

        // Filled area under the curve.
        cr.set_source_rgba(r, g, b, 0.18);
        cr.move_to(points[0].0, h as f64);
        for &(x, y) in &points {
            cr.line_to(x, y);
        }
        cr.line_to(points[points.len() - 1].0, h as f64);
        cr.close_path();
        let _ = cr.fill();

        // The line itself.
        cr.set_source_rgba(r, g, b, 0.95);
        cr.set_line_width(1.6);
        for (i, &(x, y)) in points.iter().enumerate() {
            if i == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        let _ = cr.stroke();
    });
    (area, hist)
}

/// Rebuilds every widget on the bar from `cfg`. Cheap enough to call after
/// any settings change (rename, reorder, toggle, threshold edit, ...).
fn add_module_classes(widget: &impl IsA<gtk::Widget>, id: &str) {
    widget.add_css_class("module");
    widget.add_css_class(&format!("module-{}", css_class_id(id)));
}

fn connect_action_feedback(
    button: &Button,
    idle_label: &'static str,
    pending_label: &'static str,
    action: QuickAction,
    runtime: &RuntimeServices,
) {
    let service = runtime.clone();
    button.connect_clicked(move |button| {
        let request_id = service.quick_action(action);
        button.set_label(pending_label);
        button.set_sensitive(false);
        button.remove_css_class("action-ok");
        button.remove_css_class("action-error");
        button.add_css_class("action-pending");

        let button = button.clone();
        let service = service.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if button.root().is_none() {
                return glib::ControlFlow::Break;
            }
            let snapshot = service.snapshot();
            let Some(feedback) = snapshot
                .action_results
                .iter()
                .rev()
                .find(|feedback| feedback.request_id == request_id)
            else {
                return glib::ControlFlow::Continue;
            };

            button.set_label(idle_label);
            button.set_sensitive(true);
            button.remove_css_class("action-pending");
            let status_class = if feedback.success {
                "action-ok"
            } else {
                "action-error"
            };
            button.add_css_class(status_class);
            button.set_tooltip_text(Some(&format!(
                "{} · {} · {} ms",
                feedback.message, feedback.provider, feedback.duration_ms
            )));

            let button = button.clone();
            glib::timeout_add_local(Duration::from_secs(3), move || {
                button.remove_css_class("action-ok");
                button.remove_css_class("action-error");
                glib::ControlFlow::Break
            });
            glib::ControlFlow::Break
        });
    });
}

/// Rebuilds the three user-controlled zones. This is called only for layout or
/// module-definition changes, never for simple style or poll-interval changes.
fn rebuild_bar_metrics(
    zones: &BarZones,
    active: &Active,
    adaptive: &AdaptiveEntries,
    cfg: &Config,
    runtime: &RuntimeServices,
) {
    zones.clear();
    zones.set_spacing(cfg.module_gap);
    let mut slots = active.borrow_mut();
    slots.clear();
    adaptive.borrow_mut().clear();
    let mut layout_order = 0_usize;

    let mut metric_order: Vec<usize> = (0..cfg.metrics.len()).collect();
    metric_order.sort_by(|left, right| {
        cfg.metrics[*right]
            .priority
            .cmp(&cfg.metrics[*left].priority)
            .then_with(|| left.cmp(right))
    });
    for metric_index in metric_order {
        let metric = &cfg.metrics[metric_index];
        if !metric.enabled {
            continue;
        }
        let descriptor = module_descriptor(&metric.id);
        let display_name = descriptor
            .map(|descriptor| descriptor.display_name.to_string())
            .unwrap_or_else(|| default_prefix(&metric.id).to_string());
        let profile = descriptor
            .map(|descriptor| descriptor.width_profile())
            .unwrap_or_else(|| module_width_profile(&metric.id));
        let allow_compact = cfg.adaptive_compact && metric.compact;
        if metric.id.ends_with("_graph") {
            let (area, history) = make_graph(&metric.id, metric.warn, metric.crit);
            add_module_classes(&area, &metric.id);
            zones.append(&metric.zone, &area);
            attach_click(&area, &metric.command, metric.command_trusted);
            attach_detail_popover(&area);
            let (layout, entry) = make_adaptive_entry(
                metric.id.clone(),
                display_name.clone(),
                metric.priority,
                layout_order,
                profile,
                allow_compact,
                area.clone().upcast::<gtk::Widget>(),
                None,
                Some(area.clone()),
                None,
            );
            layout_order += 1;
            adaptive.borrow_mut().push(entry);
            slots.push(Slot {
                id: metric.id.clone(),
                prefix: String::new(),
                format: String::new(),
                source_cmd: String::new(),
                warn: metric.warn,
                crit: metric.crit,
                widget: SlotWidget::Graph {
                    area,
                    hist: history,
                },
                layout,
                last_value: RefCell::new(None),
                cmd_tx: None,
                cmd_rx: None,
                busy: None,
                force_run: None,
                interval_ms: 0,
                timeout_ms: 0,
                max_output_bytes: 0,
                trusted: true,
                last_started: RefCell::new(None),
            });
        } else if metric.id == "volume" {
            let (scale, suppress) = make_range();
            let mute_button = Button::with_label("Audio");
            mute_button.set_tooltip_text(Some("Toggle mute"));
            let service = runtime.clone();
            mute_button.connect_clicked(move |_| service.toggle_mute());
            let row = GtkBox::new(Orientation::Horizontal, 3);
            add_module_classes(&row, &metric.id);
            row.append(&mute_button);
            row.append(&scale);
            zones.append(&metric.zone, &row);
            {
                let suppress = suppress.clone();
                let service = runtime.clone();
                let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
                scale.connect_value_changed(move |widget| {
                    if suppress.get() {
                        return;
                    }
                    if let Some(source) = pending.borrow_mut().take() {
                        source.remove();
                    }
                    let value = widget.value().round() as u32;
                    let service = service.clone();
                    let pending2 = pending.clone();
                    let source = glib::timeout_add_local(Duration::from_millis(80), move || {
                        service.set_volume(value);
                        *pending2.borrow_mut() = None;
                        glib::ControlFlow::Break
                    });
                    *pending.borrow_mut() = Some(source);
                });
            }
            let (layout, entry) = make_adaptive_entry(
                metric.id.clone(),
                display_name.clone(),
                metric.priority,
                layout_order,
                profile,
                allow_compact,
                row.clone().upcast::<gtk::Widget>(),
                None,
                None,
                Some(scale.clone()),
            );
            layout_order += 1;
            adaptive.borrow_mut().push(entry);
            slots.push(Slot {
                id: metric.id.clone(),
                prefix: String::new(),
                format: String::new(),
                source_cmd: String::new(),
                warn: metric.warn,
                crit: metric.crit,
                widget: SlotWidget::Range {
                    scale,
                    suppress,
                    icon: Some(mute_button),
                },
                layout,
                last_value: RefCell::new(None),
                cmd_tx: None,
                cmd_rx: None,
                busy: None,
                force_run: None,
                interval_ms: 0,
                timeout_ms: 0,
                max_output_bytes: 0,
                trusted: true,
                last_started: RefCell::new(None),
            });
        } else if metric.id == "brightness" {
            let (scale, suppress) = make_range();
            let icon = Label::new(Some("Light"));
            let row = GtkBox::new(Orientation::Horizontal, 3);
            add_module_classes(&row, &metric.id);
            row.append(&icon);
            row.append(&scale);
            zones.append(&metric.zone, &row);
            {
                let suppress = suppress.clone();
                let service = runtime.clone();
                let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
                scale.connect_value_changed(move |widget| {
                    if suppress.get() {
                        return;
                    }
                    if let Some(source) = pending.borrow_mut().take() {
                        source.remove();
                    }
                    let value = widget.value().round() as u32;
                    let service = service.clone();
                    let pending2 = pending.clone();
                    let source = glib::timeout_add_local(Duration::from_millis(80), move || {
                        service.set_brightness(value);
                        *pending2.borrow_mut() = None;
                        glib::ControlFlow::Break
                    });
                    *pending.borrow_mut() = Some(source);
                });
            }
            let (layout, entry) = make_adaptive_entry(
                metric.id.clone(),
                display_name.clone(),
                metric.priority,
                layout_order,
                profile,
                allow_compact,
                row.clone().upcast::<gtk::Widget>(),
                None,
                None,
                Some(scale.clone()),
            );
            layout_order += 1;
            adaptive.borrow_mut().push(entry);
            slots.push(Slot {
                id: metric.id.clone(),
                prefix: String::new(),
                format: String::new(),
                source_cmd: String::new(),
                warn: metric.warn,
                crit: metric.crit,
                widget: SlotWidget::Range {
                    scale,
                    suppress,
                    icon: None,
                },
                layout,
                last_value: RefCell::new(None),
                cmd_tx: None,
                cmd_rx: None,
                busy: None,
                force_run: None,
                interval_ms: 0,
                timeout_ms: 0,
                max_output_bytes: 0,
                trusted: true,
                last_started: RefCell::new(None),
            });
        } else {
            let label = Label::new(None);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_max_width_chars(64);
            add_module_classes(&label, &metric.id);
            if metric.id == "clock" {
                label.add_css_class("accent");
            }
            zones.append(&metric.zone, &label);
            attach_click(&label, &metric.command, metric.command_trusted);
            attach_detail_popover(&label);
            let prefix = if metric.label.is_empty() {
                default_prefix(&metric.id).to_string()
            } else {
                metric.label.clone()
            };
            let (layout, entry) = make_adaptive_entry(
                metric.id.clone(),
                display_name,
                metric.priority,
                layout_order,
                profile,
                allow_compact,
                label.clone().upcast::<gtk::Widget>(),
                Some(label.clone()),
                None,
                None,
            );
            layout_order += 1;
            adaptive.borrow_mut().push(entry);
            slots.push(Slot {
                id: metric.id.clone(),
                prefix,
                format: metric.format.clone(),
                source_cmd: String::new(),
                warn: metric.warn,
                crit: metric.crit,
                widget: SlotWidget::Text(label),
                layout,
                last_value: RefCell::new(None),
                cmd_tx: None,
                cmd_rx: None,
                busy: None,
                force_run: None,
                interval_ms: 0,
                timeout_ms: 0,
                max_output_bytes: 0,
                trusted: metric.command_trusted,
                last_started: RefCell::new(None),
            });
        }
    }

    let mut custom_order: Vec<usize> = (0..cfg.custom_modules.len()).collect();
    custom_order.sort_by(|left, right| {
        cfg.custom_modules[*right]
            .priority
            .cmp(&cfg.custom_modules[*left].priority)
            .then_with(|| left.cmp(right))
    });
    for module_index in custom_order {
        let module = &cfg.custom_modules[module_index];
        if !module.enabled {
            continue;
        }
        let profile = custom_width_profile();
        let allow_compact = cfg.adaptive_compact && module.compact;
        let label = Label::new(Some(if module.trusted { "…" } else { "blocked" }));
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(64);
        add_module_classes(&label, &format!("custom-{}", module.name));
        if !module.tooltip.trim().is_empty() {
            label.set_tooltip_text(Some(&module.tooltip));
        } else if !module.trusted {
            label.set_tooltip_text(Some("Command is disabled until explicitly trusted"));
        }
        zones.append(&module.zone, &label);
        attach_detail_popover(&label);
        attach_click(&label, &module.click_command, module.trusted);
        let (sender, receiver) = mpsc::channel::<CommandResult>();
        let slot_id = format!("custom:{}", module.name);
        let (layout, entry) = make_adaptive_entry(
            slot_id.clone(),
            module.name.clone(),
            module.priority,
            layout_order,
            profile,
            allow_compact,
            label.clone().upcast::<gtk::Widget>(),
            Some(label.clone()),
            None,
            None,
        );
        layout_order += 1;
        adaptive.borrow_mut().push(entry);
        slots.push(Slot {
            id: slot_id,
            prefix: module.label.clone(),
            format: module.format.clone(),
            source_cmd: module.command.clone(),
            warn: 0.0,
            crit: 0.0,
            widget: SlotWidget::Text(label),
            layout,
            last_value: RefCell::new(None),
            cmd_tx: Some(sender),
            cmd_rx: Some(receiver),
            busy: Some(Rc::new(Cell::new(false))),
            force_run: Some(Rc::new(Cell::new(false))),
            interval_ms: module.interval_ms,
            timeout_ms: module.timeout_ms,
            max_output_bytes: module.max_output_bytes,
            trusted: module.trusted,
            last_started: RefCell::new(None),
        });
    }

    if cfg.qa_lock {
        let button = Button::with_label("Lock");
        button.set_tooltip_text(Some("Lock the current session"));
        add_module_classes(&button, "action-lock");
        connect_action_feedback(&button, "Lock", "Lock…", QuickAction::Lock, runtime);
        zones.end.append(&button);
    }
    if cfg.qa_screenshot {
        let button = Button::with_label("Capture");
        button.set_tooltip_text(Some("Open the first supported screenshot tool"));
        add_module_classes(&button, "action-screenshot");
        connect_action_feedback(
            &button,
            "Capture",
            "Capture…",
            QuickAction::Screenshot,
            runtime,
        );
        zones.end.append(&button);
    }
    if cfg.qa_shutdown {
        let button = Button::with_label("Power");
        button.set_tooltip_text(Some("Click twice within four seconds to power off"));
        add_module_classes(&button, "action-shutdown");
        let armed = Rc::new(RefCell::new(None::<Instant>));
        let service = runtime.clone();
        let button2 = button.clone();
        button.connect_clicked(move |_| {
            let confirmed = armed
                .borrow()
                .map(|instant| instant.elapsed() <= Duration::from_secs(4))
                .unwrap_or(false);
            if confirmed {
                *armed.borrow_mut() = None;
                button2.set_label("Power");
                service.quick_action(QuickAction::Shutdown);
                return;
            }
            *armed.borrow_mut() = Some(Instant::now());
            button2.set_label("Confirm");
            let armed2 = armed.clone();
            let button3 = button2.clone();
            glib::timeout_add_local(Duration::from_secs(4), move || {
                if armed2
                    .borrow()
                    .map(|instant| instant.elapsed() >= Duration::from_secs(4))
                    .unwrap_or(false)
                {
                    *armed2.borrow_mut() = None;
                    button3.set_label("Power");
                }
                glib::ControlFlow::Break
            });
        });
        zones.end.append(&button);
    }
}

// ---------- Updating widgets ----------

fn set_formatted(label: &Label, prefix: &str, template: &str, value: &str) {
    if template.trim().is_empty() {
        if prefix.is_empty() {
            label.set_text(value);
        } else {
            label.set_text(&format!("{prefix} {value}"));
        }
        return;
    }
    label.set_text(
        &template
            .replace("{label}", prefix)
            .replace("{value}", value),
    );
}

#[derive(Clone, Debug, PartialEq)]
struct DiskSummary {
    name: String,
    mount: String,
    file_system: String,
    used_percent: f64,
    available_gb: f64,
    total_gb: f64,
}

fn compact_disk_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    let mut compact: String = name.chars().take(max_chars.saturating_sub(1)).collect();
    compact.push('…');
    compact
}

fn disk_bar_entries(entries: &[DiskSummary]) -> Vec<&DiskSummary> {
    let root = entries.iter().find(|entry| entry.mount == "/");
    let worst_other = entries
        .iter()
        .filter(|entry| entry.mount != "/")
        .max_by(|left, right| {
            left.used_percent
                .partial_cmp(&right.used_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let mut selected = Vec::with_capacity(2);
    if let Some(root) = root {
        selected.push(root);
    }
    if let Some(other) = worst_other {
        selected.push(other);
    }
    if selected.is_empty() {
        if let Some(worst) = entries.iter().max_by(|left, right| {
            left.used_percent
                .partial_cmp(&right.used_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            selected.push(worst);
        }
    }
    selected
}

fn format_disk_bar(entries: &[DiskSummary]) -> String {
    disk_bar_entries(entries)
        .iter()
        .map(|entry| {
            format!(
                "{} {:.0}%",
                compact_disk_name(&entry.name, 8),
                entry.used_percent
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn format_disk_compact(entries: &[DiskSummary]) -> String {
    disk_bar_entries(entries)
        .into_iter()
        .max_by(|left, right| {
            left.used_percent
                .partial_cmp(&right.used_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|entry| {
            format!(
                "{} {:.0}%",
                compact_disk_name(&entry.name, 12),
                entry.used_percent
            )
        })
        .unwrap_or_else(|| "n/a".into())
}

fn format_disk_tiny(entries: &[DiskSummary]) -> String {
    entries
        .iter()
        .map(|entry| entry.used_percent)
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
        .map(|used| format!("{used:.0}%"))
        .unwrap_or_else(|| "n/a".into())
}

fn search_text_matches(haystack: &str, query: &str) -> bool {
    let haystack = haystack.to_lowercase();
    query
        .split_whitespace()
        .all(|term| haystack.contains(&term.to_lowercase()))
}

fn unique_custom_module_name(modules: &[CustomModule], base: &str) -> String {
    let base = if base.trim().is_empty() {
        "module".to_string()
    } else {
        base.trim().to_string()
    };
    if !modules.iter().any(|module| module.name == base) {
        return base;
    }
    for suffix in 2..=999 {
        let candidate = format!("{base} {suffix}");
        if !modules.iter().any(|module| module.name == candidate) {
            return candidate;
        }
    }
    format!("{base} copy")
}

fn request_custom_module_run(active: &Active, module_name: &str) -> bool {
    let slot_id = format!("custom:{module_name}");
    for slot in active.borrow().iter() {
        if slot.id == slot_id {
            if let Some(force_run) = &slot.force_run {
                force_run.set(true);
                return true;
            }
            return false;
        }
    }
    false
}

/// Sends a desktop notification for `key`, at most once every 60 seconds.
fn maybe_notify(key: &str, active: bool, title: &str, body: &str) {
    if !active {
        return;
    }
    ALERTS.with(|m| {
        let mut m = m.borrow_mut();
        let now = Instant::now();
        let ok = m
            .get(key)
            .map(|t| now.duration_since(*t).as_secs() >= 60)
            .unwrap_or(true);
        if ok {
            m.insert(key.to_string(), now);
            let title = title.to_string();
            let body = body.to_string();
            std::thread::spawn(move || {
                let _ = run_program_limited(
                    "notify-send",
                    &[&title, &body],
                    CommandPolicy {
                        timeout: Duration::from_secs(2),
                        max_output_bytes: 1_024,
                    },
                );
            });
        }
    });
}

/// Applies `.warn`/`.crit` styling for a "higher is worse" metric.
/// `warn_o`/`crit_o` are the user's per-metric overrides (0 = use `dw`/`dc`).
#[allow(clippy::too_many_arguments)]
fn apply_level(
    label: &Label,
    v: f64,
    warn_o: f64,
    crit_o: f64,
    dw: f64,
    dc: f64,
    notify: bool,
    key: &str,
    title: &str,
) {
    let w = if warn_o > 0.0 { warn_o } else { dw };
    let c = if crit_o > 0.0 { crit_o } else { dc };
    if v >= c {
        label.add_css_class("crit");
        maybe_notify(key, notify, title, &format!("{title} = {v:.0} (critical!)"));
    } else if v >= w {
        label.add_css_class("warn");
    }
}

/// Same as `apply_level` but for a "lower is worse" metric (e.g. battery).
#[allow(clippy::too_many_arguments)]
fn apply_level_low(
    label: &Label,
    v: f64,
    warn_o: f64,
    crit_o: f64,
    dw: f64,
    dc: f64,
    notify: bool,
    key: &str,
    title: &str,
) {
    let w = if warn_o > 0.0 { warn_o } else { dw };
    let c = if crit_o > 0.0 { crit_o } else { dc };
    if v <= c {
        label.add_css_class("crit");
        maybe_notify(key, notify, title, &format!("{title} = {v:.0} (critical!)"));
    } else if v <= w {
        label.add_css_class("warn");
    }
}

fn external_metric_changed(
    id: &str,
    current: &ExternalSnapshot,
    previous: Option<&ExternalSnapshot>,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    match id {
        "wifi" => current.wifi != previous.wifi,
        "pkg_updates" => current.packages != previous.packages,
        "media" => current.media != previous.media,
        "bluetooth" => current.bluetooth != previous.bluetooth,
        "volume" => current.volume != previous.volume,
        "brightness" => current.brightness != previous.brightness,
        _ => false,
    }
}

fn update_slot(
    slot: &Slot,
    core: &CoreSnapshot,
    external: &ExternalSnapshot,
    notify: bool,
    core_changed: bool,
    external_changed: bool,
    layout_changed: bool,
) {
    match &slot.widget {
        SlotWidget::Graph { area, hist } => {
            if !core_changed {
                return;
            }
            let value = core.graph_value(&slot.id).unwrap_or(0.0);
            let (current, minimum, maximum, average) = {
                let mut history = hist.borrow_mut();
                if history.len() >= 64 {
                    history.pop_front();
                }
                history.push_back(value);
                let minimum = history.iter().cloned().fold(f64::INFINITY, f64::min);
                let maximum = history.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let average = history.iter().sum::<f64>() / history.len() as f64;
                (value, minimum, maximum, average)
            };
            let tooltip = if slot.id == "net_graph" {
                format!(
                    "Download now {} · avg {} · min {} · max {}",
                    human_rate(current as u64),
                    human_rate(average as u64),
                    human_rate(minimum as u64),
                    human_rate(maximum as u64)
                )
            } else {
                format!(
                    "Now {current:.0}% · avg {average:.0}% · min {minimum:.0}% · max {maximum:.0}%"
                )
            };
            area.set_tooltip_text(Some(&tooltip));
            area.queue_draw();
            let overflow_value = if slot.id == "net_graph" {
                human_rate(current as u64)
            } else {
                format!("{current:.0}%")
            };
            slot.layout.overflow_value.set_text(&overflow_value);
        }
        SlotWidget::Text(label) => {
            if !slot.source_cmd.is_empty() {
                let format = if slot.layout.tier.get() == LayoutTier::Tiny {
                    ""
                } else {
                    &slot.format
                };
                if !slot.trusted {
                    set_formatted(label, &slot.prefix, format, "blocked");
                    label.set_tooltip_text(Some("Command is disabled until explicitly trusted"));
                    slot.layout.overflow_value.set_text("blocked");
                    return;
                }
                if layout_changed {
                    if let Some(value) = slot.last_value.borrow().as_deref() {
                        set_formatted(label, &slot.prefix, format, value);
                        slot.layout.overflow_value.set_text(label.text().as_str());
                    }
                }
                if let (Some(sender), Some(receiver), Some(busy)) =
                    (&slot.cmd_tx, &slot.cmd_rx, &slot.busy)
                {
                    while let Ok(result) = receiver.try_recv() {
                        let display = result.display_text();
                        set_formatted(label, &slot.prefix, format, &display);
                        label.set_tooltip_text(Some(&format!(
                            "Last run: {:.0} ms · exit {}{}{}",
                            result.duration.as_secs_f64() * 1_000.0,
                            result
                                .exit_code
                                .map(|code| code.to_string())
                                .unwrap_or_else(|| "n/a".into()),
                            if result.timed_out {
                                " · timed out"
                            } else {
                                ""
                            },
                            if result.truncated {
                                " · output truncated"
                            } else {
                                ""
                            },
                        )));
                        slot.layout.overflow_value.set_text(label.text().as_str());
                        *slot.last_value.borrow_mut() = Some(display);
                        busy.set(false);
                    }
                    let due = slot
                        .last_started
                        .borrow()
                        .map(|started| started.elapsed() >= Duration::from_millis(slot.interval_ms))
                        .unwrap_or(true);
                    let forced = slot
                        .force_run
                        .as_ref()
                        .map(|force_run| force_run.get())
                        .unwrap_or(false);
                    if (due || forced) && !busy.get() {
                        if let Some(force_run) = &slot.force_run {
                            force_run.set(false);
                        }
                        busy.set(true);
                        *slot.last_started.borrow_mut() = Some(Instant::now());
                        let sender = sender.clone();
                        let command = slot.source_cmd.clone();
                        let policy = CommandPolicy {
                            timeout: Duration::from_millis(slot.timeout_ms),
                            max_output_bytes: slot.max_output_bytes,
                        };
                        std::thread::spawn(move || {
                            let _ = sender.send(run_shell_limited(&command, policy));
                        });
                    }
                }
                return;
            }
            let needs_update = match slot.id.as_str() {
                "clock" | "date" => true,
                "wifi" | "pkg_updates" | "media" | "bluetooth" => {
                    external_changed || layout_changed
                }
                _ => core_changed || layout_changed,
            };
            if needs_update {
                update_metric(
                    &slot.id,
                    &slot.prefix,
                    &slot.format,
                    label,
                    core,
                    external,
                    notify,
                    slot.warn,
                    slot.crit,
                    slot.layout.tier.get(),
                );
                slot.layout.overflow_value.set_text(label.text().as_str());
            }
        }
        SlotWidget::Range {
            scale,
            suppress,
            icon,
        } => {
            if !external_changed && !layout_changed {
                return;
            }
            let value = match slot.id.as_str() {
                "volume" => external.volume.map(|(value, _)| value as f64),
                "brightness" => external.brightness.map(|value| value as f64),
                _ => None,
            };
            if let Some(value) = value {
                if !scale.has_css_class("dragging") {
                    suppress.set(true);
                    scale.set_value(value);
                    suppress.set(false);
                }
                slot.layout.overflow_value.set_text(&format!("{value:.0}%"));
            } else {
                slot.layout.overflow_value.set_text("n/a");
            }
            if slot.id == "volume" {
                if let (Some(button), Some((value, muted))) = (icon, external.volume) {
                    button.set_label(if muted { "Muted" } else { "Audio" });
                    button.set_tooltip_text(Some(&format!(
                        "{}% · {} · click to toggle mute",
                        value,
                        if muted { "muted" } else { "active" }
                    )));
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_core_sample(
    label: &Label,
    prefix: &str,
    format: &str,
    sample: &MetricSample,
    notify: bool,
    warn_override: f64,
    crit_override: f64,
    tier: LayoutTier,
) {
    let value = match tier {
        LayoutTier::Compact => sample
            .compact_value
            .as_deref()
            .unwrap_or(sample.value.as_str()),
        LayoutTier::Tiny => sample
            .tiny_value
            .as_deref()
            .unwrap_or(sample.value.as_str()),
        LayoutTier::Normal | LayoutTier::Overflow => sample.value.as_str(),
    };
    let format = if tier == LayoutTier::Tiny { "" } else { format };
    set_formatted(label, prefix, format, value);
    label.set_tooltip_text(sample.tooltip.as_deref());
    if sample.dimmed {
        label.add_css_class("dim");
    }
    match &sample.level {
        MetricLevel::None => {}
        MetricLevel::Higher {
            value,
            default_warn,
            default_crit,
            key,
            title,
        } => apply_level(
            label,
            *value,
            warn_override,
            crit_override,
            *default_warn,
            *default_crit,
            notify,
            key,
            title,
        ),
        MetricLevel::Lower {
            value,
            default_warn,
            default_crit,
            key,
            title,
        } => apply_level_low(
            label,
            *value,
            warn_override,
            crit_override,
            *default_warn,
            *default_crit,
            notify,
            key,
            title,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn update_metric(
    id: &str,
    prefix: &str,
    format: &str,
    label: &Label,
    core: &CoreSnapshot,
    external: &ExternalSnapshot,
    notify: bool,
    warn_override: f64,
    crit_override: f64,
    tier: LayoutTier,
) {
    let format = if tier == LayoutTier::Tiny { "" } else { format };
    label.remove_css_class("warn");
    label.remove_css_class("crit");
    label.remove_css_class("dim");
    match id {
        "clock" => {
            if let Ok(date_time) = glib::DateTime::now_local() {
                if let Ok(value) = date_time.format("%H:%M:%S") {
                    set_formatted(label, prefix, format, value.as_str());
                }
            }
        }
        "date" => {
            if let Ok(date_time) = glib::DateTime::now_local() {
                if let Ok(value) = date_time.format("%a %d %b") {
                    set_formatted(label, prefix, format, value.as_str());
                }
            }
        }
        "wifi" => match &external.wifi {
            Some((ssid, level)) => {
                set_formatted(label, prefix, format, &format!("{ssid} {level}dBm"));
                label.set_tooltip_text(Some(&format!("SSID: {ssid}\nSignal: {level} dBm")));
            }
            None => {
                set_formatted(label, prefix, format, "n/a");
                label.set_tooltip_text(None);
            }
        },
        "pkg_updates" => match &external.packages {
            Some(packages) => {
                set_formatted(label, prefix, format, &packages.total().to_string());
                label.set_tooltip_text(Some(&format!(
                    "Pending package updates (refreshed every 20 minutes)\n{}",
                    packages.tooltip()
                )));
            }
            None => {
                set_formatted(label, prefix, format, "n/a");
                label.set_tooltip_text(None);
            }
        },
        "media" => match &external.media {
            Some(media) => {
                let state = match media.status.as_str() {
                    "Playing" => "Play",
                    "Paused" => "Pause",
                    _ => "Stop",
                };
                let text = if media.artist.is_empty() {
                    media.title.clone()
                } else {
                    format!("{} \u{2014} {}", media.artist, media.title)
                };
                set_formatted(label, prefix, format, &format!("{state} {text}"));
                label.set_tooltip_text(Some(&format!("Status: {}\n{text}", media.status)));
            }
            None => {
                set_formatted(label, prefix, format, "n/a");
                label.set_tooltip_text(None);
            }
        },
        "bluetooth" => match &external.bluetooth {
            Some((powered, devices)) => {
                if !powered {
                    set_formatted(label, prefix, format, "off");
                    label.set_tooltip_text(Some("Bluetooth adapter is powered off"));
                } else if devices.is_empty() {
                    set_formatted(label, prefix, format, "on");
                    label.set_tooltip_text(Some("Bluetooth is on, no devices connected"));
                } else {
                    set_formatted(
                        label,
                        prefix,
                        format,
                        &format!("{} connected", devices.len()),
                    );
                    label.set_tooltip_text(Some(&format!("Connected: {}", devices.join(", "))));
                }
            }
            None => {
                set_formatted(label, prefix, format, "n/a");
                label.set_tooltip_text(None);
            }
        },
        _ => {
            if let Some(sample) = core.sample(id) {
                apply_core_sample(
                    label,
                    prefix,
                    format,
                    sample,
                    notify,
                    warn_override,
                    crit_override,
                    tier,
                );
            }
        }
    }
}

// ---------- sysfs / hwmon helpers ----------

fn read_fan_rpm() -> Option<u32> {
    let mut max = 0u32;
    for entry in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let base = entry.path();
        for i in 1..=8 {
            if let Ok(s) = std::fs::read_to_string(base.join(format!("fan{i}_input"))) {
                if let Ok(v) = s.trim().parse::<u32>() {
                    if v > max {
                        max = v;
                    }
                }
            }
        }
    }
    (max > 0).then_some(max)
}

fn read_battery() -> Option<(u8, String)> {
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let p = entry.path();
        if let Ok(t) = std::fs::read_to_string(p.join("type")) {
            if t.trim() == "Battery" {
                if let Ok(c) = std::fs::read_to_string(p.join("capacity")) {
                    if let Ok(cap) = c.trim().parse::<u8>() {
                        let status = std::fs::read_to_string(p.join("status"))
                            .map(|s| s.trim().to_string())
                            .unwrap_or_default();
                        return Some((cap, status));
                    }
                }
            }
        }
    }
    None
}

fn read_amd_gpu(index: u32) -> Option<(u32, Option<u32>)> {
    let dev = amd_card_device(index)?;
    let busy = std::fs::read_to_string(dev.join("gpu_busy_percent"))
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()?;
    Some((
        busy,
        amd_hwmon_read(&dev, "temp1_input").map(|v| (v / 1000.0) as u32),
    ))
}

/// Returns the Nth (0-based) AMD GPU device directory, for multi-GPU setups.
fn amd_card_device(index: u32) -> Option<PathBuf> {
    let mut seen = 0u32;
    for entry in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("card") && !name.contains('-') {
            let dev = entry.path().join("device");
            if dev.join("gpu_busy_percent").exists() {
                if seen == index {
                    return Some(dev);
                }
                seen += 1;
            }
        }
    }
    None
}

fn amd_hwmon_read(dev: &Path, file: &str) -> Option<f64> {
    for e in std::fs::read_dir(dev.join("hwmon")).ok()?.flatten() {
        if let Ok(s) = std::fs::read_to_string(e.path().join(file)) {
            if let Ok(v) = s.trim().parse::<f64>() {
                return Some(v);
            }
        }
    }
    None
}

fn amd_active_clock(dev: &Path, file: &str) -> Option<u32> {
    let s = std::fs::read_to_string(dev.join(file)).ok()?;
    for line in s.lines() {
        if line.trim_end().ends_with('*') {
            for tok in line.split_whitespace() {
                let t = tok.to_lowercase();
                if t.ends_with("mhz") {
                    let num: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(v) = num.parse::<u32>() {
                        return Some(v);
                    }
                }
            }
        }
    }
    None
}

/// Best-effort Intel iGPU clock reading. Intel does not expose a generic
/// utilization or power sysfs file the way AMD does, so only the current
/// GT frequency is reported here; `gpu`/`gpu_power`/`gpu_fan` remain n/a
/// on Intel-only systems without the external `intel_gpu_top` tool.
fn read_intel_gpu_clock(index: u32) -> Option<u32> {
    let mut seen = 0u32;
    for entry in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("card") && !name.contains('-')) {
            continue;
        }
        let dev_dir = entry.path();
        let device = dev_dir.join("device");
        let Ok(vendor) = std::fs::read_to_string(device.join("vendor")) else {
            continue;
        };
        if vendor.trim() != "0x8086" {
            continue;
        }
        if seen != index {
            seen += 1;
            continue;
        }
        for rel in [
            "gt/gt0/gt_act_freq_mhz",
            "gt_act_freq_mhz",
            "gt_cur_freq_mhz",
        ] {
            if let Ok(s) = std::fs::read_to_string(dev_dir.join(rel)) {
                if let Ok(v) = s.trim().parse::<u32>() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn read_cpu_power() -> Option<f64> {
    let e = std::fs::read_to_string("/sys/class/powercap/intel-rapl:0/energy_uj")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    RAPL_PREV.with(|p| {
        let mut prev = p.borrow_mut();
        let dt = prev.1.elapsed().as_secs_f64().max(0.001);
        let dj = e.saturating_sub(prev.0) as f64 / 1e6;
        let first = prev.0 == 0;
        *prev = (e, Instant::now());
        if first {
            None
        } else {
            Some(dj / dt)
        }
    })
}

fn read_vcore() -> Option<f64> {
    for e in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let base = e.path();
        for i in 0..=12 {
            if let Ok(lbl) = std::fs::read_to_string(base.join(format!("in{i}_label"))) {
                if lbl.trim().eq_ignore_ascii_case("vcore") {
                    if let Ok(v) = std::fs::read_to_string(base.join(format!("in{i}_input"))) {
                        if let Ok(mv) = v.trim().parse::<f64>() {
                            return Some(mv / 1000.0);
                        }
                    }
                }
            }
        }
    }
    None
}

fn read_proc_count() -> usize {
    std::fs::read_dir("/proc")
        .map(|d| {
            d.flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .chars()
                        .all(|c| c.is_ascii_digit())
                })
                .count()
        })
        .unwrap_or(0)
}

/// Per-device disk I/O throughput (bytes/sec) for every whole physical disk
/// found in /proc/diskstats. Partitions are skipped so a disk with several
/// partitions is only counted once. Rates are computed from the delta
/// against the previous reading of that same device, so a brand-new device
/// (first time seen) reports 0 until the next poll.
fn read_disk_io_breakdown() -> Vec<(String, f64, f64)> {
    let Ok(content) = std::fs::read_to_string("/proc/diskstats") else {
        return Vec::new();
    };
    let mut raw: Vec<(String, u64, u64)> = Vec::new();
    for line in content.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 {
            continue;
        }
        let name = f[2];
        let whole = if name.starts_with("nvme") || name.starts_with("mmcblk") {
            !name.contains('p')
        } else if name.starts_with("sd") || name.starts_with("vd") || name.starts_with("hd") {
            !name
                .chars()
                .last()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(true)
        } else {
            false
        };
        if !whole {
            continue;
        }
        let rd = f[5].parse::<u64>().unwrap_or(0);
        let wr = f[9].parse::<u64>().unwrap_or(0);
        raw.push((name.to_string(), rd, wr));
    }
    DISK_PREV.with(|p| {
        let mut prev = p.borrow_mut();
        let now = Instant::now();
        let mut out = Vec::new();
        for (name, rd, wr) in raw {
            let (dr, dw) = match prev.get(&name) {
                Some((pr, pw, pt)) => {
                    let dt = pt.elapsed().as_secs_f64().max(0.001);
                    (
                        rd.saturating_sub(*pr) as f64 * 512.0 / dt,
                        wr.saturating_sub(*pw) as f64 * 512.0 / dt,
                    )
                }
                None => (0.0, 0.0),
            };
            prev.insert(name.clone(), (rd, wr, now));
            out.push((name, dr, dw));
        }
        out
    })
}

/// Best-effort disk temperature lookup via the hwmon drivers that expose it
/// (`nvme` for NVMe SSDs, `drivetemp` for SATA/USB spinning and SSD drives).
/// This cannot be perfectly correlated to one specific mount point, so all
/// sensors found are listed together in the disk metric's tooltip.
fn disk_temps() -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/hwmon") else {
        return out;
    };
    for e in dir.flatten() {
        let base = e.path();
        let Ok(chip) = std::fs::read_to_string(base.join("name")) else {
            continue;
        };
        let chip = chip.trim().to_lowercase();
        if !(chip.contains("nvme") || chip.contains("drivetemp")) {
            continue;
        }
        for i in 1..=4 {
            if let Ok(s) = std::fs::read_to_string(base.join(format!("temp{i}_input"))) {
                if let Ok(v) = s.trim().parse::<f64>() {
                    let label = std::fs::read_to_string(base.join(format!("temp{i}_label")))
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|_| chip.clone());
                    out.push((label, v / 1000.0));
                }
            }
        }
    }
    out
}

/// Creates a small horizontal Scale (0-100) sized to fit the topbar, used
/// for the Volume and Brightness slider metrics. Returns the scale plus a
/// suppression flag so periodic value syncs from the OS don't re-trigger
/// the user-drag handler and fight over the value.
fn make_range() -> (Scale, Rc<Cell<bool>>) {
    let scale = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_size_request(80, -1);
    scale.set_draw_value(false);
    scale.set_hexpand(false);
    let gesture = GestureClick::new();
    gesture.set_button(1);
    let pressed = scale.clone();
    gesture.connect_pressed(move |_, _, _, _| pressed.add_css_class("dragging"));
    let released = scale.clone();
    gesture.connect_released(move |_, _, _, _| released.remove_css_class("dragging"));
    scale.add_controller(gesture);
    (scale, Rc::new(Cell::new(false)))
}

fn human_rate(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1_048_576.0 {
        format!("{:.1}MB/s", b / 1_048_576.0)
    } else {
        format!("{:.0}KB/s", b / 1024.0)
    }
}
fn human_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.1}GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0}MB", b / 1e6)
    } else {
        format!("{:.0}KB", b / 1e3)
    }
}
fn human_uptime(secs: u64) -> String {
    format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
}

/// Path to the CSV history log, `~/.config/topmonitoring/history.csv`.
fn history_log_path() -> PathBuf {
    let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("topmonitoring");
    let _ = std::fs::create_dir_all(&p);
    p.push("history.csv");
    p
}

#[derive(Clone, Debug, Default)]
struct HistorySummary {
    sample_count: usize,
    first_timestamp: String,
    last_timestamp: String,
    cpu_average: f64,
    cpu_maximum: f64,
    memory_average: f64,
    memory_maximum: f64,
    temperature_average: f64,
    temperature_maximum: f64,
    samples: Vec<(f64, f64, f64)>,
}

fn summarize_history(raw: &str) -> HistorySummary {
    let mut summary = HistorySummary::default();
    let mut recent_samples = VecDeque::with_capacity(180);
    let mut cpu_total = 0.0;
    let mut memory_total = 0.0;
    let mut temperature_total = 0.0;
    for line in raw.lines() {
        let mut fields = line.splitn(4, ',');
        let timestamp = fields.next().unwrap_or_default().trim();
        let Some(cpu) = fields
            .next()
            .and_then(|value| value.trim().parse::<f64>().ok())
        else {
            continue;
        };
        let Some(memory) = fields
            .next()
            .and_then(|value| value.trim().parse::<f64>().ok())
        else {
            continue;
        };
        let Some(temperature) = fields
            .next()
            .and_then(|value| value.trim().parse::<f64>().ok())
        else {
            continue;
        };
        if timestamp.is_empty()
            || !cpu.is_finite()
            || !memory.is_finite()
            || !temperature.is_finite()
        {
            continue;
        }
        if summary.sample_count == 0 {
            summary.first_timestamp = timestamp.to_string();
        }
        summary.last_timestamp = timestamp.to_string();
        summary.sample_count += 1;
        cpu_total += cpu;
        memory_total += memory;
        temperature_total += temperature;
        summary.cpu_maximum = summary.cpu_maximum.max(cpu);
        summary.memory_maximum = summary.memory_maximum.max(memory);
        summary.temperature_maximum = summary.temperature_maximum.max(temperature);
        if recent_samples.len() >= 180 {
            recent_samples.pop_front();
        }
        recent_samples.push_back((cpu, memory, temperature));
    }
    summary.samples = recent_samples.into_iter().collect();
    if summary.sample_count > 0 {
        let count = summary.sample_count as f64;
        summary.cpu_average = cpu_total / count;
        summary.memory_average = memory_total / count;
        summary.temperature_average = temperature_total / count;
    }
    summary
}

fn history_summary_text(summary: &HistorySummary) -> String {
    if summary.sample_count == 0 {
        return "No history samples yet.".into();
    }
    format!(
        "{} samples · {} → {}\nCPU avg {:.1}% / max {:.1}% · RAM avg {:.1}% / max {:.1}% · Temp avg {:.1}°C / max {:.1}°C",
        summary.sample_count,
        summary.first_timestamp,
        summary.last_timestamp,
        summary.cpu_average,
        summary.cpu_maximum,
        summary.memory_average,
        summary.memory_maximum,
        summary.temperature_average,
        summary.temperature_maximum,
    )
}

fn open_history_dashboard(runtime: RuntimeServices) {
    let window = Window::builder()
        .title("TopMonitoring — History dashboard")
        .default_width(760)
        .default_height(430)
        .build();
    let shell = GtkBox::new(Orientation::Vertical, 10);
    shell.set_margin_top(14);
    shell.set_margin_bottom(14);
    shell.set_margin_start(14);
    shell.set_margin_end(14);

    let summary_label = Label::new(None);
    summary_label.set_xalign(0.0);
    summary_label.set_wrap(true);
    shell.append(&summary_label);

    let legend = Label::new(Some(
        "CPU — accent · RAM — warning · Temperature — critical",
    ));
    legend.add_css_class("settings-caption");
    legend.set_xalign(0.0);
    shell.append(&legend);

    let chart = DrawingArea::new();
    chart.set_content_width(720);
    chart.set_content_height(250);
    chart.set_hexpand(true);
    chart.set_vexpand(true);
    let chart_samples: Rc<RefCell<Vec<(f64, f64, f64)>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let samples = chart_samples.clone();
        chart.set_draw_func(move |_, context, width, height| {
            let width = width as f64;
            let height = height as f64;
            context.set_source_rgba(0.08, 0.09, 0.11, 0.96);
            let _ = context.paint();
            context.set_line_width(1.0);
            context.set_source_rgba(1.0, 1.0, 1.0, 0.10);
            for step in 0..=4 {
                let y = height * step as f64 / 4.0;
                context.move_to(0.0, y);
                context.line_to(width, y);
            }
            let _ = context.stroke();
            let values = samples.borrow();
            if values.len() < 2 {
                return;
            }
            let draw_line = |channel: usize, red: f64, green: f64, blue: f64| {
                context.set_source_rgba(red, green, blue, 0.95);
                context.set_line_width(2.0);
                for (index, sample) in values.iter().enumerate() {
                    let raw = match channel {
                        0 => sample.0,
                        1 => sample.1,
                        _ => sample.2,
                    };
                    let ceiling = if channel == 2 { 120.0 } else { 100.0 };
                    let value = raw.clamp(0.0, ceiling) / ceiling;
                    let x = index as f64 * width / (values.len() - 1) as f64;
                    let y = height - value * height;
                    if index == 0 {
                        context.move_to(x, y);
                    } else {
                        context.line_to(x, y);
                    }
                }
                let _ = context.stroke();
            };
            draw_line(0, 0.49, 0.83, 0.98);
            draw_line(1, 0.98, 0.75, 0.14);
            draw_line(2, 0.98, 0.44, 0.53);
        });
    }
    shell.append(&chart);

    let status = Label::new(None);
    status.add_css_class("settings-caption");
    status.set_xalign(0.0);
    shell.append(&status);

    let (history_sender, history_receiver) =
        mpsc::channel::<(u64, Result<(HistorySummary, PathBuf), String>)>();
    let history_generation = Rc::new(Cell::new(0_u64));
    let refresh: Rc<dyn Fn()> = {
        let sender = history_sender.clone();
        let generation = history_generation.clone();
        let status = status.clone();
        Rc::new(move || {
            let request_generation = generation.get().wrapping_add(1);
            generation.set(request_generation);
            status.set_text("Loading history…");
            std::thread::spawn({
                let sender = sender.clone();
                move || {
                    let path = history_log_path();
                    let result = match std::fs::read_to_string(&path) {
                        Ok(raw) => Ok((summarize_history(&raw), path)),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            Ok((HistorySummary::default(), path))
                        }
                        Err(error) => Err(format!("Could not read history: {error}")),
                    };
                    let _ = sender.send((request_generation, result));
                }
            });
        })
    };
    {
        let window = window.clone();
        let generation = history_generation.clone();
        let summary_label = summary_label.clone();
        let status = status.clone();
        let samples = chart_samples.clone();
        let chart = chart.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            while let Ok((result_generation, result)) = history_receiver.try_recv() {
                if result_generation != generation.get() {
                    continue;
                }
                match result {
                    Ok((summary, path)) => {
                        summary_label.set_text(&history_summary_text(&summary));
                        *samples.borrow_mut() = summary.samples;
                        if summary.sample_count == 0 {
                            status.set_text("Enable history recording to collect samples.");
                        } else {
                            status.set_text(&format!("Source: {}", path.display()));
                        }
                        chart.queue_draw();
                    }
                    Err(error) => status.set_text(&error),
                }
            }
            if window.is_visible() {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }
    refresh();

    let actions = GtkBox::new(Orientation::Horizontal, 8);
    let refresh_button = Button::with_label("Refresh");
    {
        let refresh = refresh.clone();
        refresh_button.connect_clicked(move |_| refresh());
    }
    let open_folder = Button::with_label("Open folder");
    {
        let service = runtime.clone();
        open_folder.connect_clicked(move |_| {
            if let Some(directory) = history_log_path().parent() {
                service.open_path(directory.to_path_buf());
            }
        });
    }
    let clear = Button::with_label("Clear history");
    let armed = Rc::new(Cell::new(false));
    {
        let armed = armed.clone();
        let refresh = refresh.clone();
        let status = status.clone();
        let clear_button = clear.clone();
        clear.connect_clicked(move |_| {
            if !armed.replace(true) {
                clear_button.set_label("Confirm clear");
                status.set_text("Click Confirm clear to delete current and rotated history.");
                let armed = armed.clone();
                let clear_button = clear_button.clone();
                glib::timeout_add_local(Duration::from_secs(4), move || {
                    armed.set(false);
                    clear_button.set_label("Clear history");
                    glib::ControlFlow::Break
                });
                return;
            }
            armed.set(false);
            let path = history_log_path();
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("csv.1"));
            clear_button.set_label("Clear history");
            status.set_text("History cleared.");
            refresh();
        });
    }
    let close = Button::with_label("Close");
    {
        let window = window.clone();
        close.connect_clicked(move |_| window.close());
    }
    actions.append(&refresh_button);
    actions.append(&open_folder);
    actions.append(&clear);
    actions.append(&close);
    shell.append(&actions);
    window.set_child(Some(&shell));
    window.present();
}

/// Queues one history row without blocking the GTK main loop.
fn append_history_values_async(
    cpu: f32,
    memory_percent: f64,
    temperature: f32,
    max_megabytes: u32,
) {
    let timestamp = glib::DateTime::now_local()
        .ok()
        .and_then(|date| date.format("%Y-%m-%d %H:%M:%S").ok())
        .map(|value| value.to_string())
        .unwrap_or_default();
    std::thread::spawn(move || {
        let path = history_log_path();
        let max_bytes = u64::from(max_megabytes.clamp(1, 1_024)) * 1_048_576;
        if path
            .metadata()
            .map(|metadata| metadata.len() >= max_bytes)
            .unwrap_or(false)
        {
            let rotated = path.with_extension("csv.1");
            let _ = std::fs::remove_file(&rotated);
            if let Err(error) = std::fs::rename(&path, &rotated) {
                eprintln!("[TopMonitoring] History rotation failed: {error}");
            }
        }
        let is_new = !path.exists();
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                if is_new {
                    let _ = writeln!(file, "timestamp,cpu_percent,mem_percent,temp_c");
                }
                if let Err(error) = writeln!(
                    file,
                    "{timestamp},{cpu:.1},{memory_percent:.1},{temperature:.1}"
                ) {
                    eprintln!("[TopMonitoring] History write failed: {error}");
                }
            }
            Err(error) => eprintln!(
                "[TopMonitoring] Could not open history log {}: {error}",
                path.display()
            ),
        }
    });
}

/// Writes an autostart `.desktop` entry pointing at the currently running
/// executable. The path is quoted so it works even when the binary lives
/// somewhere with spaces in the path.
fn install_autostart() -> bool {
    if let (Ok(exe), Some(dir)) = (std::env::current_exe(), dirs::config_dir()) {
        let adir = dir.join("autostart");
        if std::fs::create_dir_all(&adir).is_err() {
            return false;
        }
        let escaped_executable = exe
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let content = format!(
            "[Desktop Entry]\nType=Application\nVersion=1.0\nName=TopMonitoring\nExec=\"{escaped_executable}\"\nIcon=io.github.fakedevbagus.TopMonitoring\nTerminal=false\nStartupWMClass=io.github.fakedevbagus.TopMonitoring\nX-KDE-autostart-phase=2\nX-GNOME-Autostart-enabled=true\n"
        );
        return std::fs::write(
            adir.join("io.github.fakedevbagus.TopMonitoring.desktop"),
            content,
        )
        .is_ok();
    }
    false
}

// ---------- Hardware Sensors window ----------

#[derive(Clone, Debug, PartialEq, Eq)]
struct HwSensor {
    chip: String,
    label: String,
    value: String,
}

/// Enumerates every readable channel under every `/sys/class/hwmon/hwmon*`
/// entry: temperatures, fan speeds, voltages, currents, and power.
fn scan_hwmon() -> Vec<HwSensor> {
    let mut out = Vec::new();
    let dir = match std::fs::read_dir("/sys/class/hwmon") {
        Ok(d) => d,
        Err(_) => return out,
    };
    for e in dir.flatten() {
        let base = e.path();
        let chip = std::fs::read_to_string(base.join("name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "hwmon".into());
        let specs: [(&str, &str, f64); 5] = [
            ("temp", "\u{b0}C", 1000.0),
            ("fan", "rpm", 1.0),
            ("in", "V", 1000.0),
            ("curr", "A", 1000.0),
            ("power", "W", 1_000_000.0),
        ];
        for (pfx, unit, div) in specs {
            for i in 0..=24 {
                let mut raw = std::fs::read_to_string(base.join(format!("{pfx}{i}_input"))).ok();
                if raw.is_none() && pfx == "power" {
                    raw = std::fs::read_to_string(base.join(format!("{pfx}{i}_average"))).ok();
                }
                let Some(raw) = raw else { continue };
                let Ok(val) = raw.trim().parse::<f64>() else {
                    continue;
                };
                let label = std::fs::read_to_string(base.join(format!("{pfx}{i}_label")))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| format!("{pfx}{i}"));
                let v = val / div;
                let value = if unit == "rpm" {
                    format!("{v:.0} {unit}")
                } else {
                    format!("{v:.2} {unit}")
                };
                out.push(HwSensor {
                    chip: chip.clone(),
                    label,
                    value,
                });
            }
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
struct ProcessSnapshotRow {
    name: String,
    cpu_usage: f32,
    memory_mb: f64,
}

fn sort_process_rows(rows: &mut [ProcessSnapshotRow], by_memory: bool) {
    if by_memory {
        rows.sort_by(|left, right| right.memory_mb.total_cmp(&left.memory_mb));
    } else {
        rows.sort_by(|left, right| right.cpu_usage.total_cmp(&left.cpu_usage));
    }
}

fn render_process_rows(content: &GtkBox, rows: &[ProcessSnapshotRow], by_memory: bool) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    let mut rows = rows.to_vec();
    sort_process_rows(&mut rows, by_memory);
    if rows.is_empty() {
        let empty = Label::new(Some("No processes available."));
        empty.set_xalign(0.0);
        content.append(&empty);
        return;
    }
    for process in rows.iter().take(20) {
        let row = Label::new(None);
        row.set_markup(&format!(
            "<tt>{:<24} CPU {:>5.1}%  RAM {:>7.1} MB</tt>",
            glib::markup_escape_text(&process.name),
            process.cpu_usage,
            process.memory_mb
        ));
        row.set_xalign(0.0);
        content.append(&row);
    }
}

/// A small snapshot-backed task manager. Sysinfo ownership and refresh stay on
/// a worker; GTK only sorts owned rows and reconciles labels.
fn open_process_manager() {
    let win = Window::builder()
        .title("Process Manager")
        .default_width(480)
        .default_height(560)
        .build();
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    let header = GtkBox::new(Orientation::Horizontal, 6);
    header.set_margin_top(8);
    header.set_margin_start(12);
    header.set_margin_end(12);
    let sort_dd = DropDown::from_strings(&["Sort by CPU", "Sort by RAM"]);
    header.append(&Label::new(Some("Top processes:")));
    header.append(&sort_dd);
    vbox.append(&header);
    let content = GtkBox::new(Orientation::Vertical, 2);
    content.set_margin_top(8);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&Label::new(Some("Collecting process snapshot…")));
    let scroll = ScrolledWindow::builder()
        .vexpand(true)
        .child(&content)
        .build();
    vbox.append(&scroll);
    let close = Button::with_label("Close");
    close.set_margin_top(6);
    close.set_margin_bottom(6);
    vbox.append(&close);
    win.set_child(Some(&vbox));

    let (sender, receiver) = mpsc::sync_channel::<Vec<ProcessSnapshotRow>>(1);
    std::thread::Builder::new()
        .name("topmonitoring-processes".into())
        .spawn(move || {
            let mut system = System::new_all();
            loop {
                system.refresh_processes(ProcessesToUpdate::All, true);
                let rows = system
                    .processes()
                    .values()
                    .map(|process| ProcessSnapshotRow {
                        name: process.name().to_string_lossy().to_string(),
                        cpu_usage: process.cpu_usage(),
                        memory_mb: process.memory() as f64 / 1_048_576.0,
                    })
                    .collect();
                match sender.try_send(rows) {
                    Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("failed to start process snapshot worker");

    let latest = Rc::new(RefCell::new(Vec::<ProcessSnapshotRow>::new()));
    let render: Rc<dyn Fn()> = {
        let content = content.clone();
        let sort_dd = sort_dd.clone();
        let latest = latest.clone();
        Rc::new(move || {
            let rows = latest.borrow();
            render_process_rows(&content, rows.as_slice(), sort_dd.selected() == 1)
        })
    };
    {
        let render = render.clone();
        sort_dd.connect_selected_notify(move |_| render());
    }
    {
        let latest = latest.clone();
        let render = render.clone();
        let win2 = win.clone();
        glib::timeout_add_local(Duration::from_millis(200), move || {
            if !win2.is_visible() {
                return glib::ControlFlow::Break;
            }
            let mut received = None;
            while let Ok(snapshot) = receiver.try_recv() {
                received = Some(snapshot);
            }
            if let Some(snapshot) = received {
                *latest.borrow_mut() = snapshot;
                render();
            }
            glib::ControlFlow::Continue
        });
    }
    {
        let win2 = win.clone();
        close.connect_clicked(move |_| win2.close());
    }
    win.present();
}

fn render_sensor_rows(content: &GtkBox, sensors: &[HwSensor]) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    if sensors.is_empty() {
        let empty = Label::new(Some("No readable hardware sensors were found."));
        empty.set_xalign(0.0);
        content.append(&empty);
        return;
    }
    let mut last_chip = String::new();
    for sensor in sensors {
        if sensor.chip != last_chip {
            let heading = Label::new(None);
            heading.set_markup(&format!(
                "<b>{}</b>",
                glib::markup_escape_text(&sensor.chip)
            ));
            heading.set_xalign(0.0);
            heading.set_margin_top(6);
            content.append(&heading);
            last_chip = sensor.chip.clone();
        }
        let row = Label::new(Some(&format!("   {} : {}", sensor.label, sensor.value)));
        row.set_xalign(0.0);
        content.append(&row);
    }
}

fn open_sensors() {
    let win = Window::builder()
        .title("Hardware Sensors")
        .default_width(400)
        .default_height(560)
        .build();
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    let content = GtkBox::new(Orientation::Vertical, 2);
    content.set_margin_top(10);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&Label::new(Some("Collecting sensor snapshot…")));
    let scroll = ScrolledWindow::builder()
        .vexpand(true)
        .child(&content)
        .build();
    vbox.append(&scroll);
    let close = Button::with_label("Close");
    close.set_margin_top(6);
    close.set_margin_bottom(6);
    vbox.append(&close);
    win.set_child(Some(&vbox));

    let (sender, receiver) = mpsc::sync_channel::<Vec<HwSensor>>(1);
    std::thread::Builder::new()
        .name("topmonitoring-sensors".into())
        .spawn(move || {
            while let Ok(()) | Err(mpsc::TrySendError::Full(_)) = sender.try_send(scan_hwmon()) {
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("failed to start sensor snapshot worker");

    {
        let content = content.clone();
        let win2 = win.clone();
        glib::timeout_add_local(Duration::from_millis(200), move || {
            if !win2.is_visible() {
                return glib::ControlFlow::Break;
            }
            let mut received = None;
            while let Ok(snapshot) = receiver.try_recv() {
                received = Some(snapshot);
            }
            if let Some(snapshot) = received {
                render_sensor_rows(&content, &snapshot);
            }
            glib::ControlFlow::Continue
        });
    }
    {
        let win2 = win.clone();
        close.connect_clicked(move |_| win2.close());
    }
    win.present();
}

/// A sensor discovered via hwmon that can be added as a custom module,
/// including a ready-to-use shell command (`command`) that reads and
/// formats its current value.
struct HwmonPick {
    display: String,
    command: String,
}

fn scan_hwmon_pickable() -> Vec<HwmonPick> {
    let mut out = Vec::new();
    let dir = match std::fs::read_dir("/sys/class/hwmon") {
        Ok(d) => d,
        Err(_) => return out,
    };
    for e in dir.flatten() {
        let base = e.path();
        let chip = std::fs::read_to_string(base.join("name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "hwmon".into());
        let specs: [(&str, &str, f64); 5] = [
            ("temp", "\u{b0}C", 1000.0),
            ("fan", "rpm", 1.0),
            ("in", "V", 1000.0),
            ("curr", "A", 1000.0),
            ("power", "W", 1_000_000.0),
        ];
        for (pfx, unit, div) in specs {
            for i in 0..=24 {
                let mut path = base.join(format!("{pfx}{i}_input"));
                if !path.exists() && pfx == "power" {
                    path = base.join(format!("{pfx}{i}_average"));
                }
                if !path.exists() {
                    continue;
                }
                let label = std::fs::read_to_string(base.join(format!("{pfx}{i}_label")))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| format!("{pfx}{i}"));
                let display = format!("{chip}: {label} ({unit})");
                let path_str = path.to_string_lossy().to_string();
                let decimals = if unit == "rpm" { "0" } else { "2" };
                let command =
                    format!("awk '{{printf \"%.{decimals}f {unit}\", $1/{div}}}' {path_str}");
                out.push(HwmonPick { display, command });
            }
        }
    }
    out
}

/// A small picker window listing every hwmon sensor found on the system;
/// clicking "Add" turns the selected sensor straight into a pre-filled
/// custom module, so the user doesn't need to hand-write the shell command.
fn populate_hwmon_picker(
    content: &GtkBox,
    picks: Vec<HwmonPick>,
    cfg: &Rc<RefCell<Config>>,
    apply_all: &Rc<dyn Fn()>,
    repop_cm: &RefreshSlot,
    window: &Window,
) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    if picks.is_empty() {
        content.append(&Label::new(Some(
            "No hwmon sensors were found on this system.",
        )));
        return;
    }
    for pick in picks {
        let row = GtkBox::new(Orientation::Horizontal, 6);
        let label = Label::new(Some(&pick.display));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        row.append(&label);
        let add = Button::with_label("Add");
        {
            let (cfg, apply_all, repop_cm, command, display, window) = (
                cfg.clone(),
                apply_all.clone(),
                repop_cm.clone(),
                pick.command,
                pick.display,
                window.clone(),
            );
            add.connect_clicked(move |_| {
                cfg.borrow_mut().custom_modules.push(CustomModule {
                    name: display.clone(),
                    label: String::new(),
                    command: command.clone(),
                    enabled: true,
                    trusted: true,
                    interval_ms: 1_000,
                    timeout_ms: 750,
                    ..CustomModule::default()
                });
                if let Some(repopulate) = repop_cm.borrow().as_ref() {
                    repopulate();
                }
                apply_all();
                window.close();
            });
        }
        row.append(&add);
        content.append(&row);
    }
}

fn open_hwmon_picker(cfg: Rc<RefCell<Config>>, apply_all: Rc<dyn Fn()>, repop_cm: RefreshSlot) {
    let win = Window::builder()
        .title("Add sensor from hwmon")
        .default_width(460)
        .default_height(560)
        .build();
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    let content = GtkBox::new(Orientation::Vertical, 2);
    content.set_margin_top(10);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&Label::new(Some("Scanning hwmon sensors…")));
    let scroll = ScrolledWindow::builder()
        .vexpand(true)
        .child(&content)
        .build();
    vbox.append(&scroll);
    let close = Button::with_label("Close");
    close.set_margin_top(6);
    close.set_margin_bottom(6);
    vbox.append(&close);
    win.set_child(Some(&vbox));

    let (sender, receiver) = mpsc::channel::<Vec<HwmonPick>>();
    std::thread::Builder::new()
        .name("topmonitoring-hwmon-picker".into())
        .spawn(move || {
            let _ = sender.send(scan_hwmon_pickable());
        })
        .expect("failed to start hwmon picker worker");
    {
        let content = content.clone();
        let cfg = cfg.clone();
        let apply_all = apply_all.clone();
        let repop_cm = repop_cm.clone();
        let win2 = win.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if !win2.is_visible() {
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Ok(picks) => {
                    populate_hwmon_picker(&content, picks, &cfg, &apply_all, &repop_cm, &win2);
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    while let Some(child) = content.first_child() {
                        content.remove(&child);
                    }
                    content.append(&Label::new(Some("Sensor scan failed.")));
                    glib::ControlFlow::Break
                }
            }
        });
    }
    {
        let win2 = win.clone();
        close.connect_clicked(move |_| win2.close());
    }
    win.present();
}

// ---------- Settings window (live-apply) ----------

fn open_settings(cfg: &Rc<RefCell<Config>>, is_wayland: bool, context: &SettingsContext) {
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
    appearance.append(&theme_section);

    let colors = settings_section("Colors", "CSS color values are validated before use.");
    for (title, value, field) in [
        ("Background", cfg.borrow().custom_bg.clone(), 0_u8),
        ("Accent", cfg.borrow().accent_color.clone(), 1_u8),
        ("Module surface", cfg.borrow().surface_color.clone(), 2_u8),
        ("Warning", cfg.borrow().warning_color.clone(), 3_u8),
        ("Critical", cfg.borrow().critical_color.clone(), 4_u8),
    ] {
        let entry = Entry::new();
        entry.set_text(&value);
        entry.set_placeholder_text(Some(if field == 0 {
            "empty = theme default"
        } else {
            "#RRGGBB or rgba(…)"
        }));
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
        labeled_row(&colors, title, &entry);
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

const CATALOG_CATEGORIES: &[&str] = &[
    "Time",
    "System",
    "Processor",
    "Memory",
    "Thermal",
    "Graphics",
    "Storage",
    "Network",
    "Connectivity",
    "Media",
    "Power",
    "Controls",
    "Packages",
    "Custom",
];

fn module_catalog_record(config: &Config, key: &ModuleCatalogKey) -> Option<ModuleCatalogRecord> {
    match key {
        ModuleCatalogKey::BuiltIn(id) => {
            let metric = config.metrics.iter().find(|metric| metric.id == *id)?;
            let descriptor = module_descriptor(id);
            let title = descriptor
                .map(|descriptor| descriptor.display_name)
                .unwrap_or(metric.id.as_str())
                .to_string();
            let category = descriptor
                .map(|descriptor| descriptor.category.label())
                .unwrap_or("System")
                .to_string();
            let capability = descriptor
                .map(|descriptor| descriptor.capability)
                .unwrap_or("built-in provider");
            Some(ModuleCatalogRecord {
                title,
                subtitle: format!(
                    "Built-in · {category} · {} · priority {} · {capability}",
                    metric.zone, metric.priority
                ),
                search_text: module_search_text(
                    &metric.id,
                    &metric.label,
                    default_prefix(&metric.id),
                ),
                category,
                kind: CatalogKind::BuiltIn,
                enabled: metric.enabled,
            })
        }
        ModuleCatalogKey::Custom(index) => {
            let module = config.custom_modules.get(*index)?;
            Some(ModuleCatalogRecord {
                title: if module.name.trim().is_empty() {
                    format!("Custom module {}", index + 1)
                } else {
                    module.name.clone()
                },
                subtitle: format!(
                    "Custom · {} · priority {} · {}",
                    module.zone,
                    module.priority,
                    if module.trusted {
                        "trusted"
                    } else {
                        "not trusted"
                    }
                ),
                search_text: format!(
                    "{} {} {} {} custom external command module",
                    module.name, module.label, module.command, module.click_command
                ),
                category: "Custom".into(),
                kind: CatalogKind::Custom,
                enabled: module.enabled,
            })
        }
    }
}

fn module_catalog_matches(
    record: &ModuleCatalogRecord,
    query: &str,
    state_filter: u32,
    kind_filter: u32,
    category_filter: Option<&str>,
) -> bool {
    let state_matches = match state_filter {
        1 => record.enabled,
        2 => !record.enabled,
        _ => true,
    };
    let kind_matches = match kind_filter {
        1 => record.kind == CatalogKind::BuiltIn,
        2 => record.kind == CatalogKind::Custom,
        _ => true,
    };
    let category_matches = category_filter.is_none_or(|category| record.category == category);
    state_matches
        && kind_matches
        && category_matches
        && search_text_matches(&record.search_text, query)
}

fn settings_page_search_target(query: &str) -> Option<&'static str> {
    [
        (
            "overview",
            "overview runtime health diagnostic provider worker accessibility notification reduce motion blink preview version backend",
        ),
        (
            "bars",
            "bar bars layout placement screen edge monitor height offset density gap padding radius refresh interval adaptive overflow more quick action lock screenshot power",
        ),
        (
            "appearance",
            "appearance theme dark light font opacity color background accent surface warning critical preset css style",
        ),
        (
            "tools",
            "data tools source interface gpu history csv dashboard export update release configuration import autostart process sensor",
        ),
    ]
    .into_iter()
    .find_map(|(page, search_text)| search_text_matches(search_text, query).then_some(page))
}

fn run_refresh(slot: &RefreshSlot) {
    if let Some(refresh) = slot.borrow().as_ref() {
        refresh();
    }
}

fn build_module_catalog_page(
    cfg: &Rc<RefCell<Config>>,
    actions: &ApplyActions,
    active: &Active,
    status: &Label,
    global_search: &SearchEntry,
    global_search_status: &Label,
    settings_stack: &Stack,
) -> ModuleCatalogUi {
    let root = GtkBox::new(Orientation::Vertical, 12);
    root.add_css_class("settings-page");
    let title = Label::new(Some("Modules"));
    title.add_css_class("settings-title");
    title.set_xalign(0.0);
    root.append(&title);
    let description = Label::new(Some(&format!(
        "Search all {} built-ins and custom commands, then edit only the selected module.",
        module_registry().len()
    )));
    description.add_css_class("settings-caption");
    description.set_xalign(0.0);
    description.set_wrap(true);
    root.append(&description);

    let filters = GtkBox::new(Orientation::Horizontal, 8);
    filters.add_css_class("module-catalog-filters");
    let state_filter = DropDown::from_strings(&["All states", "Enabled", "Disabled"]);
    let kind_filter = DropDown::from_strings(&["All modules", "Built-in", "Custom"]);
    let mut category_labels = vec!["All categories"];
    category_labels.extend_from_slice(CATALOG_CATEGORIES);
    let category_filter = DropDown::from_strings(&category_labels);
    filters.append(&state_filter);
    filters.append(&kind_filter);
    filters.append(&category_filter);
    root.append(&filters);

    let result = Label::new(Some("Loading module catalog…"));
    result.add_css_class("settings-caption");
    result.set_xalign(0.0);
    root.append(&result);

    let catalog_list = ListBox::new();
    catalog_list.add_css_class("module-catalog-list");
    catalog_list.set_selection_mode(gtk::SelectionMode::Single);
    let catalog_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_width(270)
        .child(&catalog_list)
        .build();
    let catalog_empty = Label::new(Some(
        "No modules match this search and filter combination. Clear search with Esc or reset a filter.",
    ));
    catalog_empty.add_css_class("module-catalog-empty");
    catalog_empty.set_wrap(true);
    catalog_empty.set_justify(gtk::Justification::Center);
    catalog_empty.set_margin_top(36);
    catalog_empty.set_margin_bottom(36);
    catalog_empty.set_margin_start(18);
    catalog_empty.set_margin_end(18);
    let catalog_state = Stack::new();
    catalog_state.add_named(&catalog_scroll, Some("list"));
    catalog_state.add_named(&catalog_empty, Some("empty"));
    catalog_state.set_visible_child_name("list");

    let catalog_column = GtkBox::new(Orientation::Vertical, 8);
    catalog_column.add_css_class("module-catalog-column");
    catalog_column.append(&catalog_state);
    catalog_state.set_vexpand(true);

    let catalog_actions = GtkBox::new(Orientation::Horizontal, 8);
    let add_custom = Button::with_label("Add command");
    let add_sensor = Button::with_label("Add hwmon sensor");
    catalog_actions.append(&add_custom);
    catalog_actions.append(&add_sensor);
    catalog_column.append(&catalog_actions);

    let detail = GtkBox::new(Orientation::Vertical, 12);
    detail.add_css_class("module-catalog-detail");
    let detail_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&detail)
        .build();

    let paned = Paned::new(Orientation::Horizontal);
    paned.add_css_class("module-catalog-paned");
    paned.set_start_child(Some(&catalog_column));
    paned.set_end_child(Some(&detail_scroll));
    paned.set_resize_start_child(false);
    paned.set_shrink_start_child(false);
    paned.set_position(310);
    paned.set_vexpand(true);
    root.append(&paned);

    let rows: Rc<RefCell<Vec<ModuleCatalogRow>>> = Rc::new(RefCell::new(Vec::new()));
    let selection: Rc<RefCell<Option<ModuleCatalogKey>>> = Rc::new(RefCell::new(None));
    let match_count = Rc::new(Cell::new(0_usize));
    let render_slot: RefreshSlot = Rc::new(RefCell::new(None));
    let filter_slot: RefreshSlot = Rc::new(RefCell::new(None));
    let rebuild_slot: RefreshSlot = Rc::new(RefCell::new(None));

    {
        let detail = detail.clone();
        let cfg = cfg.clone();
        let actions = actions.clone();
        let active = active.clone();
        let status = status.clone();
        let selection = selection.clone();
        let rebuild_slot = rebuild_slot.clone();
        let filter_slot = filter_slot.clone();
        *render_slot.borrow_mut() = Some(Box::new(move || {
            while let Some(child) = detail.first_child() {
                detail.remove(&child);
            }
            let Some(key) = selection.borrow().clone() else {
                append_catalog_detail_empty(
                    &detail,
                    "Select a module from the catalog to edit it.",
                );
                return;
            };
            match key {
                ModuleCatalogKey::BuiltIn(id) => {
                    render_builtin_module_editor(&detail, &id, &cfg, &actions, &filter_slot)
                }
                ModuleCatalogKey::Custom(index) => render_custom_module_editor(
                    &detail,
                    index,
                    &cfg,
                    &actions,
                    &active,
                    &status,
                    &selection,
                    &rebuild_slot,
                    &filter_slot,
                ),
            }
        }));
    }

    {
        let rows = rows.clone();
        let selection = selection.clone();
        let render_slot = render_slot.clone();
        catalog_list.connect_row_selected(move |_, selected_row| {
            let Some(selected_row) = selected_row else {
                return;
            };
            let Ok(index) = usize::try_from(selected_row.index()) else {
                return;
            };
            let Some(key) = rows.borrow().get(index).map(|row| row.key.clone()) else {
                return;
            };
            if selection.borrow().as_ref() != Some(&key) {
                *selection.borrow_mut() = Some(key);
            }
            run_refresh(&render_slot);
        });
    }

    {
        let cfg = cfg.clone();
        let rows = rows.clone();
        let selection = selection.clone();
        let match_count = match_count.clone();
        let global_search = global_search.clone();
        let state_filter = state_filter.clone();
        let kind_filter = kind_filter.clone();
        let category_filter = category_filter.clone();
        let result = result.clone();
        let catalog_state = catalog_state.clone();
        let catalog_empty = catalog_empty.clone();
        let catalog_list = catalog_list.clone();
        let render_slot = render_slot.clone();
        *filter_slot.borrow_mut() = Some(Box::new(move || {
            let query = global_search.text();
            let state = state_filter.selected();
            let kind = kind_filter.selected();
            let category = category_filter
                .selected()
                .checked_sub(1)
                .and_then(|index| CATALOG_CATEGORIES.get(index as usize).copied());
            let config = cfg.borrow();
            let selected_key = selection.borrow().clone();
            let mut visible = 0_usize;
            let mut selected_row = None;
            let mut first_row = None;
            for catalog_row in rows.borrow().iter() {
                let Some(record) = module_catalog_record(&config, &catalog_row.key) else {
                    catalog_row.row.set_visible(false);
                    continue;
                };
                catalog_row.title.set_text(&record.title);
                catalog_row.subtitle.set_text(&record.subtitle);
                catalog_row.state.set_text(if record.enabled {
                    "Enabled"
                } else {
                    "Disabled"
                });
                catalog_row.state.remove_css_class("enabled");
                catalog_row.state.remove_css_class("disabled");
                catalog_row.state.add_css_class(if record.enabled {
                    "enabled"
                } else {
                    "disabled"
                });
                let matches =
                    module_catalog_matches(&record, query.as_str(), state, kind, category);
                catalog_row.row.set_visible(matches);
                if matches {
                    visible += 1;
                    first_row.get_or_insert_with(|| catalog_row.row.clone());
                    if selected_key.as_ref() == Some(&catalog_row.key) {
                        selected_row = Some(catalog_row.row.clone());
                    }
                }
            }
            drop(config);
            match_count.set(visible);
            result.set_text(&format!("{visible} of {} modules", rows.borrow().len()));
            if visible == 0 {
                catalog_empty.set_text(if query.trim().is_empty() {
                    "No modules match these filters. Choose All states, All modules, and All categories."
                } else {
                    "No modules match this search. Press Esc to clear it or broaden the filters."
                });
                catalog_state.set_visible_child_name("empty");
            } else {
                catalog_state.set_visible_child_name("list");
            }
            if let Some(row) = selected_row {
                if catalog_list.selected_row().as_ref() != Some(&row) {
                    catalog_list.select_row(Some(&row));
                }
            } else if selected_key.is_none() {
                if let Some(row) = first_row {
                    catalog_list.select_row(Some(&row));
                }
            }
            if selection.borrow().is_none() {
                run_refresh(&render_slot);
            }
        }));
    }

    {
        let cfg = cfg.clone();
        let rows = rows.clone();
        let selection = selection.clone();
        let catalog_list = catalog_list.clone();
        let filter_slot = filter_slot.clone();
        *rebuild_slot.borrow_mut() = Some(Box::new(move || {
            catalog_list.unselect_all();
            while let Some(child) = catalog_list.first_child() {
                catalog_list.remove(&child);
            }
            rows.borrow_mut().clear();
            let keys = {
                let config = cfg.borrow();
                let mut keys: Vec<ModuleCatalogKey> = config
                    .metrics
                    .iter()
                    .map(|metric| ModuleCatalogKey::BuiltIn(metric.id.clone()))
                    .collect();
                keys.extend((0..config.custom_modules.len()).map(ModuleCatalogKey::Custom));
                keys
            };
            if selection
                .borrow()
                .as_ref()
                .is_some_and(|selected| !keys.contains(selected))
            {
                *selection.borrow_mut() = None;
            }
            for key in keys {
                let record = {
                    let config = cfg.borrow();
                    module_catalog_record(&config, &key)
                };
                let Some(record) = record else {
                    continue;
                };
                let row = ListBoxRow::new();
                row.add_css_class("module-catalog-row");
                row.set_activatable(true);
                row.set_selectable(true);
                let content = GtkBox::new(Orientation::Vertical, 3);
                let top = GtkBox::new(Orientation::Horizontal, 8);
                let title = Label::new(Some(&record.title));
                title.add_css_class("module-catalog-title");
                title.set_xalign(0.0);
                title.set_hexpand(true);
                title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                let state = Label::new(Some(if record.enabled {
                    "Enabled"
                } else {
                    "Disabled"
                }));
                state.add_css_class("module-catalog-state");
                top.append(&title);
                top.append(&state);
                let subtitle = Label::new(Some(&record.subtitle));
                subtitle.add_css_class("settings-caption");
                subtitle.set_xalign(0.0);
                subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
                content.append(&top);
                content.append(&subtitle);
                row.set_child(Some(&content));
                catalog_list.append(&row);
                rows.borrow_mut().push(ModuleCatalogRow {
                    key,
                    row,
                    title,
                    subtitle,
                    state,
                });
            }
            run_refresh(&filter_slot);
        }));
    }

    for filter in [&state_filter, &kind_filter, &category_filter] {
        let filter_slot = filter_slot.clone();
        filter.connect_selected_notify(move |_| run_refresh(&filter_slot));
    }

    {
        let cfg = cfg.clone();
        let actions = actions.clone();
        let selection = selection.clone();
        let rebuild_slot = rebuild_slot.clone();
        let status = status.clone();
        add_custom.connect_clicked(move |_| {
            let index = {
                let mut config = cfg.borrow_mut();
                let mut module = CustomModule::default();
                module.name = unique_custom_module_name(&config.custom_modules, &module.name);
                config.custom_modules.push(module);
                config.custom_modules.len() - 1
            };
            *selection.borrow_mut() = Some(ModuleCatalogKey::Custom(index));
            run_refresh(&rebuild_slot);
            (actions.layout)();
            status
                .set_text("Added a disabled custom command. Review and trust it before enabling.");
        });
    }
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let rebuild_slot = rebuild_slot.clone();
        add_sensor.connect_clicked(move |_| {
            open_hwmon_picker(cfg.clone(), layout.clone(), rebuild_slot.clone());
        });
    }

    run_refresh(&rebuild_slot);

    {
        let filter_slot = filter_slot.clone();
        let match_count = match_count.clone();
        let settings_stack = settings_stack.clone();
        let global_search_status = global_search_status.clone();
        global_search.connect_search_changed(move |entry| {
            run_refresh(&filter_slot);
            let query = entry.text();
            if query.trim().is_empty() {
                global_search_status
                    .set_text("Search pages, controls, built-ins, aliases, and custom commands.");
                return;
            }
            let matches = match_count.get();
            if matches > 0 {
                settings_stack.set_visible_child_name("modules");
                global_search_status.set_text(&format!(
                    "{matches} module{} matched. Esc clears search.",
                    if matches == 1 { "" } else { "s" }
                ));
            } else if let Some(page) = settings_page_search_target(query.as_str()) {
                settings_stack.set_visible_child_name(page);
                global_search_status.set_text(&format!(
                    "Matching Settings page: {}. Esc clears search.",
                    match page {
                        "overview" => "Overview",
                        "bars" => "Bars & layout",
                        "appearance" => "Appearance",
                        _ => "Data & tools",
                    }
                ));
            } else {
                global_search_status.set_text(
                    "No matching setting or module. Try a module alias such as HDD or Wi-Fi.",
                );
            }
        });
    }

    ModuleCatalogUi {
        root,
        paned,
        refresh_slots: vec![render_slot, filter_slot, rebuild_slot],
    }
}

fn append_catalog_detail_empty(parent: &GtkBox, message: &str) {
    let empty = Label::new(Some(message));
    empty.add_css_class("module-detail-empty");
    empty.set_wrap(true);
    empty.set_justify(gtk::Justification::Center);
    empty.set_margin_top(48);
    empty.set_margin_bottom(48);
    empty.set_margin_start(24);
    empty.set_margin_end(24);
    parent.append(&empty);
}

fn render_builtin_module_editor(
    detail: &GtkBox,
    id: &str,
    cfg: &Rc<RefCell<Config>>,
    actions: &ApplyActions,
    refresh_rows: &RefreshSlot,
) {
    let Some(index) = cfg
        .borrow()
        .metrics
        .iter()
        .position(|metric| metric.id == id)
    else {
        append_catalog_detail_empty(detail, "This built-in module is no longer available.");
        return;
    };
    let metric = cfg.borrow().metrics[index].clone();
    let descriptor = module_descriptor(&metric.id);
    let title = descriptor
        .map(|descriptor| descriptor.display_name)
        .unwrap_or(metric.id.as_str());
    let caption = descriptor
        .map(|descriptor| {
            format!(
                "Built-in · {} · {} · aliases: {}",
                descriptor.category.label(),
                descriptor.capability,
                descriptor.aliases
            )
        })
        .unwrap_or_else(|| "Built-in telemetry provider".into());
    let card = settings_section(title, &caption);
    card.add_css_class("module-editor");

    let enabled = Switch::new();
    enabled.set_active(metric.enabled);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        enabled.connect_active_notify(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.enabled = widget.is_active();
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Enabled", &enabled);

    let zone = zone_dropdown(&metric.zone);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        zone.connect_selected_notify(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.zone = zone_name(widget.selected()).into();
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Zone", &zone);

    let priority = SpinButton::with_range(0.0, 100.0, 1.0);
    priority.set_value(metric.priority as f64);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        priority.connect_value_changed(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.priority = widget.value() as i32;
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Survival priority", &priority);

    let label_entry = Entry::new();
    label_entry.set_text(&metric.label);
    label_entry.set_placeholder_text(Some(default_prefix(&metric.id)));
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        label_entry.connect_changed(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.label = widget.text().to_string();
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Label", &label_entry);

    let format_entry = Entry::new();
    format_entry.set_text(&metric.format);
    format_entry.set_placeholder_text(Some("Optional: {label} {value}"));
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        format_entry.connect_changed(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.format = widget.text().to_string();
            }
            layout();
        });
    }
    labeled_row(&card, "Display template", &format_entry);

    let compact = Switch::new();
    compact.set_active(metric.compact);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        compact.connect_active_notify(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.compact = widget.is_active();
            }
            layout();
        });
    }
    labeled_row(&card, "Allow compact/tiny tiers", &compact);

    let thresholds = GtkBox::new(Orientation::Horizontal, 8);
    let warning = SpinButton::with_range(0.0, 5_000.0, 1.0);
    warning.set_digits(1);
    warning.set_value(metric.warn);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        warning.connect_value_changed(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.warn = widget.value();
            }
            layout();
        });
    }
    let critical = SpinButton::with_range(0.0, 5_000.0, 1.0);
    critical.set_digits(1);
    critical.set_value(metric.crit);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        critical.connect_value_changed(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.crit = widget.value();
            }
            layout();
        });
    }
    thresholds.append(&Label::new(Some("Warning")));
    thresholds.append(&warning);
    thresholds.append(&Label::new(Some("Critical")));
    thresholds.append(&critical);
    labeled_row(&card, "Thresholds (0 = default)", &thresholds);

    let colors = GtkBox::new(Orientation::Horizontal, 8);
    let foreground = Entry::new();
    foreground.set_placeholder_text(Some("foreground"));
    foreground.set_text(&metric.fg_color);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        foreground.connect_changed(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.fg_color = widget.text().to_string();
            }
            style();
        });
    }
    let background = Entry::new();
    background.set_placeholder_text(Some("background"));
    background.set_text(&metric.bg_color);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        background.connect_changed(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.bg_color = widget.text().to_string();
            }
            style();
        });
    }
    colors.append(&foreground);
    colors.append(&background);
    labeled_row(&card, "Module colors", &colors);

    let command_row = GtkBox::new(Orientation::Horizontal, 8);
    let command = Entry::new();
    command.set_hexpand(true);
    command.set_text(&metric.command);
    command.set_placeholder_text(Some("Optional primary-click command"));
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        command.connect_changed(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.command = widget.text().to_string();
            }
            layout();
        });
    }
    let trusted = Switch::new();
    trusted.set_active(metric.command_trusted);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        trusted.connect_active_notify(move |widget| {
            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(index) {
                metric.command_trusted = widget.is_active();
            }
            layout();
        });
    }
    command_row.append(&command);
    command_row.append(&Label::new(Some("Trusted")));
    command_row.append(&trusted);
    labeled_row(&card, "Click action", &command_row);
    detail.append(&card);
}

#[allow(clippy::too_many_arguments)]
fn render_custom_module_editor(
    detail: &GtkBox,
    index: usize,
    cfg: &Rc<RefCell<Config>>,
    actions: &ApplyActions,
    active: &Active,
    status: &Label,
    selection: &Rc<RefCell<Option<ModuleCatalogKey>>>,
    rebuild_rows: &RefreshSlot,
    refresh_rows: &RefreshSlot,
) {
    let Some(module) = cfg.borrow().custom_modules.get(index).cloned() else {
        append_catalog_detail_empty(detail, "This custom module was removed.");
        return;
    };
    let title = if module.name.trim().is_empty() {
        format!("Custom module {}", index + 1)
    } else {
        module.name.clone()
    };
    let card = settings_section(
        &title,
        "External command module · imported commands remain disabled until trusted.",
    );
    card.add_css_class("module-editor");

    let actions_row = GtkBox::new(Orientation::Horizontal, 8);
    let run_now = Button::with_label("Run now");
    {
        let cfg = cfg.clone();
        let active = active.clone();
        let status = status.clone();
        run_now.connect_clicked(move |_| {
            let name = cfg
                .borrow()
                .custom_modules
                .get(index)
                .map(|module| module.name.clone())
                .unwrap_or_default();
            if request_custom_module_run(&active, &name) {
                status.set_text(&format!("Queued custom module “{name}”."));
            } else {
                status.set_text("Custom module is not active on the bar.");
            }
        });
    }
    let duplicate = Button::with_label("Duplicate");
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let status = status.clone();
        let selection = selection.clone();
        let rebuild_rows = rebuild_rows.clone();
        duplicate.connect_clicked(move |_| {
            let copy = {
                let config = cfg.borrow();
                config.custom_modules.get(index).cloned().map(|mut copy| {
                    copy.name = unique_custom_module_name(&config.custom_modules, &copy.name);
                    copy
                })
            };
            let Some(copy) = copy else {
                return;
            };
            cfg.borrow_mut().custom_modules.insert(index + 1, copy);
            let name = cfg.borrow().custom_modules[index + 1].name.clone();
            *selection.borrow_mut() = Some(ModuleCatalogKey::Custom(index + 1));
            run_refresh(&rebuild_rows);
            layout();
            status.set_text(&format!("Duplicated custom module as “{name}”."));
        });
    }
    let remove = Button::with_label("Remove");
    remove.add_css_class("destructive-action");
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let selection = selection.clone();
        let rebuild_rows = rebuild_rows.clone();
        let status = status.clone();
        remove.connect_clicked(move |_| {
            if index < cfg.borrow().custom_modules.len() {
                cfg.borrow_mut().custom_modules.remove(index);
            }
            *selection.borrow_mut() = None;
            run_refresh(&rebuild_rows);
            layout();
            status.set_text("Removed custom module. Save changes to persist.");
        });
    }
    actions_row.append(&run_now);
    actions_row.append(&duplicate);
    actions_row.append(&remove);
    card.append(&actions_row);

    let enabled = Switch::new();
    enabled.set_active(module.enabled);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        enabled.connect_active_notify(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.enabled = widget.is_active();
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Enabled", &enabled);

    let trusted = Switch::new();
    trusted.set_active(module.trusted);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        trusted.connect_active_notify(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.trusted = widget.is_active();
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Trusted executable behavior", &trusted);

    let zone = zone_dropdown(&module.zone);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        zone.connect_selected_notify(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.zone = zone_name(widget.selected()).into();
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Zone", &zone);

    let priority = SpinButton::with_range(0.0, 100.0, 1.0);
    priority.set_value(module.priority as f64);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        priority.connect_value_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.priority = widget.value() as i32;
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Survival priority", &priority);

    let compact = Switch::new();
    compact.set_active(module.compact);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        compact.connect_active_notify(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.compact = widget.is_active();
            }
            layout();
        });
    }
    labeled_row(&card, "Allow compact/tiny tiers", &compact);

    let name = Entry::new();
    name.set_text(&module.name);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        name.connect_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.name = widget.text().to_string();
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Name", &name);

    let label = Entry::new();
    label.set_text(&module.label);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        label.connect_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.label = widget.text().to_string();
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Label", &label);

    let format_entry = Entry::new();
    format_entry.set_text(&module.format);
    format_entry.set_placeholder_text(Some("Optional: {label} {value}"));
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        format_entry.connect_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.format = widget.text().to_string();
            }
            layout();
        });
    }
    labeled_row(&card, "Display template", &format_entry);

    let tooltip = Entry::new();
    tooltip.set_text(&module.tooltip);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        tooltip.connect_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.tooltip = widget.text().to_string();
            }
            layout();
        });
    }
    labeled_row(&card, "Tooltip", &tooltip);

    let command = Entry::new();
    command.set_text(&module.command);
    command.set_placeholder_text(Some("Command executed on its own cadence"));
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        command.connect_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.command = widget.text().to_string();
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Command", &command);

    let click = Entry::new();
    click.set_text(&module.click_command);
    click.set_placeholder_text(Some("Optional primary-click command"));
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        let refresh_rows = refresh_rows.clone();
        click.connect_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.click_command = widget.text().to_string();
            }
            layout();
            run_refresh(&refresh_rows);
        });
    }
    labeled_row(&card, "Click action", &click);

    let cadence = GtkBox::new(Orientation::Horizontal, 8);
    let interval = SpinButton::with_range(250.0, 3_600_000.0, 250.0);
    interval.set_value(module.interval_ms as f64);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        interval.connect_value_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.interval_ms = widget.value() as u64;
            }
            layout();
        });
    }
    let timeout = SpinButton::with_range(100.0, 60_000.0, 100.0);
    timeout.set_value(module.timeout_ms as f64);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        timeout.connect_value_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.timeout_ms = widget.value() as u64;
            }
            layout();
        });
    }
    cadence.append(&Label::new(Some("Every ms")));
    cadence.append(&interval);
    cadence.append(&Label::new(Some("Timeout ms")));
    cadence.append(&timeout);
    labeled_row(&card, "Cadence", &cadence);

    let output_cap = SpinButton::with_range(256.0, 65_536.0, 256.0);
    output_cap.set_value(module.max_output_bytes as f64);
    {
        let cfg = cfg.clone();
        let layout = actions.layout.clone();
        output_cap.connect_value_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.max_output_bytes = widget.value() as usize;
            }
            layout();
        });
    }
    labeled_row(&card, "Output cap (bytes)", &output_cap);

    let colors = GtkBox::new(Orientation::Horizontal, 8);
    let foreground = Entry::new();
    foreground.set_placeholder_text(Some("foreground"));
    foreground.set_text(&module.fg_color);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        foreground.connect_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.fg_color = widget.text().to_string();
            }
            style();
        });
    }
    let background = Entry::new();
    background.set_placeholder_text(Some("background"));
    background.set_text(&module.bg_color);
    {
        let cfg = cfg.clone();
        let style = actions.style.clone();
        background.connect_changed(move |widget| {
            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(index) {
                module.bg_color = widget.text().to_string();
            }
            style();
        });
    }
    colors.append(&foreground);
    colors.append(&background);
    labeled_row(&card, "Module colors", &colors);
    detail.append(&card);
}

fn settings_page(title: &str, description: &str) -> (ScrolledWindow, GtkBox) {
    let page = GtkBox::new(Orientation::Vertical, 12);
    page.add_css_class("settings-page");
    let title_label = Label::new(Some(title));
    title_label.add_css_class("settings-title");
    title_label.set_xalign(0.0);
    page.append(&title_label);
    let description_label = Label::new(Some(description));
    description_label.add_css_class("settings-caption");
    description_label.set_xalign(0.0);
    description_label.set_wrap(true);
    page.append(&description_label);
    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&page)
        .build();
    (scroll, page)
}

fn settings_section(title: &str, description: &str) -> GtkBox {
    let section = GtkBox::new(Orientation::Vertical, 8);
    section.add_css_class("settings-section");
    let title_label = Label::new(None);
    title_label.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(title)));
    title_label.set_xalign(0.0);
    section.append(&title_label);
    if !description.is_empty() {
        let description_label = Label::new(Some(description));
        description_label.add_css_class("settings-caption");
        description_label.set_xalign(0.0);
        description_label.set_wrap(true);
        section.append(&description_label);
    }
    section
}

fn zone_dropdown(zone: &str) -> DropDown {
    let dropdown = DropDown::from_strings(&["Start", "Center", "End"]);
    dropdown.set_selected(match zone {
        "start" => 0,
        "end" => 2,
        _ => 1,
    });
    dropdown
}

fn zone_name(index: u32) -> &'static str {
    match index {
        0 => "start",
        2 => "end",
        _ => "center",
    }
}

fn labeled_row(parent: &GtkBox, text: &str, widget: &impl IsA<gtk::Widget>) {
    let row = GtkBox::new(Orientation::Horizontal, 10);
    let label = Label::new(Some(text));
    label.set_hexpand(true);
    label.set_xalign(0.0);
    label.set_wrap(true);
    row.append(&label);
    row.append(widget);
    parent.append(&row);
}

#[cfg(test)]
mod history_tests {
    use super::*;

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

    #[test]
    fn process_rows_sort_without_live_sysinfo_objects() {
        let mut rows = vec![
            ProcessSnapshotRow {
                name: "memory-heavy".into(),
                cpu_usage: 10.0,
                memory_mb: 900.0,
            },
            ProcessSnapshotRow {
                name: "cpu-heavy".into(),
                cpu_usage: 80.0,
                memory_mb: 100.0,
            },
        ];
        sort_process_rows(&mut rows, false);
        assert_eq!(rows[0].name, "cpu-heavy");
        sort_process_rows(&mut rows, true);
        assert_eq!(rows[0].name, "memory-heavy");
    }
}

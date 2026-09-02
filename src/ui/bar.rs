#![allow(unused, dead_code)]
#![allow(clippy::too_many_arguments)]
//! Bar rendering — extracted from main.rs (Fase A.1).
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

pub(crate) enum SlotWidget {
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
pub(crate) struct Slot {
    pub(crate) id: String,
    pub(crate) prefix: String,
    /// Optional display template with `{label}` and `{value}`.
    pub(crate) format: String,
    /// Non-empty only for custom (shell-command) modules.
    pub(crate) source_cmd: String,
    pub(crate) warn: f64,
    pub(crate) crit: f64,
    pub(crate) widget: SlotWidget,
    pub(crate) layout: SlotLayoutHandle,
    pub(crate) last_value: RefCell<Option<String>>,
    /// Channel used to receive command output from a background thread,
    /// so a slow custom-module command never blocks the UI thread.
    pub(crate) cmd_tx: Option<Sender<CommandResult>>,
    pub(crate) cmd_rx: Option<Receiver<CommandResult>>,
    /// True while a background command for this slot is still running,
    /// preventing threads from piling up if a command is slow.
    pub(crate) busy: Option<Rc<Cell<bool>>>,
    /// On-demand run request, kept pending while the previous run is busy.
    pub(crate) force_run: Option<Rc<Cell<bool>>>,
    pub(crate) interval_ms: u64,
    pub(crate) timeout_ms: u64,
    pub(crate) max_output_bytes: usize,
    pub(crate) trusted: bool,
    pub(crate) last_started: RefCell<Option<Instant>>,
}

#[derive(Clone)]
pub(crate) struct SlotLayoutHandle {
    pub(crate) tier: Rc<Cell<LayoutTier>>,
    pub(crate) dirty: Rc<Cell<bool>>,
    pub(crate) overflow_value: Label,
}

pub(crate) struct AdaptiveEntry {
    pub(crate) key: String,
    pub(crate) display_name: String,
    pub(crate) priority: i32,
    pub(crate) order: usize,
    pub(crate) profile: WidthProfile,
    pub(crate) root: gtk::Widget,
    pub(crate) text: Option<Label>,
    pub(crate) graph: Option<DrawingArea>,
    pub(crate) scale: Option<Scale>,
    pub(crate) overflow_row: GtkBox,
    pub(crate) handle: SlotLayoutHandle,
}

pub(crate) type AdaptiveEntries = Rc<RefCell<Vec<AdaptiveEntry>>>;

#[derive(Clone)]
pub(crate) struct OverflowUi {
    pub(crate) button: gtk::MenuButton,
    pub(crate) content: GtkBox,
}

pub(crate) type Active = Rc<RefCell<Vec<Slot>>>;
type RefreshFn = Box<dyn Fn()>;
pub(crate) type RefreshSlot = Rc<RefCell<Option<RefreshFn>>>;

#[derive(Clone)]
pub(crate) struct BarZones {
    pub(crate) start: GtkBox,
    pub(crate) center: GtkBox,
    pub(crate) end: GtkBox,
}

impl BarZones {
    pub(crate) fn new(spacing: i32) -> Self {
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
    pub(crate) fn new() -> Self {
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

pub(crate) fn make_adaptive_entry(
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

pub(crate) fn module_width_budget(window_width: i32, cfg: &Config) -> u32 {
    let quick_actions = [cfg.qa_lock, cfg.qa_screenshot, cfg.qa_shutdown]
        .into_iter()
        .filter(|enabled| *enabled)
        .count() as u32;
    let reserved = 116_u32.saturating_add(quick_actions.saturating_mul(78));
    (window_width.max(0) as u32).saturating_sub(reserved)
}

pub(crate) fn apply_adaptive_layout(
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
pub(crate) struct ApplyActions {
    pub(crate) style: Rc<dyn Fn()>,
    pub(crate) layout: Rc<dyn Fn()>,
    pub(crate) geometry: Rc<dyn Fn()>,
    pub(crate) interval: Rc<dyn Fn()>,
    pub(crate) all: Rc<dyn Fn()>,
}

#[derive(Clone)]
pub(crate) struct SettingsContext {
    pub(crate) bar_window: ApplicationWindow,
    pub(crate) active: Active,
    pub(crate) runtime: RuntimeServices,
    pub(crate) core: CoreCollector,
    pub(crate) actions: ApplyActions,
    pub(crate) settings_slot: Rc<RefCell<Option<Window>>>,
}

thread_local! {
    static DISK_PREV: RefCell<HashMap<String, (u64, u64, Instant)>> = RefCell::new(HashMap::new());
    static RAPL_PREV: RefCell<(u64, Instant)> = RefCell::new((0, Instant::now()));
    static ALERTS: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
}

// ---------- Bar widgets ----------

/// Runs `cmd` in a shell when `w` is left-clicked. No-op if `cmd` is empty.
pub(crate) fn attach_click(w: &impl IsA<gtk::Widget>, command: &str, trusted: bool) {
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
pub(crate) fn attach_detail_popover<W: IsA<gtk::Widget> + Clone + 'static>(w: &W) {
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
pub(crate) fn make_graph(
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
pub(crate) fn add_module_classes(widget: &impl IsA<gtk::Widget>, id: &str) {
    widget.add_css_class("module");
    widget.add_css_class(&format!("module-{}", css_class_id(id)));
}

/// V3-009 transient action feedback: a floating label over the bar that shows
/// pending → success/error with backend name and error detail, then hides.
/// `None` = pending, `Some(true)` = success, `Some(false)` = error.
pub(crate) type ActionToast = Rc<dyn Fn(&str, Option<bool>)>;

/// Creates a non-interactive toast label layered on top of the bar overlay.
pub(crate) fn make_action_toast(overlay: &gtk::Overlay) -> ActionToast {
    let label = Label::new(None);
    label.add_css_class("action-toast");
    label.set_halign(gtk::Align::Center);
    label.set_valign(gtk::Align::Start);
    label.set_wrap(true);
    label.set_visible(false);
    label.set_selectable(false);
    overlay.add_overlay(&label);

    let hide_source: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let label = label.clone();
    Rc::new(move |message: &str, state: Option<bool>| {
        label.set_text(message);
        label.remove_css_class("action-toast-pending");
        label.remove_css_class("action-toast-ok");
        label.remove_css_class("action-toast-error");
        match state {
            None => label.add_css_class("action-toast-pending"),
            Some(true) => label.add_css_class("action-toast-ok"),
            Some(false) => label.add_css_class("action-toast-error"),
        }
        label.set_visible(true);
        if let Some(source) = hide_source.borrow_mut().take() {
            source.remove();
        }
        let hide = label.clone();
        let source = glib::timeout_add_local(Duration::from_secs(3), move || {
            hide.set_visible(false);
            glib::ControlFlow::Break
        });
        *hide_source.borrow_mut() = Some(source);
    })
}

pub(crate) fn connect_action_feedback(
    button: &Button,
    idle_label: &'static str,
    pending_label: &'static str,
    action: QuickAction,
    runtime: &RuntimeServices,
    action_toast: &ActionToast,
) {
    let service = runtime.clone();
    let toast = action_toast.clone();
    button.connect_clicked(move |button| {
        let request_id = service.quick_action(action);
        button.set_label(pending_label);
        button.set_sensitive(false);
        button.remove_css_class("action-ok");
        button.remove_css_class("action-error");
        button.add_css_class("action-pending");
        toast(&format!("{idle_label}… pending"), None);

        let button = button.clone();
        let service = service.clone();
        let toast = toast.clone();
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
            if feedback.success {
                toast(
                    &format!(
                        "{idle_label} ✓ — {} · {} ms",
                        feedback.provider, feedback.duration_ms
                    ),
                    Some(true),
                );
            } else {
                toast(
                    &format!(
                        "{idle_label} ✗ — {} ({})",
                        feedback.provider, feedback.message
                    ),
                    Some(false),
                );
            }

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
pub(crate) fn rebuild_bar_metrics(
    zones: &BarZones,
    active: &Active,
    adaptive: &AdaptiveEntries,
    cfg: &Config,
    runtime: &RuntimeServices,
    action_toast: &ActionToast,
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
        connect_action_feedback(
            &button,
            "Lock",
            "Lock…",
            QuickAction::Lock,
            runtime,
            action_toast,
        );
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
            action_toast,
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

pub(crate) fn set_formatted(label: &Label, prefix: &str, template: &str, value: &str) {
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

pub(crate) fn search_text_matches(haystack: &str, query: &str) -> bool {
    let haystack = haystack.to_lowercase();
    query
        .split_whitespace()
        .all(|term| haystack.contains(&term.to_lowercase()))
}

pub(crate) fn unique_custom_module_name(modules: &[CustomModule], base: &str) -> String {
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

pub(crate) fn request_custom_module_run(active: &Active, module_name: &str) -> bool {
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
pub(crate) fn maybe_notify(key: &str, active: bool, title: &str, body: &str) {
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
pub(crate) fn apply_level(
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
pub(crate) fn apply_level_low(
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

pub(crate) fn external_metric_changed(
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

pub(crate) fn update_slot(
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
pub(crate) fn apply_core_sample(
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
pub(crate) fn update_metric(
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

/// Writes an autostart `.desktop` entry pointing at the currently running
/// executable. The path is quoted so it works even when the binary lives
/// somewhere with spaces in the path.
pub(crate) fn install_autostart() -> bool {
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

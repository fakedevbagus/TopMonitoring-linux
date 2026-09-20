//! Layout editor extracted from main.rs.
use crate::config::{default_prefix, Config};
use crate::layout::{
    allocate_layout, custom_width_profile, module_width_profile, LayoutItem, LayoutTier,
    WidthProfile,
};
use crate::model::module_descriptor;
use crate::ui::bar::{module_width_budget, ApplyActions};
use gtk::glib;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, DropDown, Label, Orientation, ScrolledWindow};
use std::cell::RefCell;
use std::rc::Rc;

// ---------- Module layout editor (Step 0.5) ----------

/// Identifies which configuration entry a layout row edits.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutRowTarget {
    BuiltIn(usize),
    Custom(usize),
}

/// One enabled module in bar order, described independently of the two config
/// vecs so the layout editor can present a single visual list.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutRow {
    pub target: LayoutRowTarget,
    pub key: String,
    pub display: String,
    pub zone: String,
    pub priority: i32,
    pub compact: bool,
    pub profile: WidthProfile,
}

/// Enabled modules exactly as the bar renders them: built-ins first (priority
/// descending, config order as tie-break), then custom modules with the same
/// rule. Disabled modules are intentionally excluded.
pub(crate) fn collect_layout_rows(config: &Config) -> Vec<LayoutRow> {
    let mut rows = Vec::new();
    let mut metric_order: Vec<usize> = (0..config.metrics.len()).collect();
    metric_order.sort_by(|left, right| {
        config.metrics[*right]
            .priority
            .cmp(&config.metrics[*left].priority)
            .then_with(|| left.cmp(right))
    });
    for index in metric_order {
        let metric = &config.metrics[index];
        if !metric.enabled {
            continue;
        }
        let descriptor = module_descriptor(&metric.id);
        let display = descriptor
            .map(|descriptor| descriptor.display_name.to_string())
            .unwrap_or_else(|| default_prefix(&metric.id).to_string());
        let profile = descriptor
            .map(|descriptor| descriptor.width_profile())
            .unwrap_or_else(|| module_width_profile(&metric.id));
        rows.push(LayoutRow {
            target: LayoutRowTarget::BuiltIn(index),
            key: format!("builtin:{}", metric.id),
            display,
            zone: metric.zone.clone(),
            priority: metric.priority,
            compact: metric.compact,
            profile,
        });
    }
    let mut custom_order: Vec<usize> = (0..config.custom_modules.len()).collect();
    custom_order.sort_by(|left, right| {
        config.custom_modules[*right]
            .priority
            .cmp(&config.custom_modules[*left].priority)
            .then_with(|| left.cmp(right))
    });
    for index in custom_order {
        let module = &config.custom_modules[index];
        if !module.enabled {
            continue;
        }
        rows.push(LayoutRow {
            target: LayoutRowTarget::Custom(index),
            key: format!("custom:{}", module.name),
            display: module.name.clone(),
            zone: module.zone.clone(),
            priority: module.priority,
            compact: module.compact,
            profile: custom_width_profile(),
        });
    }
    rows
}

/// Swap a module with its nearest same-zone neighbour in the config vec.
/// Move buttons reorder only within one zone, so a module cannot leap across
/// zones by accident. Returns `true` when a swap actually happened.
pub(crate) fn layout_row_swap(
    config: &mut Config,
    target: &LayoutRowTarget,
    direction: i8,
) -> bool {
    match target {
        LayoutRowTarget::BuiltIn(from) => {
            let zone = config.metrics[*from].zone.clone();
            let mut cursor = *from as isize + direction as isize;
            while (0..config.metrics.len() as isize).contains(&cursor) {
                let index = cursor as usize;
                if config.metrics[index].zone == zone {
                    config.metrics.swap(*from, index);
                    return true;
                }
                cursor += direction as isize;
            }
            false
        }
        LayoutRowTarget::Custom(from) => {
            let zone = config.custom_modules[*from].zone.clone();
            let mut cursor = *from as isize + direction as isize;
            while (0..config.custom_modules.len() as isize).contains(&cursor) {
                let index = cursor as usize;
                if config.custom_modules[index].zone == zone {
                    config.custom_modules.swap(*from, index);
                    return true;
                }
                cursor += direction as isize;
            }
            false
        }
    }
}

/// Items fed to `allocate_layout`, matching the real bar's order and compact
/// policy, so the tier preview always agrees with the live bar.
pub(crate) fn layout_preview_items(config: &Config) -> Vec<LayoutItem> {
    collect_layout_rows(config)
        .into_iter()
        .enumerate()
        .map(|(order, row)| {
            let profile = if config.adaptive_compact && row.compact {
                row.profile
            } else {
                row.profile.fixed_normal()
            };
            LayoutItem {
                key: row.key,
                priority: row.priority,
                order,
                profile,
            }
        })
        .collect()
}

pub(crate) fn zone_tier_summary(tier: LayoutTier, width_px: u16) -> String {
    match tier {
        LayoutTier::Normal => format!("Normal · {width_px}px"),
        LayoutTier::Compact => format!("Compact · {width_px}px"),
        LayoutTier::Tiny => format!("Tiny · {width_px}px"),
        LayoutTier::Overflow => "→ More (hidden)".into(),
    }
}

/// Rebuilds the module-layout list inside `list` from the live config.
/// Handlers own their own widget clones and are therefore dropped together
/// with their row, so repeated renders never accumulate signal handlers.
pub(crate) fn render_module_layout(
    list: &GtkBox,
    cfg: &Rc<RefCell<Config>>,
    actions: &ApplyActions,
    window_width: i32,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let config = cfg.borrow();
    let rows = collect_layout_rows(&config);
    let items = layout_preview_items(&config);
    let budget = module_width_budget(window_width, &config);
    let gap = config.module_gap.max(0) as u16;
    let decisions = allocate_layout(&items, budget, gap);
    let decision_for = |key: &str| {
        decisions
            .iter()
            .find(|decision| decision.key == key)
            .map(|decision| decision.tier)
            .unwrap_or(LayoutTier::Overflow)
    };
    let width_for = |key: &str| {
        decisions
            .iter()
            .find(|decision| decision.key == key)
            .map(|decision| decision.width_px)
            .unwrap_or(0)
    };

    for (zone_index, zone) in ["start", "center", "end"].iter().enumerate() {
        let zone_rows: Vec<&LayoutRow> = rows.iter().filter(|row| row.zone == *zone).collect();
        if zone_rows.is_empty() {
            continue;
        }
        let header = Label::new(None);
        header.set_markup(&format!(
            "<b>{}</b>",
            glib::markup_escape_text(["Start", "Center", "End"][zone_index])
        ));
        header.set_xalign(0.0);
        list.append(&header);

        for (position, row) in zone_rows.iter().enumerate() {
            let tier = decision_for(&row.key);
            let summary = zone_tier_summary(tier, width_for(&row.key));

            let row_box = GtkBox::new(Orientation::Horizontal, 6);
            row_box.add_css_class("module-layout-row");

            let up = Button::with_label("↑");
            up.set_tooltip_text(Some("Move up within this zone"));
            let down = Button::with_label("↓");
            down.set_tooltip_text(Some("Move down within this zone"));
            // Comfortable 44x44 touch target for icon-only buttons.
            up.set_size_request(44, 44);
            down.set_size_request(44, 44);

            let name = Label::new(Some(&row.display));
            name.set_hexpand(true);
            name.set_xalign(0.0);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);

            let tier_label = Label::new(Some(&summary));
            tier_label.add_css_class(match tier {
                LayoutTier::Overflow => "tier-overflow",
                LayoutTier::Normal => "tier-normal",
                LayoutTier::Compact | LayoutTier::Tiny => "tier-tight",
            });

            let zone_drop = DropDown::from_strings(&["Start", "Center", "End"]);
            zone_drop.set_selected(match row.zone.as_str() {
                "start" => 0,
                "end" => 2,
                _ => 1,
            });

            up.set_sensitive(position > 0);
            down.set_sensitive(position + 1 < zone_rows.len());

            let up_target = row.target.clone();
            {
                let cfg = cfg.clone();
                let actions = actions.clone();
                let list = list.clone();
                up.connect_clicked(move |_| {
                    if layout_row_swap(&mut cfg.borrow_mut(), &up_target, -1) {
                        (actions.layout.clone())();
                        render_module_layout(&list, &cfg, &actions, window_width);
                    }
                });
            }
            let down_target = row.target.clone();
            {
                let cfg = cfg.clone();
                let actions = actions.clone();
                let list = list.clone();
                down.connect_clicked(move |_| {
                    if layout_row_swap(&mut cfg.borrow_mut(), &down_target, 1) {
                        (actions.layout.clone())();
                        render_module_layout(&list, &cfg, &actions, window_width);
                    }
                });
            }
            let zone_target = row.target.clone();
            {
                let cfg = cfg.clone();
                let actions = actions.clone();
                let list = list.clone();
                zone_drop.connect_selected_notify(move |widget| {
                    let zone = zone_name(widget.selected()).to_string();
                    match &zone_target {
                        LayoutRowTarget::BuiltIn(index) => {
                            if let Some(metric) = cfg.borrow_mut().metrics.get_mut(*index) {
                                metric.zone = zone;
                            }
                        }
                        LayoutRowTarget::Custom(index) => {
                            if let Some(module) = cfg.borrow_mut().custom_modules.get_mut(*index) {
                                module.zone = zone;
                            }
                        }
                    }
                    (actions.layout.clone())();
                    render_module_layout(&list, &cfg, &actions, window_width);
                });
            }

            row_box.append(&up);
            row_box.append(&down);
            row_box.append(&name);
            row_box.append(&tier_label);
            row_box.append(&zone_drop);
            list.append(&row_box);
        }
    }
}

pub(crate) fn settings_page(title: &str, description: &str) -> (ScrolledWindow, GtkBox) {
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

pub(crate) fn settings_section(title: &str, description: &str) -> GtkBox {
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

pub(crate) fn zone_dropdown(zone: &str) -> DropDown {
    let dropdown = DropDown::from_strings(&["Start", "Center", "End"]);
    dropdown.set_selected(match zone {
        "start" => 0,
        "end" => 2,
        _ => 1,
    });
    dropdown
}

pub(crate) fn zone_name(index: u32) -> &'static str {
    match index {
        0 => "start",
        2 => "end",
        _ => "center",
    }
}

pub(crate) fn labeled_row(parent: &GtkBox, text: &str, widget: &impl IsA<gtk::Widget>) {
    let row = GtkBox::new(Orientation::Horizontal, 10);
    let label = Label::new(Some(text));
    label.set_hexpand(true);
    label.set_xalign(0.0);
    label.set_wrap(true);
    row.append(&label);
    row.append(widget);
    parent.append(&row);
}

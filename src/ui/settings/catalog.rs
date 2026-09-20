//! Module catalog UI.
use crate::config::{default_prefix, Config, CustomModule};
use crate::model::{module_descriptor, module_registry, module_search_text};
use crate::ui::bar::{Active, ApplyActions, RefreshSlot};
use crate::ui::open_hwmon_picker;
use crate::ui::settings::layout::{labeled_row, settings_section, zone_dropdown, zone_name};
use crate::ui::settings::search::{
    request_custom_module_run, search_text_matches, settings_page_search_target,
    unique_custom_module_name,
};
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, DropDown, Entry, Label, ListBox, ListBoxRow, Orientation, Paned,
    ScrolledWindow, SearchEntry, SpinButton, Stack, Switch,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogKind {
    BuiltIn,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleCatalogKey {
    BuiltIn(String),
    Custom(usize),
}

pub struct ModuleCatalogRecord {
    pub title: String,
    pub subtitle: String,
    pub search_text: String,
    pub category: String,
    pub kind: CatalogKind,
    pub enabled: bool,
}

pub(crate) struct ModuleCatalogRow {
    key: ModuleCatalogKey,
    row: ListBoxRow,
    title: Label,
    subtitle: Label,
    state: Label,
}

pub(crate) struct ModuleCatalogUi {
    pub(crate) root: GtkBox,
    pub(crate) paned: Paned,
    pub(crate) refresh_slots: Vec<RefreshSlot>,
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

pub(crate) fn module_catalog_record(
    config: &Config,
    key: &ModuleCatalogKey,
) -> Option<ModuleCatalogRecord> {
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

pub(crate) fn module_catalog_matches(
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

pub(crate) fn run_refresh(slot: &RefreshSlot) {
    if let Some(refresh) = slot.borrow().as_ref() {
        refresh();
    }
}

pub(crate) fn build_module_catalog_page(
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
    // Non-homogeneous: the list and empty placeholders are both scrolled/filled
    // and a homogeneous stack re-measures both each frame, feeding the
    // GtkPaned "minimum height" Gtk-CRITICAL warnings in this page.
    catalog_state.set_hhomogeneous(false);
    catalog_state.set_vhomogeneous(false);
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

pub(crate) fn append_catalog_detail_empty(parent: &GtkBox, message: &str) {
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

pub(crate) fn render_builtin_module_editor(
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
pub(crate) fn render_custom_module_editor(
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

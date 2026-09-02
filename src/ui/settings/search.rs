#![allow(unused, dead_code)]
//! Settings search helpers.
use crate::config::CustomModule;
use crate::ui::bar::Active;

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

pub(crate) fn settings_page_search_target(query: &str) -> Option<&'static str> {
    [
        (
            "overview",
            "overview runtime health diagnostic provider worker accessibility notification reduce motion blink preview version backend",
        ),
        (
            "bars",
            "bar bars layout placement screen edge monitor height offset density gap padding radius refresh interval adaptive overflow more quick action lock screenshot power module order zone reorder up down tier preview simulate width",
        ),
        (
            "appearance",
            "appearance theme dark light font weight opacity color picker background accent surface warning critical border radius preset undo restore default css style",
        ),
        (
            "tools",
            "data tools source interface gpu history csv dashboard export update release configuration import autostart process sensor",
        ),
    ]
    .into_iter()
    .find_map(|(page, search_text)| search_text_matches(search_text, query).then_some(page))
}

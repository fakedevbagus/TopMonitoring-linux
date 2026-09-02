//! Hwmon sensor picker window.

use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use crate::config::{Config, CustomModule};

/// A sensor discovered via hwmon that can be added as a custom module,
/// including a ready-to-use shell command (`command`) that reads and
/// formats its current value.
#[derive(Clone, Debug)]
pub struct HwmonPick {
    pub display: String,
    pub command: String,
}

type RefreshFn = Box<dyn Fn()>;
type RefreshSlot = Rc<RefCell<Option<RefreshFn>>>;

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

/// Populate the picker window. The remaining arguments mirror the
/// `open_hwmon_picker` signature so users get a pre-filled custom module
/// without hand-writing the shell command.
fn populate_hwmon_picker(
    content: &gtk::Box,
    picks: Vec<HwmonPick>,
    cfg: &Rc<RefCell<Config>>,
    apply_all: &Rc<dyn Fn()>,
    repop_cm: &RefreshSlot,
    window: &gtk::Window,
) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    if picks.is_empty() {
        content.append(&gtk::Label::new(Some(
            "No hwmon sensors were found on this system.",
        )));
        return;
    }
    for pick in picks {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let label = gtk::Label::new(Some(&pick.display));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        row.append(&label);
        let add = gtk::Button::with_label("Add");
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

/// Open the hwmon sensor picker window.
pub fn open_hwmon_picker(cfg: Rc<RefCell<Config>>, apply_all: Rc<dyn Fn()>, repop_cm: RefreshSlot) {
    let win = gtk::Window::builder()
        .title("Add sensor from hwmon")
        .default_width(460)
        .default_height(560)
        .build();
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_margin_top(10);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&gtk::Label::new(Some("Scanning hwmon sensors…")));
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&content)
        .build();
    vbox.append(&scroll);
    let close = gtk::Button::with_label("Close");
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
                    content.append(&gtk::Label::new(Some("Sensor scan failed.")));
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

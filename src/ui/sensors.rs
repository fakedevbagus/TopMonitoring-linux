//! Hardware sensor window and the hwmon scanner behind it.

use gtk::glib;
use gtk::prelude::*;
use std::sync::mpsc;
use std::time::Duration;

/// One discovered hardware sensor with its formatted value.
#[derive(Clone, Debug, PartialEq)]
pub struct HwSensor {
    pub chip: String,
    pub label: String,
    pub value: String,
}

/// Enumerate every readable channel under every `/sys/class/hwmon/hwmon*`
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

fn render_sensor_rows(content: &gtk::Box, sensors: &[HwSensor]) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    if sensors.is_empty() {
        let empty = gtk::Label::new(Some("No readable hardware sensors were found."));
        empty.set_xalign(0.0);
        content.append(&empty);
        return;
    }
    let mut last_chip = String::new();
    for sensor in sensors {
        if sensor.chip != last_chip {
            let heading = gtk::Label::new(None);
            heading.set_markup(&format!(
                "<b>{}</b>",
                glib::markup_escape_text(&sensor.chip)
            ));
            heading.set_xalign(0.0);
            heading.set_margin_top(6);
            content.append(&heading);
            last_chip = sensor.chip.clone();
        }
        let row = gtk::Label::new(Some(&format!("   {} : {}", sensor.label, sensor.value)));
        row.set_xalign(0.0);
        content.append(&row);
    }
}

/// Open the hardware sensor window.
pub fn open_sensors() {
    let win = gtk::Window::builder()
        .title("Hardware Sensors")
        .default_width(400)
        .default_height(560)
        .build();
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_margin_top(10);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&gtk::Label::new(Some("Collecting sensor snapshot…")));
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

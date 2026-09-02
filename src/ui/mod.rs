//! Secondary windows (process manager, sensors, hwmon picker) extracted
//! from `main.rs` as part of v4 Fase A.1.

pub mod hwmon;
pub mod processes;
pub mod sensors;

pub use hwmon::open_hwmon_picker;
pub use processes::open_process_manager;
pub use sensors::open_sensors;

use crate::history::{history_log_path, history_summary_text, summarize_history, HistorySummary};
use crate::runtime::RuntimeServices;
use gtk::glib;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, DrawingArea, Label, Orientation, Window};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

/// Create a horizontal range slider. The returned `suppress` flag is set while
/// the periodic poll syncs the value from the OS, so the sync does not
/// re-trigger the drag handler and fight the user's input. A left-click drag
/// toggles the `dragging` CSS class, which the poller uses to skip mid-drag
/// syncs.
pub fn make_range() -> (gtk::Scale, Rc<Cell<bool>>) {
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_size_request(80, -1);
    scale.set_draw_value(false);
    scale.set_hexpand(false);
    let gesture = gtk::GestureClick::new();
    gesture.set_button(1);
    let pressed = scale.clone();
    gesture.connect_pressed(move |_, _, _, _| pressed.add_css_class("dragging"));
    let released = scale.clone();
    gesture.connect_released(move |_, _, _, _| released.remove_css_class("dragging"));
    scale.add_controller(gesture);
    (scale, Rc::new(Cell::new(false)))
}
/// Non-blocking history dashboard window with a CPU/RAM/temperature chart.
pub fn open_history_dashboard(runtime: RuntimeServices) {
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

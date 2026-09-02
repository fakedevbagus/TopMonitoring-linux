//! Snapshot-backed process manager window.
//!
//! Sysinfo ownership and refresh stay on a worker thread; GTK only sorts
//! owned rows and reconciles labels (V3-012).

use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

/// One owned row in the process list.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessSnapshotRow {
    pub name: String,
    pub cpu_usage: f32,
    pub memory_mb: f64,
}

fn sort_process_rows(rows: &mut [ProcessSnapshotRow], by_memory: bool) {
    if by_memory {
        rows.sort_by(|left, right| right.memory_mb.total_cmp(&left.memory_mb));
    } else {
        rows.sort_by(|left, right| right.cpu_usage.total_cmp(&left.cpu_usage));
    }
}

fn render_process_rows(content: &gtk::Box, rows: &[ProcessSnapshotRow], by_memory: bool) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    let mut rows = rows.to_vec();
    sort_process_rows(&mut rows, by_memory);
    if rows.is_empty() {
        let empty = gtk::Label::new(Some("No processes available."));
        empty.set_xalign(0.0);
        content.append(&empty);
        return;
    }
    for process in rows.iter().take(20) {
        let row = gtk::Label::new(None);
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

/// Open the small task manager window.
pub fn open_process_manager() {
    let win = gtk::Window::builder()
        .title("Process Manager")
        .default_width(480)
        .default_height(560)
        .build();
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header.set_margin_top(8);
    header.set_margin_start(12);
    header.set_margin_end(12);
    let sort_dd = gtk::DropDown::from_strings(&["Sort by CPU", "Sort by RAM"]);
    header.append(&gtk::Label::new(Some("Top processes:")));
    header.append(&sort_dd);
    vbox.append(&header);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_margin_top(8);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&gtk::Label::new(Some("Collecting process snapshot…")));
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

    let (sender, receiver) = mpsc::sync_channel::<Vec<ProcessSnapshotRow>>(1);
    std::thread::Builder::new()
        .name("topmonitoring-processes".into())
        .spawn(move || {
            let mut system = sysinfo::System::new_all();
            loop {
                system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
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

#[cfg(test)]
mod tests {
    use super::*;

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

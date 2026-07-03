mod config;

use config::{build_css, default_metrics, default_prefix, Config, CustomModule};
use gdk4_x11::prelude::*;
use gtk::gio::prelude::FileExt;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, CenterBox, ColorDialog,
    ColorDialogButton, CssProvider, DrawingArea, DropDown, Entry, FileDialog, GestureClick, Label,
    Orientation, ScrolledWindow, SpinButton, Switch, TextView, Window,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use sysinfo::{Components, Disks, Networks, System};

enum SlotWidget {
    Text(Label),
    Graph { area: DrawingArea, hist: Rc<RefCell<VecDeque<f64>>> },
}

struct Slot {
    id: String,
    prefix: String,
    source_cmd: String,
    warn: f64,
    crit: f64,
    widget: SlotWidget,
}

type Active = Rc<RefCell<Vec<Slot>>>;

thread_local! {
    static DISK_PREV: RefCell<(u64, u64, Instant)> = RefCell::new((0, 0, Instant::now()));
    static RAPL_PREV: RefCell<(u64, Instant)> = RefCell::new((0, Instant::now()));
    static ALERTS: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
}

fn main() {
    let app = Application::builder()
        .application_id("com.satria.topmonitoring")
        .build();
    let built = std::rc::Rc::new(std::cell::Cell::new(false));
    app.connect_activate(move |app| {
        if built.get() {
            return; // sudah jalan → jangan bikin bar kedua
        }
        built.set(true);
        build_bar(app);
    });
    app.run();
}

fn build_bar(app: &Application) {
    let cfg = Rc::new(RefCell::new(Config::load()));
    let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();

    let provider = Rc::new(CssProvider::new());
    provider.load_from_string(&build_css(&cfg.borrow(), None));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &*provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let window = ApplicationWindow::builder().application(app).build();
    window.add_css_class("topbar"); // scope tema hanya ke bar
    window.set_default_height(cfg.borrow().height);

    if is_wayland {
        window.init_layer_shell();
        configure_wayland(&window, &cfg.borrow());
    } else {
        window.set_decorated(false);
        window.set_resizable(false);
        let cfg2 = cfg.clone();
        window.connect_realize(move |w| configure_x11(w, &cfg2.borrow()));
    }

    let root = CenterBox::new();
    let metrics_box = GtkBox::new(Orientation::Horizontal, 4);
    metrics_box.set_halign(gtk::Align::Center);
    root.set_center_widget(Some(&metrics_box));
    let settings_btn = Button::with_label("⚙");
    settings_btn.set_margin_end(8);
    root.set_end_widget(Some(&settings_btn));
    window.set_child(Some(&root));

    let active: Active = Rc::new(RefCell::new(Vec::new()));
    rebuild_bar_metrics(&metrics_box, &active, &cfg.borrow());

    let open = {
        let (cfg, provider, window, metrics_box, active) = (
            cfg.clone(), provider.clone(), window.clone(), metrics_box.clone(), active.clone(),
        );
        Rc::new(move || open_settings(&cfg, &provider, &window, is_wayland, &metrics_box, &active))
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

    let sys = Rc::new(RefCell::new(System::new_all()));
    let comps = Rc::new(RefCell::new(Components::new_with_refreshed_list()));
    let nets = Rc::new(RefCell::new(Networks::new_with_refreshed_list()));
    let disks = Rc::new(RefCell::new(Disks::new_with_refreshed_list()));
    let nvml = Rc::new(Nvml::init().ok());
    let tick = Rc::new(Cell::new(0u32));
    let interval = cfg.borrow().interval_ms.max(200);

    {
        let (active, cfg, provider, tick) =
            (active.clone(), cfg.clone(), provider.clone(), tick.clone());
        glib::timeout_add_local(Duration::from_millis(interval), move || {
            {
                let mut s = sys.borrow_mut();
                s.refresh_cpu_all();
                s.refresh_memory();
            }
            comps.borrow_mut().refresh(true);
            nets.borrow_mut().refresh(true);
            disks.borrow_mut().refresh(true);

            let s = sys.borrow();
            let cc = comps.borrow();
            let nn = nets.borrow();
            let dd = disks.borrow();
            let c = cfg.borrow();
            for slot in active.borrow().iter() {
                update_slot(slot, &s, &cc, &nn, &dd, &nvml, c.notifications);
            }
            if c.animated_bg {
                let t = tick.get();
                tick.set(t.wrapping_add(1));
                provider.load_from_string(&build_css(&c, Some((t as f64 * 3.0) % 360.0)));
            }
            glib::ControlFlow::Continue
        });
    }

    window.present();
}

// ---------- positioning ----------
fn configure_wayland(window: &ApplicationWindow, cfg: &Config) {
    let bottom = cfg.position == "bottom";
    window.set_layer(Layer::Top);
    window.set_anchor(Edge::Top, !bottom);
    window.set_anchor(Edge::Bottom, bottom);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, if bottom { 0 } else { cfg.margin_top });
    window.set_margin(Edge::Bottom, if bottom { cfg.margin_top } else { 0 });
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
            match apply_x11_dock(x11.xid() as u32, geo, cfg.margin_top as u32, cfg.height as u32, cfg.position == "bottom") {
                Ok(w) => window.set_size_request(w as i32, cfg.height),
                Err(e) => eprintln!("[TopMonitoring] strut X11 gagal: {e}"),
            }
        }
    }
}

fn apply_x11_dock(
    xid: u32, geo: Option<(i32, i32, i32, i32)>, offset: u32, bar_height: u32, bottom: bool,
) -> Result<u32, Box<dyn std::error::Error>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConfigureWindowAux, ConnectionExt as XProto, PropMode};
    use x11rb::wrapper::ConnectionExt as Wrapper;

    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let sw = screen.width_in_pixels as i32;
    let sh = screen.height_in_pixels as i32;
    let (mx, my, mw, mh) = geo.unwrap_or((0, 0, sw, sh));
    let mw_u = mw as u32;

    let atom = |name: &[u8]| -> Result<u32, Box<dyn std::error::Error>> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    };

    let wtype = atom(b"_NET_WM_WINDOW_TYPE")?;
    let dock = atom(b"_NET_WM_WINDOW_TYPE_DOCK")?;
    conn.change_property32(PropMode::REPLACE, xid, wtype, AtomEnum::ATOM, &[dock])?;

    let strut_p = atom(b"_NET_WM_STRUT_PARTIAL")?;
    let strut = atom(b"_NET_WM_STRUT")?;
    let x_start = mx as u32;
    let x_end = (mx + mw - 1) as u32;
    let (y, sp, sl): (i32, [u32; 12], [u32; 4]) = if bottom {
        let reserve = (bar_height as i32 + offset as i32 + (sh - (my + mh))) as u32;
        (my + mh - bar_height as i32 - offset as i32,
         [0, 0, 0, reserve, 0, 0, 0, 0, 0, 0, x_start, x_end],
         [0, 0, 0, reserve])
    } else {
        let reserve = (my as u32) + offset + bar_height;
        (my + offset as i32,
         [0, 0, reserve, 0, 0, 0, 0, 0, x_start, x_end, 0, 0],
         [0, 0, reserve, 0])
    };
    conn.change_property32(PropMode::REPLACE, xid, strut_p, AtomEnum::CARDINAL, &sp)?;
    conn.change_property32(PropMode::REPLACE, xid, strut, AtomEnum::CARDINAL, &sl)?;

    let desktop = atom(b"_NET_WM_DESKTOP")?;
    conn.change_property32(PropMode::REPLACE, xid, desktop, AtomEnum::CARDINAL, &[0xFFFF_FFFF])?;

    let state = atom(b"_NET_WM_STATE")?;
    let sticky = atom(b"_NET_WM_STATE_STICKY")?;
    let above = atom(b"_NET_WM_STATE_ABOVE")?;
    let skip_tb = atom(b"_NET_WM_STATE_SKIP_TASKBAR")?;
    let skip_pg = atom(b"_NET_WM_STATE_SKIP_PAGER")?;
    conn.change_property32(PropMode::REPLACE, xid, state, AtomEnum::ATOM, &[sticky, above, skip_tb, skip_pg])?;

    conn.configure_window(xid, &ConfigureWindowAux::new().x(mx).y(y).width(mw_u).height(bar_height))?;
    conn.flush()?;
    Ok(mw_u)
}

// ---------- bar build ----------
fn attach_click(w: &impl IsA<gtk::Widget>, cmd: &str) {
    if cmd.trim().is_empty() {
        return;
    }
    let g = GestureClick::new();
    g.set_button(1);
    let cmd = cmd.to_string();
    g.connect_pressed(move |_, _, _, _| {
        let _ = std::process::Command::new("sh").arg("-c").arg(&cmd).spawn();
    });
    w.add_controller(g);
}

fn make_graph() -> (DrawingArea, Rc<RefCell<VecDeque<f64>>>) {
    let area = DrawingArea::new();
    area.set_content_width(64);
    area.set_vexpand(true);
    let hist = Rc::new(RefCell::new(VecDeque::with_capacity(64)));
    let h2 = hist.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let hist = h2.borrow();
        if hist.len() < 2 {
            return;
        }
        let maxv = hist.iter().cloned().fold(1.0_f64, f64::max);
        let n = hist.len();
        let dx = w as f64 / (n - 1) as f64;
        cr.set_source_rgba(0.0, 0.82, 1.0, 0.9);
        cr.set_line_width(1.5);
        for (i, v) in hist.iter().enumerate() {
            let x = i as f64 * dx;
            let y = h as f64 - (v / maxv).clamp(0.0, 1.0) * h as f64;
            if i == 0 { cr.move_to(x, y); } else { cr.line_to(x, y); }
        }
        let _ = cr.stroke();
    });
    (area, hist)
}

fn rebuild_bar_metrics(metrics_box: &GtkBox, active: &Active, cfg: &Config) {
    while let Some(child) = metrics_box.first_child() {
        metrics_box.remove(&child);
    }
    let mut act = active.borrow_mut();
    act.clear();
    for m in &cfg.metrics {
        if !m.enabled {
            continue;
        }
        if m.id.ends_with("_graph") {
            let (area, hist) = make_graph();
            metrics_box.append(&area);
            attach_click(&area, &m.command);
            act.push(Slot { id: m.id.clone(), prefix: String::new(), source_cmd: String::new(), warn: m.warn, crit: m.crit, widget: SlotWidget::Graph { area, hist } });
        } else {
            let label = Label::new(None);
            if m.id == "clock" {
                label.add_css_class("accent");
            }
            metrics_box.append(&label);
            attach_click(&label, &m.command);
            let prefix = if m.label.is_empty() { default_prefix(&m.id).to_string() } else { m.label.clone() };
            act.push(Slot { id: m.id.clone(), prefix, source_cmd: String::new(), warn: m.warn, crit: m.crit, widget: SlotWidget::Text(label) });
        }
    }
    for cm in &cfg.custom_modules {
        if !cm.enabled {
            continue;
        }
        let label = Label::new(None);
        metrics_box.append(&label);
        act.push(Slot { id: format!("custom:{}", cm.name), prefix: cm.label.clone(), source_cmd: cm.command.clone(), warn: 0.0, crit: 0.0, widget: SlotWidget::Text(label) });
    }
}

// ---------- updating ----------
fn set_prefixed(label: &Label, prefix: &str, value: &str) {
    if prefix.is_empty() { label.set_text(value); } else { label.set_text(&format!("{prefix} {value}")); }
}

fn maybe_notify(key: &str, active: bool, title: &str, body: &str) {
    if !active { return; }
    ALERTS.with(|m| {
        let mut m = m.borrow_mut();
        let now = Instant::now();
        let ok = m.get(key).map(|t| now.duration_since(*t).as_secs() >= 60).unwrap_or(true);
        if ok {
            m.insert(key.to_string(), now);
            let _ = std::process::Command::new("notify-send").arg(title).arg(body).spawn();
        }
    });
}

fn apply_level(label: &Label, v: f64, warn_o: f64, crit_o: f64, dw: f64, dc: f64, notify: bool, key: &str, title: &str) {
    let w = if warn_o > 0.0 { warn_o } else { dw };
    let c = if crit_o > 0.0 { crit_o } else { dc };
    if v >= c {
        label.add_css_class("crit");
        maybe_notify(key, notify, title, &format!("{title} = {v:.0} (kritis!)"));
    } else if v >= w {
        label.add_css_class("warn");
    }
}

fn update_slot(slot: &Slot, s: &System, cc: &Components, nn: &Networks, dd: &Disks, nvml: &Option<Nvml>, notify: bool) {
    match &slot.widget {
        SlotWidget::Graph { area, hist } => {
            let v = match slot.id.as_str() {
                "cpu_graph" => s.global_cpu_usage() as f64,
                "ram_graph" => if s.total_memory() > 0 { s.used_memory() as f64 / s.total_memory() as f64 * 100.0 } else { 0.0 },
                "net_graph" => { let mut rx = 0u64; for (_n, d) in nn.iter() { rx += d.received(); } rx as f64 }
                _ => 0.0,
            };
            {
                let mut h = hist.borrow_mut();
                if h.len() >= 64 { h.pop_front(); }
                h.push_back(v);
            }
            area.queue_draw();
        }
        SlotWidget::Text(label) => {
            if !slot.source_cmd.is_empty() {
                set_prefixed(label, &slot.prefix, &run_cmd(&slot.source_cmd));
            } else {
                update_metric(&slot.id, &slot.prefix, label, s, cc, nn, dd, nvml, notify, slot.warn, slot.crit);
            }
        }
    }
}

fn run_cmd(cmd: &str) -> String {
    match std::process::Command::new("sh").arg("-c").arg(cmd).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "err".into(),
    }
}

fn update_metric(
    id: &str, prefix: &str, label: &Label, s: &System, cc: &Components, nn: &Networks, dd: &Disks,
    nvml: &Option<Nvml>, notify: bool, warn_o: f64, crit_o: f64,
) {
    label.remove_css_class("warn");
    label.remove_css_class("crit");
    match id {
        "clock" => { if let Ok(dt) = glib::DateTime::now_local() { if let Ok(t) = dt.format("%H:%M:%S") { set_prefixed(label, prefix, t.as_str()); } } }
        "date" => { if let Ok(dt) = glib::DateTime::now_local() { if let Ok(t) = dt.format("%a %d %b") { set_prefixed(label, prefix, t.as_str()); } } }
        "cpu" => {
            let v = s.global_cpu_usage() as f64;
            set_prefixed(label, prefix, &format!("{v:>3.0}%"));
            apply_level(label, v, warn_o, crit_o, 70.0, 90.0, notify, "cpu", "CPU");
            let mut tip = String::from("Per-core:");
            for (i, c) in s.cpus().iter().enumerate() { tip.push_str(&format!("\ncore{i}: {:.0}%", c.cpu_usage())); }
            label.set_tooltip_text(Some(&tip));
        }
        "cpumodel" => { if let Some(c) = s.cpus().first() { set_prefixed(label, prefix, c.brand().trim()); } }
        "freq" => { if let Some(c0) = s.cpus().first() { set_prefixed(label, prefix, &format!("{:.1}GHz", c0.frequency() as f64 / 1000.0)); } }
        "cpu_power" => match read_cpu_power() { Some(w) => set_prefixed(label, prefix, &format!("{w:.0}W")), None => set_prefixed(label, prefix, "n/a") },
        "vcore" => match read_vcore() { Some(v) => set_prefixed(label, prefix, &format!("{v:.2}V")), None => set_prefixed(label, prefix, "n/a") },
        "memory" => {
            let (u, t) = (s.used_memory() as f64, s.total_memory() as f64);
            let p = if t > 0.0 { u / t * 100.0 } else { 0.0 };
            set_prefixed(label, prefix, &format!("{:.1}/{:.0}G", u / 1e9, t / 1e9));
            apply_level(label, p, warn_o, crit_o, 80.0, 92.0, notify, "mem", "RAM");
            label.set_tooltip_text(Some(&format!("RAM: {:.0}/{:.0} MB\nSwap: {:.0}/{:.0} MB", u / 1e6, t / 1e6, s.used_swap() as f64 / 1e6, s.total_swap() as f64 / 1e6)));
        }
        "memavail" => set_prefixed(label, prefix, &format!("{:.1}G", s.available_memory() as f64 / 1e9)),
        "swap" => { let st = s.total_swap().max(1) as f64; set_prefixed(label, prefix, &format!("{:>3.0}%", s.used_swap() as f64 / st * 100.0)); }
        "temp" => {
            let t = cc.iter().filter_map(|c| c.temperature()).fold(0.0f32, f32::max);
            set_prefixed(label, prefix, &format!("{t:>2.0}°C"));
            apply_level(label, t as f64, warn_o, crit_o, 75.0, 90.0, notify, "temp", "Suhu");
            let mut tt = String::from("Sensor:");
            for c in cc.iter() { if let Some(v) = c.temperature() { tt.push_str(&format!("\n{}: {:.0}°C", c.label(), v)); } }
            label.set_tooltip_text(Some(&tt));
        }
        "fan" => match read_fan_rpm() { Some(rpm) => set_prefixed(label, prefix, &format!("{rpm}rpm")), None => set_prefixed(label, prefix, "n/a") },
        "gpu" => {
            if let Some(nv) = nvml {
                if let Ok(dev) = nv.device_by_index(0) {
                    let util = dev.utilization_rates().map(|u| u.gpu).unwrap_or(0);
                    let gt = dev.temperature(TemperatureSensor::Gpu).unwrap_or(0);
                    let vram = match dev.memory_info() { Ok(m) => format!(" {:.1}/{:.0}G", m.used as f64 / 1e9, m.total as f64 / 1e9), Err(_) => String::new() };
                    set_prefixed(label, prefix, &format!("{util}% {gt}°C{vram}"));
                    apply_level(label, util as f64, warn_o, crit_o, 80.0, 92.0, notify, "gpu", "GPU");
                    return;
                }
            }
            if let Some((busy, temp)) = read_amd_gpu() {
                let t = temp.map(|t| format!(" {t}°C")).unwrap_or_default();
                set_prefixed(label, prefix, &format!("{busy}%{t}"));
                apply_level(label, busy as f64, warn_o, crit_o, 80.0, 92.0, notify, "gpu", "GPU");
            } else { set_prefixed(label, prefix, "n/a"); }
        }
        "gpu_power" => {
            let w = nvml.as_ref().and_then(|nv| nv.device_by_index(0).ok()).and_then(|d| d.power_usage().ok()).map(|mw| mw as f64 / 1000.0)
                .or_else(|| amd_card_device().and_then(|d| amd_hwmon_read(&d, "power1_average").or_else(|| amd_hwmon_read(&d, "power1_input"))).map(|uw| uw / 1e6));
            match w { Some(v) => set_prefixed(label, prefix, &format!("{v:.0}W")), None => set_prefixed(label, prefix, "n/a") }
        }
        "gpu_clock" => {
            let c = nvml.as_ref().and_then(|nv| nv.device_by_index(0).ok()).and_then(|d| d.clock_info(Clock::Graphics).ok())
                .or_else(|| amd_card_device().and_then(|d| amd_active_clock(&d, "pp_dpm_sclk")));
            match c { Some(v) => set_prefixed(label, prefix, &format!("{v}MHz")), None => set_prefixed(label, prefix, "n/a") }
        }
        "gpu_memclock" => {
            let c = nvml.as_ref().and_then(|nv| nv.device_by_index(0).ok()).and_then(|d| d.clock_info(Clock::Memory).ok())
                .or_else(|| amd_card_device().and_then(|d| amd_active_clock(&d, "pp_dpm_mclk")));
            match c { Some(v) => set_prefixed(label, prefix, &format!("{v}MHz")), None => set_prefixed(label, prefix, "n/a") }
        }
        "gpu_fan" => {
            if let Some(pct) = nvml.as_ref().and_then(|nv| nv.device_by_index(0).ok()).and_then(|d| d.fan_speed(0).ok()) {
                set_prefixed(label, prefix, &format!("{pct}%"));
            } else if let Some(rpm) = amd_card_device().and_then(|d| amd_hwmon_read(&d, "fan1_input")) {
                set_prefixed(label, prefix, &format!("{rpm:.0}rpm"));
            } else { set_prefixed(label, prefix, "n/a"); }
        }
        "disk" => {
            for disk in dd.iter() {
                if disk.mount_point() == Path::new("/") {
                    let tot = disk.total_space() as f64;
                    let avail = disk.available_space() as f64;
                    let p = if tot > 0.0 { (tot - avail) / tot * 100.0 } else { 0.0 };
                    set_prefixed(label, prefix, &format!("{p:>3.0}%"));
                    apply_level(label, p, warn_o, crit_o, 85.0, 95.0, notify, "disk", "Disk");
                    label.set_tooltip_text(Some(&format!("Free: {:.1} GB", avail / 1e9)));
                }
            }
        }
        "diskio" => match read_disk_io() {
            Some((r, w)) => set_prefixed(label, prefix, &format!("R{} W{}", human_rate(r as u64), human_rate(w as u64))),
            None => set_prefixed(label, prefix, "n/a"),
        },
        "network" => {
            let (mut rx, mut tx) = (0u64, 0u64);
            let mut tip = String::from("Interface:");
            for (name, data) in nn.iter() {
                rx += data.received();
                tx += data.transmitted();
                tip.push_str(&format!("\n{name}: ↓{} ↑{}", human_rate(data.received()), human_rate(data.transmitted())));
            }
            set_prefixed(label, prefix, &format!("↓{} ↑{}", human_rate(rx), human_rate(tx)));
            label.set_tooltip_text(Some(&tip));
        }
        "netttl" => {
            let (mut rx, mut tx) = (0u64, 0u64);
            for (_n, d) in nn.iter() { rx += d.total_received(); tx += d.total_transmitted(); }
            set_prefixed(label, prefix, &format!("Σ↓{} ↑{}", human_bytes(rx), human_bytes(tx)));
        }
        "battery" => match read_battery() {
            Some((cap, status)) => {
                let icon = if status == "Charging" { "⚡" } else { "🔋" };
                set_prefixed(label, prefix, &format!("{icon}{cap}%"));
                if cap as f64 <= 10.0 { label.add_css_class("crit"); maybe_notify("bat", notify, "Baterai", &format!("Baterai {cap}%!")); }
                else if cap as f64 <= 20.0 { label.add_css_class("warn"); }
            }
            None => set_prefixed(label, prefix, "n/a"),
        },
        "procs" => set_prefixed(label, prefix, &read_proc_count().to_string()),
        "uptime" => set_prefixed(label, prefix, &human_uptime(System::uptime())),
        "load" => {
            let la = System::load_average();
            set_prefixed(label, prefix, &format!("{:.2}", la.one));
            label.set_tooltip_text(Some(&format!("1m {:.2}  5m {:.2}  15m {:.2}", la.one, la.five, la.fifteen)));
        }
        "host" => { if let Some(h) = System::host_name() { set_prefixed(label, prefix, &h); } }
        "kernel" => { if let Some(k) = System::kernel_version() { set_prefixed(label, prefix, &k); } }
        "os" => { let txt = format!("{} {}", System::name().unwrap_or_default(), System::os_version().unwrap_or_default()); set_prefixed(label, prefix, txt.trim()); }
        _ => {}
    }
}

// ---------- sysfs helpers ----------
fn read_fan_rpm() -> Option<u32> {
    let mut max = 0u32;
    for entry in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let base = entry.path();
        for i in 1..=8 {
            if let Ok(s) = std::fs::read_to_string(base.join(format!("fan{i}_input"))) {
                if let Ok(v) = s.trim().parse::<u32>() { if v > max { max = v; } }
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
                        let status = std::fs::read_to_string(p.join("status")).map(|s| s.trim().to_string()).unwrap_or_default();
                        return Some((cap, status));
                    }
                }
            }
        }
    }
    None
}
fn read_amd_gpu() -> Option<(u32, Option<u32>)> {
    let dev = amd_card_device()?;
    let busy = std::fs::read_to_string(dev.join("gpu_busy_percent")).ok()?.trim().parse::<u32>().ok()?;
    Some((busy, amd_hwmon_read(&dev, "temp1_input").map(|v| (v / 1000.0) as u32)))
}
fn amd_card_device() -> Option<PathBuf> {
    for entry in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("card") && !name.contains('-') {
            let dev = entry.path().join("device");
            if dev.join("gpu_busy_percent").exists() { return Some(dev); }
        }
    }
    None
}
fn amd_hwmon_read(dev: &Path, file: &str) -> Option<f64> {
    for e in std::fs::read_dir(dev.join("hwmon")).ok()?.flatten() {
        if let Ok(s) = std::fs::read_to_string(e.path().join(file)) {
            if let Ok(v) = s.trim().parse::<f64>() { return Some(v); }
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
                    if let Ok(v) = num.parse::<u32>() { return Some(v); }
                }
            }
        }
    }
    None
}
fn read_cpu_power() -> Option<f64> {
    let e = std::fs::read_to_string("/sys/class/powercap/intel-rapl:0/energy_uj").ok()?.trim().parse::<u64>().ok()?;
    RAPL_PREV.with(|p| {
        let mut prev = p.borrow_mut();
        let dt = prev.1.elapsed().as_secs_f64().max(0.001);
        let dj = e.saturating_sub(prev.0) as f64 / 1e6;
        let first = prev.0 == 0;
        *prev = (e, Instant::now());
        if first { None } else { Some(dj / dt) }
    })
}
fn read_vcore() -> Option<f64> {
    for e in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let base = e.path();
        for i in 0..=12 {
            if let Ok(lbl) = std::fs::read_to_string(base.join(format!("in{i}_label"))) {
                if lbl.trim().eq_ignore_ascii_case("vcore") {
                    if let Ok(v) = std::fs::read_to_string(base.join(format!("in{i}_input"))) {
                        if let Ok(mv) = v.trim().parse::<f64>() { return Some(mv / 1000.0); }
                    }
                }
            }
        }
    }
    None
}
fn read_proc_count() -> usize {
    std::fs::read_dir("/proc").map(|d| d.flatten().filter(|e| e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit())).count()).unwrap_or(0)
}
fn read_disk_io() -> Option<(f64, f64)> {
    let content = std::fs::read_to_string("/proc/diskstats").ok()?;
    let (mut rd, mut wr) = (0u64, 0u64);
    for line in content.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 { continue; }
        let name = f[2];
        let whole = if name.starts_with("nvme") || name.starts_with("mmcblk") { !name.contains('p') }
            else if name.starts_with("sd") { !name.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(true) }
            else { false };
        if !whole { continue; }
        rd += f[5].parse::<u64>().unwrap_or(0);
        wr += f[9].parse::<u64>().unwrap_or(0);
    }
    DISK_PREV.with(|p| {
        let mut prev = p.borrow_mut();
        let dt = prev.2.elapsed().as_secs_f64().max(0.001);
        let dr = rd.saturating_sub(prev.0) as f64 * 512.0 / dt;
        let dw = wr.saturating_sub(prev.1) as f64 * 512.0 / dt;
        *prev = (rd, wr, Instant::now());
        Some((dr, dw))
    })
}
fn human_rate(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1_048_576.0 { format!("{:.1}MB/s", b / 1_048_576.0) } else { format!("{:.0}KB/s", b / 1024.0) }
}
fn human_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 { format!("{:.1}GB", b / 1e9) } else if b >= 1e6 { format!("{:.0}MB", b / 1e6) } else { format!("{:.0}KB", b / 1e3) }
}
fn human_uptime(secs: u64) -> String { format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60) }

fn install_autostart() -> bool {
    if let (Ok(exe), Some(dir)) = (std::env::current_exe(), dirs::config_dir()) {
        let adir = dir.join("autostart");
        if std::fs::create_dir_all(&adir).is_err() { return false; }
        let content = format!(
            "[Desktop Entry]\nType=Application\nName=TopMonitoring\nExec=\"{}\"\nX-KDE-autostart-phase=2\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        return std::fs::write(adir.join("topmonitoring.desktop"), content).is_ok();
    }
    false
}

// ---------- Sensor Hardware window ----------
struct HwSensor { chip: String, label: String, value: String }

fn scan_hwmon() -> Vec<HwSensor> {
    let mut out = Vec::new();
    let dir = match std::fs::read_dir("/sys/class/hwmon") { Ok(d) => d, Err(_) => return out };
    for e in dir.flatten() {
        let base = e.path();
        let chip = std::fs::read_to_string(base.join("name")).map(|s| s.trim().to_string()).unwrap_or_else(|_| "hwmon".into());
        let specs: [(&str, &str, f64); 5] = [("temp", "°C", 1000.0), ("fan", "rpm", 1.0), ("in", "V", 1000.0), ("curr", "A", 1000.0), ("power", "W", 1_000_000.0)];
        for (pfx, unit, div) in specs {
            for i in 0..=24 {
                let mut raw = std::fs::read_to_string(base.join(format!("{pfx}{i}_input"))).ok();
                if raw.is_none() && pfx == "power" { raw = std::fs::read_to_string(base.join(format!("{pfx}{i}_average"))).ok(); }
                let Some(raw) = raw else { continue };
                let Ok(val) = raw.trim().parse::<f64>() else { continue };
                let label = std::fs::read_to_string(base.join(format!("{pfx}{i}_label"))).map(|s| s.trim().to_string()).unwrap_or_else(|_| format!("{pfx}{i}"));
                let v = val / div;
                let value = if unit == "rpm" { format!("{v:.0} {unit}") } else { format!("{v:.2} {unit}") };
                out.push(HwSensor { chip: chip.clone(), label, value });
            }
        }
    }
    out
}

fn open_sensors() {
    let win = Window::builder().title("Sensor Hardware").default_width(400).default_height(560).build();
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    let content = GtkBox::new(Orientation::Vertical, 2);
    content.set_margin_top(10);
    content.set_margin_start(12);
    content.set_margin_end(12);
    let scroll = ScrolledWindow::builder().vexpand(true).child(&content).build();
    vbox.append(&scroll);
    let close = Button::with_label("Tutup");
    close.set_margin_top(6);
    close.set_margin_bottom(6);
    vbox.append(&close);
    win.set_child(Some(&vbox));

    let refresh: Rc<dyn Fn()> = {
        let content = content.clone();
        Rc::new(move || {
            while let Some(c) = content.first_child() { content.remove(&c); }
            let mut last = String::new();
            for s in scan_hwmon() {
                if s.chip != last {
                    let h = Label::new(None);
                    h.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(&s.chip)));
                    h.set_xalign(0.0);
                    h.set_margin_top(6);
                    content.append(&h);
                    last = s.chip.clone();
                }
                let row = Label::new(Some(&format!("   {} : {}", s.label, s.value)));
                row.set_xalign(0.0);
                content.append(&row);
            }
        })
    };
    refresh();
    {
        let refresh = refresh.clone();
        let win2 = win.clone();
        glib::timeout_add_seconds_local(1, move || {
            if !win2.is_visible() { return glib::ControlFlow::Break; }
            refresh();
            glib::ControlFlow::Continue
        });
    }
    {
        let win2 = win.clone();
        close.connect_clicked(move |_| win2.close());
    }
    win.present();
}

// ---------- Settings (LIVE-APPLY) ----------
fn open_settings(
    cfg: &Rc<RefCell<Config>>,
    provider: &Rc<CssProvider>,
    window: &ApplicationWindow,
    is_wayland: bool,
    metrics_box: &GtkBox,
    active: &Active,
) {
    let win = Window::builder()
        .title("TopMonitoring — Pengaturan")
        .default_width(560)
        .default_height(760)
        .build();

    let original = Rc::new(RefCell::new(cfg.borrow().clone()));

    // Terapkan cfg ke bar secara live (tanpa simpan ke disk)
    let apply_all: Rc<dyn Fn()> = {
        let (cfg, provider, window, metrics_box, active) =
            (cfg.clone(), provider.clone(), window.clone(), metrics_box.clone(), active.clone());
        Rc::new(move || {
            let c = cfg.borrow();
            provider.load_from_string(&build_css(&c, None));
            rebuild_bar_metrics(&metrics_box, &active, &c);
            window.set_default_height(c.height);
            if is_wayland { configure_wayland(&window, &c); } else { configure_x11(&window, &c); }
        })
    };

    let vbox = GtkBox::new(Orientation::Vertical, 8);
    vbox.set_margin_top(14);
    vbox.set_margin_bottom(14);
    vbox.set_margin_start(14);
    vbox.set_margin_end(14);

    // Tema (gelap/terang)
    let themes = ["dark", "light"];
    let theme_dd = DropDown::from_strings(&themes);
    theme_dd.set_selected(if cfg.borrow().theme == "light" { 1 } else { 0 });
    {
        let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
        theme_dd.connect_selected_notify(move |d| {
            cfg.borrow_mut().theme = if d.selected() == 1 { "light".into() } else { "dark".into() };
            apply_all();
        });
    }
    labeled_row(&vbox, "Tema topbar", &theme_dd);

    let positions = ["top", "bottom"];
    let pos_dd = DropDown::from_strings(&positions);
    pos_dd.set_selected(if cfg.borrow().position == "bottom" { 1 } else { 0 });
    {
        let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
        pos_dd.connect_selected_notify(move |d| {
            cfg.borrow_mut().position = if d.selected() == 1 { "bottom".into() } else { "top".into() };
            apply_all();
        });
    }
    labeled_row(&vbox, "Posisi", &pos_dd);

    let mut mon_labels = vec!["default".to_string()];
    if let Some(display) = gtk::gdk::Display::default() {
        let mons = display.monitors();
        for i in 0..mons.n_items() {
            if let Some(obj) = mons.item(i) {
                if let Ok(m) = obj.downcast::<gtk::gdk::Monitor>() {
                    let g = m.geometry();
                    mon_labels.push(format!("{i}: {}x{}", g.width(), g.height()));
                }
            }
        }
    }
    let mon_refs: Vec<&str> = mon_labels.iter().map(|s| s.as_str()).collect();
    let mon_dd = DropDown::from_strings(&mon_refs);
    mon_dd.set_selected(if cfg.borrow().monitor < 0 { 0 } else { cfg.borrow().monitor as u32 + 1 });
    {
        let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
        mon_dd.connect_selected_notify(move |d| {
            cfg.borrow_mut().monitor = if d.selected() == 0 { -1 } else { d.selected() as i32 - 1 };
            apply_all();
        });
    }
    labeled_row(&vbox, "Monitor", &mon_dd);

    let height_spin = SpinButton::with_range(20.0, 60.0, 1.0);
    height_spin.set_value(cfg.borrow().height as f64);
    {
        let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
        height_spin.connect_value_changed(move |s| { cfg.borrow_mut().height = s.value() as i32; apply_all(); });
    }
    labeled_row(&vbox, "Tinggi bar (px)", &height_spin);

    let margin_spin = SpinButton::with_range(0.0, 200.0, 1.0);
    margin_spin.set_value(cfg.borrow().margin_top as f64);
    {
        let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
        margin_spin.connect_value_changed(move |s| { cfg.borrow_mut().margin_top = s.value() as i32; apply_all(); });
    }
    labeled_row(&vbox, "Offset (px) — hindari panel XFCE", &margin_spin);

    let font_entry = Entry::new();
    font_entry.set_text(&cfg.borrow().font_family);
    {
        let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
        font_entry.connect_changed(move |e| { cfg.borrow_mut().font_family = e.text().to_string(); apply_all(); });
    }
    labeled_row(&vbox, "Font", &font_entry);

    let size_spin = SpinButton::with_range(6.0, 24.0, 1.0);
    size_spin.set_value(cfg.borrow().font_size as f64);
    {
        let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
        size_spin.connect_value_changed(move |s| { cfg.borrow_mut().font_size = s.value() as i32; apply_all(); });
    }
    labeled_row(&vbox, "Ukuran font", &size_spin);

    let interval_spin = SpinButton::with_range(200.0, 5000.0, 100.0);
    interval_spin.set_value(cfg.borrow().interval_ms as f64);
    {
        let cfg = cfg.clone();
        interval_spin.connect_value_changed(move |s| { cfg.borrow_mut().interval_ms = s.value() as u64; });
    }
    labeled_row(&vbox, "Interval (ms, perlu restart)", &interval_spin);

    let anim_sw = Switch::new();
    anim_sw.set_active(cfg.borrow().animated_bg);
    anim_sw.set_halign(gtk::Align::End);
    {
        let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
        anim_sw.connect_active_notify(move |s| { cfg.borrow_mut().animated_bg = s.is_active(); apply_all(); });
    }
    labeled_row(&vbox, "Background animasi (hue)", &anim_sw);

    let notif_sw = Switch::new();
    notif_sw.set_active(cfg.borrow().notifications);
    notif_sw.set_halign(gtk::Align::End);
    {
        let cfg = cfg.clone();
        notif_sw.connect_active_notify(move |s| { cfg.borrow_mut().notifications = s.is_active(); });
    }
    labeled_row(&vbox, "Notifikasi kritis", &notif_sw);

    let bg_entry = Entry::new();
    bg_entry.set_text(&cfg.borrow().custom_bg);
    bg_entry.set_placeholder_text(Some("kosong = pakai tema"));
    {
        let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
        bg_entry.connect_changed(move |e| { cfg.borrow_mut().custom_bg = e.text().to_string(); apply_all(); });
    }
    let color_btn = ColorDialogButton::new(Some(ColorDialog::new()));
    if let Ok(rgba) = cfg.borrow().custom_bg.parse::<gtk::gdk::RGBA>() { color_btn.set_rgba(&rgba); }
    {
        let bg_entry = bg_entry.clone();
        color_btn.connect_rgba_notify(move |b| { bg_entry.set_text(&b.rgba().to_string()); });
    }
    labeled_row(&vbox, "Warna background (live)", &color_btn);
    labeled_row(&vbox, "Background (teks)", &bg_entry);

    vbox.append(&Label::new(Some("CSS sendiri (pakai selector .topbar):")));
    let css_view = TextView::new();
    css_view.buffer().set_text(&cfg.borrow().custom_css);
    {
        let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
        css_view.buffer().connect_changed(move |b| {
            let (s, e) = b.bounds();
            cfg.borrow_mut().custom_css = b.text(&s, &e, false).to_string();
            apply_all();
        });
    }
    vbox.append(&ScrolledWindow::builder().child(&css_view).min_content_height(70).build());

    // Aksi
    let io_row = GtkBox::new(Orientation::Horizontal, 8);
    let export = Button::with_label("Export…");
    let import = Button::with_label("Import…");
    let autostart = Button::with_label("Pasang Autostart");
    let sensors_btn = Button::with_label("Sensor Hardware…");
    io_row.append(&export);
    io_row.append(&import);
    io_row.append(&autostart);
    io_row.append(&sensors_btn);
    vbox.append(&io_row);
    autostart.connect_clicked(move |b| { b.set_label(if install_autostart() { "Autostart ✓" } else { "Gagal" }); });
    sensors_btn.connect_clicked(|_| open_sensors());
    {
        let (cfg, win2) = (cfg.clone(), win.clone());
        export.connect_clicked(move |_| {
            let dialog = FileDialog::builder().title("Export").initial_name("topmonitoring-config.toml").build();
            let cfg = cfg.clone();
            dialog.save(Some(&win2), gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res { if let Some(path) = file.path() {
                    if let Ok(s) = toml::to_string_pretty(&*cfg.borrow()) { let _ = std::fs::write(path, s); }
                }}
            });
        });
    }
    {
        let (cfg, apply_all, win2) = (cfg.clone(), apply_all.clone(), win.clone());
        import.connect_clicked(move |_| {
            let dialog = FileDialog::builder().title("Import").build();
            let (cfg, apply_all, win3) = (cfg.clone(), apply_all.clone(), win2.clone());
            dialog.open(Some(&win2), gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res { if let Some(path) = file.path() {
                    if let Ok(s) = std::fs::read_to_string(path) {
                        if let Ok(newc) = toml::from_str::<Config>(&s) {
                            *cfg.borrow_mut() = newc;
                            cfg.borrow().save();
                            apply_all();
                            win3.close();
                        }
                    }
                }}
            });
        });
    }

    // Daftar metrik (live)
    vbox.append(&Label::new(Some("Metrik — kolom perintah = app yang dibuka saat metrik diklik:")));
    let rows_box = GtkBox::new(Orientation::Vertical, 4);
    vbox.append(&rows_box);

    let repop: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    {
        let (rows_box, cfg, apply_all, repop2) = (rows_box.clone(), cfg.clone(), apply_all.clone(), repop.clone());
        *repop.borrow_mut() = Some(Box::new(move || {
            while let Some(child) = rows_box.first_child() { rows_box.remove(&child); }
            let len = cfg.borrow().metrics.len();
            for i in 0..len {
                let m = cfg.borrow().metrics[i].clone();
                let row = GtkBox::new(Orientation::Horizontal, 6);
                row.append(&Label::new(Some(&m.id)));

                let name = Entry::new();
                name.set_text(&m.label);
                name.set_placeholder_text(Some(default_prefix(&m.id)));
                name.set_width_chars(6);
                {
                    let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
                    name.connect_changed(move |e| {
                        if let Some(mm) = cfg.borrow_mut().metrics.get_mut(i) { mm.label = e.text().to_string(); }
                        apply_all();
                    });
                }
                row.append(&name);

                let cmd = Entry::new();
                cmd.set_text(&m.command);
                cmd.set_placeholder_text(Some("klik → jalankan (opsional)"));
                cmd.set_tooltip_text(Some("Perintah shell yang dijalankan saat metrik ini diklik. Contoh: xfce4-taskmanager"));
                cmd.set_hexpand(true);
                {
                    let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
                    cmd.connect_changed(move |e| {
                        if let Some(mm) = cfg.borrow_mut().metrics.get_mut(i) { mm.command = e.text().to_string(); }
                        apply_all();
                    });
                }
                row.append(&cmd);

                let sw = Switch::new();
                sw.set_active(m.enabled);
                {
                    let (cfg, apply_all) = (cfg.clone(), apply_all.clone());
                    sw.connect_active_notify(move |s| {
                        if let Some(mm) = cfg.borrow_mut().metrics.get_mut(i) { mm.enabled = s.is_active(); }
                        apply_all();
                    });
                }
                row.append(&sw);

                let up = Button::with_label("↑");
                {
                    let (cfg, apply_all, repop2) = (cfg.clone(), apply_all.clone(), repop2.clone());
                    up.connect_clicked(move |_| {
                        if i > 0 { cfg.borrow_mut().metrics.swap(i, i - 1); }
                        if let Some(f) = repop2.borrow().as_ref() { f(); }
                        apply_all();
                    });
                }
                let down = Button::with_label("↓");
                {
                    let (cfg, apply_all, repop2) = (cfg.clone(), apply_all.clone(), repop2.clone());
                    down.connect_clicked(move |_| {
                        let len = cfg.borrow().metrics.len();
                        if i + 1 < len { cfg.borrow_mut().metrics.swap(i, i + 1); }
                        if let Some(f) = repop2.borrow().as_ref() { f(); }
                        apply_all();
                    });
                }
                row.append(&up);
                row.append(&down);
                rows_box.append(&row);
            }
        }));
    }
    if let Some(f) = repop.borrow().as_ref() { f(); }

    // Modul custom (live)
    vbox.append(&Label::new(Some("Modul custom (nama · label · perintah shell):")));
    let cm_box = GtkBox::new(Orientation::Vertical, 4);
    vbox.append(&cm_box);
    let repop_cm: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    {
        let (cm_box, cfg, apply_all, repop_cm2) = (cm_box.clone(), cfg.clone(), apply_all.clone(), repop_cm.clone());
        *repop_cm.borrow_mut() = Some(Box::new(move || {
            while let Some(child) = cm_box.first_child() { cm_box.remove(&child); }
            let len = cfg.borrow().custom_modules.len();
            for i in 0..len {
                let m = cfg.borrow().custom_modules[i].clone();
                let row = GtkBox::new(Orientation::Horizontal, 6);

                let name = Entry::new();
                name.set_text(&m.name);
                name.set_placeholder_text(Some("nama"));
                name.set_width_chars(6);
                { let (cfg, apply_all) = (cfg.clone(), apply_all.clone()); name.connect_changed(move |e| { if let Some(mm) = cfg.borrow_mut().custom_modules.get_mut(i) { mm.name = e.text().to_string(); } apply_all(); }); }
                let lbl = Entry::new();
                lbl.set_text(&m.label);
                lbl.set_placeholder_text(Some("label"));
                lbl.set_width_chars(5);
                { let (cfg, apply_all) = (cfg.clone(), apply_all.clone()); lbl.connect_changed(move |e| { if let Some(mm) = cfg.borrow_mut().custom_modules.get_mut(i) { mm.label = e.text().to_string(); } apply_all(); }); }
                let cmd = Entry::new();
                cmd.set_text(&m.command);
                cmd.set_placeholder_text(Some("perintah"));
                cmd.set_hexpand(true);
                { let (cfg, apply_all) = (cfg.clone(), apply_all.clone()); cmd.connect_changed(move |e| { if let Some(mm) = cfg.borrow_mut().custom_modules.get_mut(i) { mm.command = e.text().to_string(); } apply_all(); }); }
                let sw = Switch::new();
                sw.set_active(m.enabled);
                { let (cfg, apply_all) = (cfg.clone(), apply_all.clone()); sw.connect_active_notify(move |s| { if let Some(mm) = cfg.borrow_mut().custom_modules.get_mut(i) { mm.enabled = s.is_active(); } apply_all(); }); }
                let del = Button::with_label("✕");
                { let (cfg, apply_all, repop_cm2) = (cfg.clone(), apply_all.clone(), repop_cm2.clone()); del.connect_clicked(move |_| { if i < cfg.borrow().custom_modules.len() { cfg.borrow_mut().custom_modules.remove(i); } if let Some(f) = repop_cm2.borrow().as_ref() { f(); } apply_all(); }); }
                row.append(&name); row.append(&lbl); row.append(&cmd); row.append(&sw); row.append(&del);
                cm_box.append(&row);
            }
        }));
    }
    if let Some(f) = repop_cm.borrow().as_ref() { f(); }
    let add_cm = Button::with_label("+ Tambah modul");
    vbox.append(&add_cm);
    {
        let (cfg, repop_cm, apply_all) = (cfg.clone(), repop_cm.clone(), apply_all.clone());
        add_cm.connect_clicked(move |_| {
            cfg.borrow_mut().custom_modules.push(CustomModule { name: "modul".into(), label: "".into(), command: "echo hi".into(), enabled: true });
            if let Some(f) = repop_cm.borrow().as_ref() { f(); }
            apply_all();
        });
    }

    // Simpan / Reset
    let btn_row = GtkBox::new(Orientation::Horizontal, 8);
    let save = Button::with_label("💾 Simpan");
    let reset = Button::with_label("Reset default");
    btn_row.append(&save);
    btn_row.append(&reset);
    vbox.append(&btn_row);
    {
        let (cfg, original) = (cfg.clone(), original.clone());
        save.connect_clicked(move |b| {
            cfg.borrow().save();
            *original.borrow_mut() = cfg.borrow().clone();
            b.set_label("💾 Tersimpan ✓");
        });
    }
    {
        let (cfg, original, apply_all, win2) = (cfg.clone(), original.clone(), apply_all.clone(), win.clone());
        reset.connect_clicked(move |_| {
            *cfg.borrow_mut() = Config::default();
            cfg.borrow().save();
            *original.borrow_mut() = cfg.borrow().clone();
            apply_all();
            win2.close();
        });
    }
    
        let quit = Button::with_label("⏻ Keluar aplikasi");
    btn_row.append(&quit);
    {
        let window = window.clone();
        quit.connect_clicked(move |_| {
            if let Some(app) = window.application() {
                app.quit();
            }
        });
    }

    // Kalau ditutup tanpa Simpan → balik ke kondisi terakhir tersimpan
    {
        let (cfg, original, apply_all) = (cfg.clone(), original.clone(), apply_all.clone());
        win.connect_close_request(move |_| {
            *cfg.borrow_mut() = original.borrow().clone();
            apply_all();
            glib::Propagation::Proceed
        });
    }

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(true)
        .child(&vbox)
        .build();
    win.set_child(Some(&scroll));
    win.present();
}

fn labeled_row(parent: &GtkBox, text: &str, w: &impl IsA<gtk::Widget>) {
    let row = GtkBox::new(Orientation::Horizontal, 8);
    let l = Label::new(Some(text));
    l.set_hexpand(true);
    l.set_xalign(0.0);
    row.append(&l);
    row.append(w);
    parent.append(&row);
}
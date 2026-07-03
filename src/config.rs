use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct MetricConf {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub warn: f64,
    #[serde(default)]
    pub crit: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CustomModule {
    pub name: String,
    pub label: String,
    pub command: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub height: i32,
    pub margin_top: i32,
    pub position: String,
    pub monitor: i32,
    pub theme: String,
    pub custom_bg: String,
    pub custom_css: String,
    pub animated_bg: bool,
    pub notifications: bool,
    pub font_family: String,
    pub font_size: i32,
    pub interval_ms: u64,
    #[serde(default = "default_metrics")]
    pub metrics: Vec<MetricConf>,
    #[serde(default)]
    pub custom_modules: Vec<CustomModule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            height: 30,
            margin_top: 0,
            position: "top".into(),
            monitor: -1,
            theme: "dark".into(),
            custom_bg: String::new(),
            custom_css: String::new(),
            animated_bg: false,
            notifications: true,
            font_family: "JetBrains Mono, monospace".into(),
            font_size: 12,
            interval_ms: 1000,
            metrics: default_metrics(),
            custom_modules: Vec::new(),
        }
    }
}

fn mc(id: &str, label: &str, enabled: bool) -> MetricConf {
    MetricConf { id: id.into(), label: label.into(), enabled, command: String::new(), warn: 0.0, crit: 0.0 }
}

pub fn default_metrics() -> Vec<MetricConf> {
    vec![
        mc("clock", "", true),
        mc("date", "", false),
        mc("cpu", "CPU", true),
        mc("cpu_graph", "", false),
        mc("cpumodel", "", false),
        mc("freq", "FREQ", true),
        mc("cpu_power", "PKG-W", false),
        mc("vcore", "Vcore", false),
        mc("memory", "RAM", true),
        mc("ram_graph", "", false),
        mc("memavail", "FREE", false),
        mc("swap", "SWAP", false),
        mc("temp", "TEMP", true),
        mc("fan", "FAN", true),
        mc("gpu", "GPU", true),
        mc("gpu_power", "GPU-W", false),
        mc("gpu_clock", "GCLK", false),
        mc("gpu_memclock", "GMEM", false),
        mc("gpu_fan", "GFAN", false),
        mc("disk", "DISK", true),
        mc("diskio", "IO", false),
        mc("network", "NET", true),
        mc("net_graph", "", false),
        mc("netttl", "", false),
        mc("battery", "BAT", true),
        mc("procs", "PROC", false),
        mc("uptime", "UP", true),
        mc("load", "LOAD", true),
        mc("host", "", false),
        mc("kernel", "KERN", false),
        mc("os", "OS", false),
    ]
}

impl Config {
    pub fn path() -> PathBuf {
        let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        p.push("topmonitoring");
        p.push("config.toml");
        p
    }
    pub fn load() -> Self {
        let mut cfg = match fs::read_to_string(Self::path()) {
            Ok(s) => toml::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        for dm in default_metrics() {
            if !cfg.metrics.iter().any(|m| m.id == dm.id) {
                cfg.metrics.push(dm);
            }
        }
        cfg
    }
    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = fs::write(&path, s);
        }
    }
}

// CSS di-scope ke ".topbar" → hanya window topbar yang terpengaruh, bukan settings.
pub fn build_css(cfg: &Config, hue: Option<f64>) -> String {
    let (base_bg, fg, accent) = match cfg.theme.as_str() {
        "light" => ("rgba(245,245,245,0.96)", "#202020", "#0060df"),
        _ => ("rgba(18,18,24,0.94)", "#e6e6e6", "#00d0ff"),
    };
    let bg = if let Some(h) = hue {
        format!("hsl({h:.0}, 65%, 20%)")
    } else if !cfg.custom_bg.trim().is_empty() {
        cfg.custom_bg.trim().to_string()
    } else {
        base_bg.to_string()
    };
    let font = if cfg.font_family.trim().is_empty() { "monospace" } else { cfg.font_family.trim() };
    let fs = cfg.font_size.max(6);
    let mut css = format!(
        ".topbar {{ background-color: {bg}; }}\n\
         .topbar label {{ color: {fg}; font-family: {font}; font-size: {fs}px; font-weight: 600; padding: 0 6px; }}\n\
         .topbar .accent {{ color: {accent}; }}\n\
         .topbar .warn  color: #ffb020; \n\
         .topbar .crit  color: #ff4040; \n\
         .topbar button {{ background: transparent; color: {fg}; border: none; padding: 0 8px; }}"
    );
    if !cfg.custom_css.trim().is_empty() {
        css.push('\n');
        css.push_str(cfg.custom_css.trim());
    }
    css
}

pub fn default_prefix(id: &str) -> &'static str {
    match id {
        "cpu" => "CPU",
        "freq" => "FREQ",
        "cpu_power" => "PKG-W",
        "vcore" => "Vcore",
        "memory" => "RAM",
        "memavail" => "FREE",
        "swap" => "SWAP",
        "temp" => "TEMP",
        "fan" => "FAN",
        "gpu" => "GPU",
        "gpu_power" => "GPU-W",
        "gpu_clock" => "GCLK",
        "gpu_memclock" => "GMEM",
        "gpu_fan" => "GFAN",
        "disk" => "DISK",
        "diskio" => "IO",
        "network" => "NET",
        "battery" => "BAT",
        "procs" => "PROC",
        "uptime" => "UP",
        "load" => "LOAD",
        "kernel" => "KERN",
        "os" => "OS",
        _ => "",
    }
}
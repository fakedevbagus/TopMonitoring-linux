use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Settings for a single metric shown on the bar.
#[derive(Serialize, Deserialize, Clone)]
pub struct MetricConf {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    /// Shell command executed when the user left-clicks this metric (optional).
    #[serde(default)]
    pub command: String,
    /// Custom "warning" threshold. 0 = use the metric's built-in default.
    #[serde(default)]
    pub warn: f64,
    /// Custom "critical" threshold. 0 = use the metric's built-in default.
    #[serde(default)]
    pub crit: f64,
}

/// A user-defined module that runs a shell command and displays its output.
#[derive(Serialize, Deserialize, Clone)]
pub struct CustomModule {
    pub name: String,
    pub label: String,
    pub command: String,
    pub enabled: bool,
}

/// A saved appearance preset (theme, colors, CSS, font) that can be
/// re-applied to the bar with a single click.
#[derive(Serialize, Deserialize, Clone)]
pub struct ThemePreset {
    pub name: String,
    pub theme: String,
    pub custom_bg: String,
    pub custom_css: String,
    pub font_family: String,
    pub font_size: i32,
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
    /// When true, the bar fades to low opacity while the pointer is away
    /// and returns to full opacity on hover.
    pub auto_hide: bool,
    pub notifications: bool,
    pub font_family: String,
    pub font_size: i32,
    pub interval_ms: u64,
    #[serde(default = "default_metrics")]
    pub metrics: Vec<MetricConf>,
    #[serde(default)]
    pub custom_modules: Vec<CustomModule>,
    #[serde(default = "default_presets")]
    pub presets: Vec<ThemePreset>,
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
            auto_hide: false,
            notifications: true,
            font_family: "JetBrains Mono, monospace".into(),
            font_size: 12,
            interval_ms: 1000,
            metrics: default_metrics(),
            custom_modules: Vec::new(),
            presets: default_presets(),
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

pub fn default_presets() -> Vec<ThemePreset> {
    let font = "JetBrains Mono, monospace".to_string();
    vec![
        ThemePreset { name: "Midnight Cyan".into(), theme: "dark".into(), custom_bg: String::new(), custom_css: String::new(), font_family: font.clone(), font_size: 12 },
        ThemePreset { name: "Ocean".into(), theme: "dark".into(), custom_bg: "rgba(10,35,58,0.95)".into(), custom_css: String::new(), font_family: font.clone(), font_size: 12 },
        ThemePreset { name: "Sunset".into(), theme: "dark".into(), custom_bg: "rgba(45,20,15,0.95)".into(), custom_css: String::new(), font_family: font.clone(), font_size: 12 },
        ThemePreset { name: "Light Clean".into(), theme: "light".into(), custom_bg: String::new(), custom_css: String::new(), font_family: font, font_size: 12 },
    ]
}

impl Config {
    pub fn path() -> PathBuf {
        let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        p.push("topmonitoring");
        p.push("config.toml");
        p
    }

    /// Load configuration from disk. If the file is missing, defaults are used.
    /// If the file exists but fails to parse, the broken file is preserved as a
    /// `.bak` copy and defaults are used instead, so the app always starts.
    pub fn load() -> Self {
        let path = Self::path();
        let mut cfg = match fs::read_to_string(&path) {
            Ok(raw) => match toml::from_str::<Config>(&raw) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "[TopMonitoring] Could not parse {}: {e}. Falling back to defaults; the broken file was backed up to config.toml.bak",
                        path.display()
                    );
                    let _ = fs::write(path.with_extension("toml.bak"), &raw);
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        };
        // Merge in metrics introduced by newer app versions without
        // discarding the user's existing customization.
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

/// Build the CSS applied to the topbar window only. Every rule is scoped
/// under the `.topbar` class so the Settings and Hardware Sensors windows
/// keep the desktop's own GTK theme instead of following the bar's theme.
///
/// NOTE: this is built one rule at a time with small, simple `format!`
/// calls (instead of one giant literal) specifically to avoid a previous
/// bug where a stray `{N}` placeholder silently broke the whole stylesheet.
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
    let font = if cfg.font_family.trim().is_empty() {
        "monospace".to_string()
    } else {
        cfg.font_family.trim().to_string()
    };
    let fs = cfg.font_size.max(6);

    let mut css = String::new();
    css.push_str(&format!(".topbar {{ background-color: {bg}; }}\n"));
    css.push_str(".topbar.dimmed { opacity: 0.18; }\n");
    css.push_str(&format!(
        ".topbar label {{ color: {fg}; font-family: {font}; font-size: {fs}px; font-weight: 600; padding: 0 6px; }}\n"
    ));
    css.push_str(&format!(".topbar .accent {{ color: {accent}; }}\n"));
    css.push_str(".topbar .warn { color: #ffb020; }\n");
    css.push_str(".topbar .crit { color: #ff4040; }\n");
    // The "blink-off" class is toggled on/off every 500ms by main.rs for
    // any label currently marked critical, producing a blinking effect.
    css.push_str(".topbar .crit.blink-off { color: #802020; }\n");
    css.push_str(&format!(
        ".topbar button {{ background: transparent; color: {fg}; border: none; padding: 0 8px; }}\n"
    ));

    if !cfg.custom_css.trim().is_empty() {
        css.push('\n');
        css.push_str(cfg.custom_css.trim());
    }
    css
}

/// Default label shown before a metric's value, unless the user renamed it.
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
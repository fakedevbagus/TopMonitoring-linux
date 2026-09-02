use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CONFIG_SCHEMA_VERSION: u32 = 2;

fn default_true() -> bool {
    true
}
fn default_zone() -> String {
    "center".into()
}
fn default_priority() -> i32 {
    50
}
fn default_module_interval() -> u64 {
    5_000
}
fn default_command_timeout() -> u64 {
    2_000
}
fn default_output_cap() -> usize {
    16 * 1024
}
fn default_history_interval() -> u64 {
    60
}

fn default_history_max_mb() -> u32 {
    32
}
fn default_schema_version() -> u32 {
    CONFIG_SCHEMA_VERSION
}
fn default_effects_kind() -> String {
    "off".into()
}
fn default_effects_fps_cap() -> u32 {
    30
}
fn default_effects_intensity() -> f64 {
    0.6
}
fn default_effects_trail() -> f64 {
    0.5
}
fn default_effects_thickness() -> f64 {
    3.0
}
fn default_effects_speed() -> f64 {
    1.0
}
fn default_font_weight() -> i32 {
    600
}
fn default_border_strength() -> f64 {
    0.10
}
fn default_module_gap() -> i32 {
    4
}
fn default_module_padding() -> i32 {
    7
}
fn default_module_radius() -> i32 {
    7
}
fn default_bar_opacity() -> f64 {
    0.96
}
fn default_accent() -> String {
    "#5E9FE8".into()
}
fn default_surface() -> String {
    "rgba(255,255,255,0.06)".into()
}
fn default_warning() -> String {
    "#EAC26B".into()
}
fn default_critical() -> String {
    "#E97366".into()
}

/// Settings for one built-in metric.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct MetricConf {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    /// Optional shell command executed on primary click.
    pub command: String,
    /// Imported commands are disabled until explicitly trusted.
    #[serde(default = "default_true")]
    pub command_trusted: bool,
    pub warn: f64,
    pub crit: f64,
    /// `start`, `center`, or `end` bar zone.
    #[serde(default = "default_zone")]
    pub zone: String,
    /// Higher priority survives narrow-width overflow first.
    #[serde(default = "default_priority")]
    pub priority: i32,
    /// Allow a compact renderer when space is constrained.
    #[serde(default = "default_true")]
    pub compact: bool,
    /// Optional display template reserved for richer renderers.
    pub format: String,
    /// Optional per-module foreground/background CSS colors.
    pub fg_color: String,
    pub bg_color: String,
}

impl Default for MetricConf {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            enabled: false,
            command: String::new(),
            command_trusted: true,
            warn: 0.0,
            crit: 0.0,
            zone: default_zone(),
            priority: default_priority(),
            compact: true,
            format: String::new(),
            fg_color: String::new(),
            bg_color: String::new(),
        }
    }
}

/// A user-defined module whose command output is rendered on the bar.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CustomModule {
    pub name: String,
    pub label: String,
    /// Optional display template with `{label}` and `{value}` placeholders.
    pub format: String,
    pub command: String,
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub trusted: bool,
    #[serde(default = "default_zone")]
    pub zone: String,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub compact: bool,
    #[serde(default = "default_module_interval")]
    pub interval_ms: u64,
    #[serde(default = "default_command_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_output_cap")]
    pub max_output_bytes: usize,
    pub tooltip: String,
    pub click_command: String,
    pub fg_color: String,
    pub bg_color: String,
}

impl Default for CustomModule {
    fn default() -> Self {
        Self {
            name: "module".into(),
            label: String::new(),
            format: String::new(),
            command: "echo ready".into(),
            enabled: true,
            trusted: true,
            zone: default_zone(),
            priority: default_priority(),
            compact: true,
            interval_ms: default_module_interval(),
            timeout_ms: default_command_timeout(),
            max_output_bytes: default_output_cap(),
            tooltip: String::new(),
            click_command: String::new(),
            fg_color: String::new(),
            bg_color: String::new(),
        }
    }
}

/// A saved appearance preset.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ThemePreset {
    pub name: String,
    pub theme: String,
    pub custom_bg: String,
    pub custom_css: String,
    pub font_family: String,
    pub font_size: i32,
    pub accent_color: String,
    pub surface_color: String,
    pub module_gap: i32,
    pub module_padding: i32,
    pub module_radius: i32,
    #[serde(default = "default_font_weight")]
    pub font_weight: i32,
    #[serde(default = "default_border_strength")]
    pub border_strength: f64,
}

impl Default for ThemePreset {
    fn default() -> Self {
        Self {
            name: "Preset".into(),
            theme: "dark".into(),
            custom_bg: String::new(),
            custom_css: String::new(),
            font_family: "JetBrains Mono, monospace".into(),
            font_size: 12,
            accent_color: default_accent(),
            surface_color: default_surface(),
            module_gap: default_module_gap(),
            module_padding: default_module_padding(),
            module_radius: default_module_radius(),
            font_weight: default_font_weight(),
            border_strength: default_border_strength(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub height: i32,
    pub margin_top: i32,
    pub position: String,
    pub monitor: i32,
    pub theme: String,
    pub custom_bg: String,
    pub custom_css: String,
    pub animated_bg: bool,
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
    pub net_iface: String,
    pub gpu_index: usize,
    pub qa_lock: bool,
    pub qa_shutdown: bool,
    pub qa_screenshot: bool,
    pub history_enabled: bool,
    #[serde(default = "default_history_interval")]
    pub history_interval_secs: u64,
    /// Rotates history.csv before it grows beyond this size.
    #[serde(default = "default_history_max_mb")]
    pub history_max_mb: u32,
    #[serde(default = "default_true")]
    pub check_updates: bool,
    #[serde(default = "default_module_gap")]
    pub module_gap: i32,
    #[serde(default = "default_module_padding")]
    pub module_padding: i32,
    #[serde(default = "default_module_radius")]
    pub module_radius: i32,
    #[serde(default = "default_bar_opacity")]
    pub bar_opacity: f64,
    #[serde(default = "default_accent")]
    pub accent_color: String,
    #[serde(default = "default_surface")]
    pub surface_color: String,
    #[serde(default = "default_warning")]
    pub warning_color: String,
    #[serde(default = "default_critical")]
    pub critical_color: String,
    pub reduce_motion: bool,
    #[serde(default = "default_true")]
    pub blink_critical: bool,
    #[serde(default = "default_true")]
    pub adaptive_compact: bool,
    // Visual effect engine (Step 0.2). Flat fields with serde defaults keep
    // older v2 configs parsing; unknown values are clamped at validation.
    #[serde(default)]
    pub effects_enabled: bool,
    #[serde(default = "default_effects_kind")]
    pub effects_kind: String,
    #[serde(default = "default_effects_fps_cap")]
    pub effects_fps_cap: u32,
    #[serde(default = "default_effects_intensity")]
    pub effects_intensity: f64,
    #[serde(default = "default_effects_trail")]
    pub effects_trail: f64,
    #[serde(default = "default_effects_thickness")]
    pub effects_thickness: f64,
    #[serde(default = "default_effects_speed")]
    pub effects_speed: f64,
    // Theme Studio tokens (Step 0.4): label font weight and module border
    // strength. Serde defaults keep older configs parsing unchanged.
    #[serde(default = "default_font_weight")]
    pub font_weight: i32,
    #[serde(default = "default_border_strength")]
    pub border_strength: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            height: 34,
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
            interval_ms: 1_000,
            metrics: default_metrics(),
            custom_modules: Vec::new(),
            presets: default_presets(),
            net_iface: String::new(),
            gpu_index: 0,
            qa_lock: false,
            qa_shutdown: false,
            qa_screenshot: false,
            history_enabled: false,
            history_interval_secs: default_history_interval(),
            history_max_mb: default_history_max_mb(),
            check_updates: true,
            module_gap: default_module_gap(),
            module_padding: default_module_padding(),
            module_radius: default_module_radius(),
            bar_opacity: default_bar_opacity(),
            accent_color: default_accent(),
            surface_color: default_surface(),
            warning_color: default_warning(),
            critical_color: default_critical(),
            reduce_motion: false,
            blink_critical: true,
            adaptive_compact: true,
            effects_enabled: false,
            effects_kind: default_effects_kind(),
            effects_fps_cap: default_effects_fps_cap(),
            effects_intensity: default_effects_intensity(),
            effects_trail: default_effects_trail(),
            effects_thickness: default_effects_thickness(),
            effects_speed: default_effects_speed(),
            font_weight: default_font_weight(),
            border_strength: default_border_strength(),
        }
    }
}

fn metric(id: &str, label: &str, enabled: bool, zone: &str, priority: i32) -> MetricConf {
    MetricConf {
        id: id.into(),
        label: label.into(),
        enabled,
        zone: zone.into(),
        priority,
        ..MetricConf::default()
    }
}

pub fn default_metrics() -> Vec<MetricConf> {
    vec![
        metric("clock", "", true, "start", 100),
        metric("date", "", false, "start", 80),
        metric("host", "", false, "start", 30),
        metric("os", "OS", false, "start", 20),
        metric("kernel", "KERN", false, "start", 10),
        metric("cpu", "CPU", true, "center", 100),
        metric("cpu_graph", "", false, "center", 55),
        metric("cpumodel", "", false, "center", 10),
        metric("freq", "FREQ", true, "center", 45),
        metric("cpu_power", "PKG-W", false, "center", 30),
        metric("vcore", "Vcore", false, "center", 15),
        metric("memory", "RAM", true, "center", 95),
        metric("ram_graph", "", false, "center", 50),
        metric("memavail", "FREE", false, "center", 35),
        metric("swap", "SWAP", false, "center", 25),
        metric("temp", "TEMP", true, "center", 90),
        metric("fan", "FAN", false, "center", 25),
        metric("gpu", "GPU", true, "center", 85),
        metric("gpu_power", "GPU-W", false, "center", 25),
        metric("gpu_clock", "GCLK", false, "center", 20),
        metric("gpu_memclock", "GMEM", false, "center", 15),
        metric("gpu_fan", "GFAN", false, "center", 20),
        metric("disk", "DISK", true, "center", 80),
        metric("diskio", "IO", false, "center", 35),
        metric("network", "NET", true, "center", 85),
        metric("net_graph", "", false, "center", 40),
        metric("netttl", "", false, "center", 15),
        metric("media", "", false, "end", 40),
        metric("wifi", "WIFI", false, "end", 35),
        metric("vpn", "VPN", false, "end", 60),
        metric("bluetooth", "BT", false, "end", 25),
        metric("battery", "BAT", true, "end", 90),
        metric("volume", "", false, "end", 75),
        metric("brightness", "", false, "end", 65),
        metric("pkg_updates", "UPD", false, "end", 20),
        metric("procs", "PROC", false, "end", 10),
        metric("uptime", "UP", true, "end", 45),
        metric("load", "LOAD", false, "end", 30),
    ]
}

pub fn default_presets() -> Vec<ThemePreset> {
    let base = ThemePreset::default();
    vec![
        ThemePreset {
            name: "Calm Telemetry".into(),
            ..base.clone()
        },
        ThemePreset {
            name: "Ocean".into(),
            custom_bg: "rgba(10,35,58,0.96)".into(),
            accent_color: "#4FB9C9".into(),
            ..base.clone()
        },
        ThemePreset {
            name: "Sunset".into(),
            custom_bg: "rgba(45,24,20,0.96)".into(),
            accent_color: "#DE9255".into(),
            ..base.clone()
        },
        ThemePreset {
            name: "Light Clean".into(),
            theme: "light".into(),
            accent_color: "#2783DE".into(),
            surface_color: "rgba(0,0,0,0.05)".into(),
            ..base
        },
    ]
}

impl Config {
    pub fn path() -> PathBuf {
        let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("topmonitoring");
        path.push("config.toml");
        path
    }

    pub fn load() -> Self {
        let path = Self::path();
        let mut config = match fs::read_to_string(&path) {
            Ok(raw) => match toml::from_str::<Config>(&raw) {
                Ok(config) => config,
                Err(error) => {
                    eprintln!(
                        "[TopMonitoring] Could not parse {}: {error}. Using defaults; invalid data was backed up.",
                        path.display()
                    );
                    let _ = fs::write(path.with_extension("toml.bak"), raw);
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        };
        config.migrate_and_validate();
        config
    }

    pub fn from_import(raw: &str) -> Result<Self, String> {
        let mut config = toml::from_str::<Config>(raw)
            .map_err(|error| format!("Invalid TopMonitoring configuration: {error}"))?;
        config.migrate_and_validate();
        config.sanitize_imported_commands();
        Ok(config)
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self)
            .map_err(|error| format!("Could not serialize settings: {error}"))
    }

    /// Crash-safe save: write and sync a temporary file, then atomically rename.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        let directory = path
            .parent()
            .ok_or_else(|| "Configuration path has no parent directory".to_string())?;
        fs::create_dir_all(directory)
            .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;

        let serialized = self.to_toml()?;
        let temporary = path.with_extension("toml.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("Could not open {}: {error}", temporary.display()))?;
        file.write_all(serialized.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Could not write {}: {error}", temporary.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("Could not secure {}: {error}", temporary.display()))?;
        }

        fs::rename(&temporary, &path)
            .map_err(|error| format!("Could not replace {}: {error}", path.display()))?;
        if let Ok(directory_handle) = fs::File::open(directory) {
            let _ = directory_handle.sync_all();
        }
        Ok(())
    }

    pub fn migrate_and_validate(&mut self) {
        self.schema_version = CONFIG_SCHEMA_VERSION;
        self.height = self.height.clamp(24, 64);
        self.margin_top = self.margin_top.clamp(0, 500);
        self.font_size = self.font_size.clamp(8, 24);
        self.interval_ms = self.interval_ms.clamp(250, 10_000);
        self.history_interval_secs = self.history_interval_secs.clamp(5, 86_400);
        self.history_max_mb = self.history_max_mb.clamp(1, 1_024);
        self.module_gap = self.module_gap.clamp(0, 24);
        self.module_padding = self.module_padding.clamp(0, 18);
        self.module_radius = self.module_radius.clamp(0, 18);
        self.bar_opacity = self.bar_opacity.clamp(0.35, 1.0);
        self.gpu_index = self.gpu_index.min(31);
        self.effects_fps_cap = self.effects_fps_cap.clamp(1, 60);
        self.effects_intensity = self.effects_intensity.clamp(0.0, 2.0);
        self.effects_trail = self.effects_trail.clamp(0.0, 1.0);
        self.effects_thickness = self.effects_thickness.clamp(1.0, 12.0);
        self.effects_speed = self.effects_speed.clamp(0.1, 4.0);
        self.font_weight = self.font_weight.clamp(100, 900);
        self.border_strength = self.border_strength.clamp(0.0, 0.6);
        if !matches!(
            self.effects_kind.as_str(),
            "off"
                | "orbit-glow"
                | "ember-orbit"
                | "aurora-sweep"
                | "critical-beacon"
                | "static-fallback"
        ) {
            self.effects_kind = "off".into();
        }
        if !matches!(self.position.as_str(), "top" | "bottom") {
            self.position = "top".into();
        }
        if !matches!(self.theme.as_str(), "dark" | "light") {
            self.theme = "dark".into();
        }
        self.accent_color = safe_color_or(&self.accent_color, &default_accent());
        self.surface_color = safe_color_or(&self.surface_color, &default_surface());
        self.warning_color = safe_color_or(&self.warning_color, &default_warning());
        self.critical_color = safe_color_or(&self.critical_color, &default_critical());
        if !self.custom_bg.trim().is_empty() {
            self.custom_bg = safe_color_or(&self.custom_bg, "");
        }

        let mut seen = HashSet::new();
        self.metrics
            .retain(|metric| !metric.id.trim().is_empty() && seen.insert(metric.id.clone()));
        for metric in &mut self.metrics {
            metric.zone = valid_zone(&metric.zone).into();
            metric.priority = metric.priority.clamp(0, 100);
            metric.fg_color = safe_optional_color(&metric.fg_color);
            metric.bg_color = safe_optional_color(&metric.bg_color);
        }
        for default_metric in default_metrics() {
            if !self
                .metrics
                .iter()
                .any(|metric| metric.id == default_metric.id)
            {
                self.metrics.push(default_metric);
            }
        }

        let mut custom_names = HashSet::new();
        for (index, module) in self.custom_modules.iter_mut().enumerate() {
            let base_name = if module.name.trim().is_empty() {
                format!("module-{}", index + 1)
            } else {
                module.name.trim().to_string()
            };
            let mut candidate = base_name.clone();
            let mut suffix = 2;
            while !custom_names.insert(candidate.clone()) {
                candidate = format!("{base_name} {suffix}");
                suffix += 1;
            }
            module.name = candidate;
            module.zone = valid_zone(&module.zone).into();
            module.priority = module.priority.clamp(0, 100);
            module.interval_ms = module.interval_ms.clamp(250, 3_600_000);
            module.timeout_ms = module.timeout_ms.clamp(100, 60_000);
            module.max_output_bytes = module.max_output_bytes.clamp(256, 65_536);
            module.fg_color = safe_optional_color(&module.fg_color);
            module.bg_color = safe_optional_color(&module.bg_color);
        }
    }

    /// Imported executable behavior is always opt-in, even when `enabled=true`
    /// was present in the imported file.
    pub fn sanitize_imported_commands(&mut self) {
        for metric in &mut self.metrics {
            if !metric.command.trim().is_empty() {
                metric.command_trusted = false;
            }
        }
        for module in &mut self.custom_modules {
            if !module.command.trim().is_empty() || !module.click_command.trim().is_empty() {
                module.trusted = false;
                module.enabled = false;
            }
        }
    }
}

fn valid_zone(zone: &str) -> &'static str {
    match zone {
        "start" => "start",
        "end" => "end",
        _ => "center",
    }
}

fn safe_optional_color(value: &str) -> String {
    if value.trim().is_empty() {
        String::new()
    } else {
        safe_color_or(value, "")
    }
}

fn safe_color_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    let allowed = !value.is_empty()
        && value.len() <= 96
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "#(),.% -_".contains(character))
        && !value.contains(';')
        && !value.contains('{')
        && !value.contains('}');
    if allowed {
        value.to_string()
    } else {
        fallback.to_string()
    }
}

fn css_font(value: &str) -> String {
    let trimmed = value.trim();
    let fallback = if trimmed.is_empty() {
        "monospace"
    } else {
        trimmed
    };
    format!(
        "\"{}\"",
        fallback.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

pub fn css_class_id(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            output.push(character.to_ascii_lowercase());
        } else {
            output.push('-');
        }
    }
    if output.is_empty() {
        "module".into()
    } else {
        output
    }
}

/// Base application CSS. User CSS is loaded into a separate provider so an
/// invalid override cannot remove the safe base theme.
pub fn build_css(config: &Config, hue: Option<f64>) -> String {
    let (default_bg, foreground, secondary) = match config.theme.as_str() {
        "light" => ("rgba(255,255,255,0.96)", "#262628", "#6F6F76"),
        _ => ("rgba(25,25,27,0.96)", "#F5F5F5", "#A1A1AA"),
    };
    let background = if let Some(hue) = hue {
        format!("hsla({hue:.0}, 38%, 16%, {:.2})", config.bar_opacity)
    } else if config.custom_bg.trim().is_empty() {
        default_bg.into()
    } else {
        safe_color_or(&config.custom_bg, default_bg)
    };
    let font = css_font(&config.font_family);
    let font_size = config.font_size.clamp(8, 24);
    let accent = safe_color_or(&config.accent_color, "#7DD3FC");
    let warning = safe_color_or(&config.warning_color, "#FBBF24");
    let critical = safe_color_or(&config.critical_color, "#FB7185");
    let surface = safe_color_or(&config.surface_color, "rgba(255,255,255,0.06)");
    let gap = config.module_gap;
    let padding = config.module_padding;
    let compact_padding = (padding - 2).max(1);
    let tiny_padding = (padding - 4).max(0);
    let radius = config.module_radius;
    let font_weight = config.font_weight.clamp(100, 900);
    let border_alpha = config.border_strength.clamp(0.0, 0.6);
    let border = format!("rgba(255,255,255,{border_alpha:.2})");
    let transition = if config.reduce_motion {
        "none"
    } else {
        "160ms ease"
    };

    let mut css = format!(
        ".topbar {{ background-color: {background}; }}\n\
         .topbar.dimmed {{ opacity: 0.20; }}\n\
         .topbar .zone {{ border-spacing: {gap}px; }}\n\
         .topbar .module {{ background-color: {surface}; border: 1px solid {border}; border-radius: {radius}px; padding: 0 {padding}px; transition: {transition}; }}\n\
         .topbar .module.layout-compact {{ padding-left: {compact_padding}px; padding-right: {compact_padding}px; }}\n\
         .topbar .module.layout-tiny {{ padding-left: {tiny_padding}px; padding-right: {tiny_padding}px; }}\n\
         .topbar .overflow-button {{ min-width: 48px; font-weight: 700; }}\n\
         .overflow-row {{ min-width: 280px; padding: 4px 2px; border-spacing: 12px; }}\n\
         .overflow-empty {{ min-width: 280px; padding: 8px 2px; opacity: 0.72; }}\n\
         .topbar label {{ color: {foreground}; font-family: {font}; font-size: {font_size}px; font-weight: {font_weight}; font-variant-numeric: tabular-nums; }}\n\
         .topbar .secondary {{ color: {secondary}; }}\n\
         .topbar .accent, .topbar .accent label {{ color: {accent}; }}\n\
         .topbar .warn, .topbar .warn label {{ color: {warning}; }}\n\
         .topbar .crit, .topbar .crit label {{ color: {critical}; }}\n\
         .topbar .crit.blink-off, .topbar .crit.blink-off label {{ opacity: 0.40; }}\n\
         .topbar .dim {{ opacity: 0.48; }}\n\
         .topbar button {{ min-height: 20px; min-width: 20px; padding: 0 5px; color: {foreground}; background: transparent; border: none; border-radius: {radius}px; }}\n\
         .topbar button:hover {{ background-color: rgba(255,255,255,0.10); }}\n\
         .topbar button.action-pending {{ color: {warning}; background-color: rgba(251,191,36,0.12); }}\n\
         .topbar button.action-ok {{ color: {accent}; background-color: rgba(125,211,252,0.14); }}\n\
         .topbar button.action-error {{ color: {critical}; background-color: rgba(251,113,133,0.16); }}\n\
         .topbar scale {{ min-height: 16px; padding: 0 2px; }}\n\
         .topbar scale trough {{ min-height: 4px; border-radius: 4px; background-color: rgba(255,255,255,0.18); }}\n\
         .topbar scale highlight {{ min-height: 4px; border-radius: 4px; background-color: {accent}; }}\n\
         .topbar scale slider {{ min-width: 10px; min-height: 10px; border-radius: 50%; background-color: {foreground}; margin: -4px 0; }}\n\
         .settings-shell {{ background-color: @theme_bg_color; }}\n\
         .settings-search-header {{ padding: 10px 14px; border-bottom: 1px solid alpha(@theme_fg_color, 0.12); }}\n\
         .compact-settings-nav {{ min-width: 150px; }}\n\
         .settings-sidebar {{ min-width: 176px; border-right: 1px solid alpha(@theme_fg_color, 0.12); }}\n\
         .settings-page {{ padding: 20px; border-spacing: 12px; }}\n\
         .settings-section {{ padding: 14px; border-radius: 10px; background-color: alpha(@theme_fg_color, 0.045); border: 1px solid alpha(@theme_fg_color, 0.10); }}\n\
         .settings-title {{ font-size: 18px; font-weight: 700; }}\n\
         .settings-caption {{ opacity: 0.68; }}\n\
         .module-catalog-filters {{ border-spacing: 8px; }}\n\
         .module-catalog-column {{ padding: 0 10px 10px 0; }}\n\
         .module-catalog-detail {{ padding: 0 0 10px 14px; }}\n\
         .module-catalog-list {{ background: transparent; }}\n\
         .module-catalog-row {{ padding: 9px 10px; border-radius: 8px; border-bottom: 1px solid alpha(@theme_fg_color, 0.08); }}\n\
         .module-catalog-row:selected {{ background-color: alpha({accent}, 0.18); }}\n\
         .module-catalog-title {{ font-weight: 700; }}\n\
         .module-catalog-state {{ padding: 2px 7px; border-radius: 999px; font-size: 11px; }}\n\
         .module-catalog-state.enabled {{ color: {accent}; background-color: alpha({accent}, 0.12); }}\n\
         .settings-page button {{ min-height: 34px; }}\n\
         .module-layout-row {{ padding: 4px 2px; border-bottom: 1px solid alpha(@theme_fg_color, 0.08); }}\n\
         .module-layout-row button {{ min-width: 44px; min-height: 44px; }}\n\
         .module-layout-row .tier-normal {{ color: {accent}; }}\n\
         .module-layout-row .tier-tight {{ color: {warning}; }}\n\
         .module-layout-row .tier-overflow {{ color: {critical}; }}\n\
         .action-toast {{ margin-top: 6px; padding: 5px 12px; border-radius: 999px; font-weight: 700; box-shadow: 0 1px 4px rgba(0,0,0,0.35); }}\n\
         .action-toast-pending {{ background-color: rgba(251,191,36,0.22); color: {warning}; }}\n\
         .action-toast-ok {{ background-color: rgba(125,211,252,0.24); color: {accent}; }}\n\
         .action-toast-error {{ background-color: rgba(251,113,133,0.26); color: {critical}; }}\n\
         .settings-page button:focus-visible, .module-layout-row button:focus-visible, .settings-page drop-down:focus-visible {{ outline: 2px solid {accent}; outline-offset: 1px; }}\n\
         .module-catalog-state.disabled {{ opacity: 0.58; }}\n\
         .module-catalog-empty, .module-detail-empty {{ opacity: 0.68; }}\n\
         .module-editor {{ margin-bottom: 12px; }}\n\
         .settings-footer {{ padding: 10px 14px; border-top: 1px solid alpha(@theme_fg_color, 0.12); }}\n"
    );

    for metric in &config.metrics {
        append_module_css(&mut css, &metric.id, &metric.fg_color, &metric.bg_color);
    }
    for module in &config.custom_modules {
        append_module_css(
            &mut css,
            &format!("custom-{}", module.name),
            &module.fg_color,
            &module.bg_color,
        );
    }
    css
}

fn append_module_css(css: &mut String, id: &str, foreground: &str, background: &str) {
    let class = css_class_id(id);
    let foreground = safe_optional_color(foreground);
    let background = safe_optional_color(background);
    if !foreground.is_empty() {
        css.push_str(&format!(
            ".topbar .module-{class}, .topbar .module-{class} label {{ color: {foreground}; }}\n"
        ));
    }
    if !background.is_empty() {
        css.push_str(&format!(
            ".topbar .module-{class} {{ background-color: {background}; }}\n"
        ));
    }
}

pub fn build_custom_css(config: &Config) -> String {
    config.custom_css.trim().to_string()
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
        "wifi" => "WIFI",
        "pkg_updates" => "UPD",
        "bluetooth" => "BT",
        "vpn" => "VPN",
        _ => "",
    }
}

pub fn read_import(path: &Path) -> Result<Config, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    Config::from_import(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_migrates_and_gets_new_metrics() {
        let raw = r#"
height = 999
interval_ms = 1
position = "side"
metrics = [{ id = "cpu", label = "C", enabled = true }]
"#;
        let mut config: Config = toml::from_str(raw).expect("old config parses");
        config.migrate_and_validate();
        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
        assert_eq!(config.height, 64);
        assert_eq!(config.interval_ms, 250);
        assert_eq!(config.position, "top");
        assert!(config.metrics.iter().any(|metric| metric.id == "network"));
    }

    #[test]
    fn effect_fields_clamp_and_reject_unknown_kinds() {
        let raw = r#"
            effects_enabled = true
            effects_kind = "aurora-sweep"
            effects_fps_cap = 500
            effects_intensity = 42.0
            effects_trail = -1.0
            effects_thickness = 999.0
            effects_speed = 100.0
        "#;
        let mut config: Config = toml::from_str(raw).expect("flat effect fields parse");
        config.migrate_and_validate();
        assert_eq!(config.effects_fps_cap, 60);
        assert_eq!(config.effects_intensity, 2.0);
        assert_eq!(config.effects_trail, 0.0);
        assert_eq!(config.effects_thickness, 12.0);
        assert_eq!(config.effects_speed, 4.0);
        assert_eq!(config.effects_kind, "aurora-sweep");

        let mut bogus: Config = toml::from_str("effects_kind = \"rainbow\"").expect("parses");
        bogus.migrate_and_validate();
        assert_eq!(bogus.effects_kind, "off");
        assert!(!bogus.effects_enabled);
    }

    #[test]
    fn theme_tokens_clamp() {
        let mut config = Config {
            font_weight: 5_000,
            border_strength: 12.0,
            ..Config::default()
        };
        config.migrate_and_validate();
        assert_eq!(config.font_weight, 900);
        assert_eq!(config.border_strength, 0.6);
        let mut config = Config {
            font_weight: 0,
            border_strength: -1.0,
            ..Config::default()
        };
        config.migrate_and_validate();
        assert_eq!(config.font_weight, 100);
        assert_eq!(config.border_strength, 0.0);
    }

    #[test]
    fn imported_commands_are_disabled_and_untrusted() {
        let raw = r#"
[[custom_modules]]
name = "weather"
command = "curl example.invalid"
enabled = true
"#;
        let config = Config::from_import(raw).expect("import parses");
        assert!(!config.custom_modules[0].enabled);
        assert!(!config.custom_modules[0].trusted);
    }

    #[test]
    fn duplicate_custom_module_names_are_made_unique() {
        let mut config = Config {
            custom_modules: vec![
                CustomModule {
                    name: "weather".into(),
                    ..CustomModule::default()
                },
                CustomModule {
                    name: "weather".into(),
                    ..CustomModule::default()
                },
            ],
            ..Config::default()
        };
        config.migrate_and_validate();
        assert_eq!(config.custom_modules[0].name, "weather");
        assert_eq!(config.custom_modules[1].name, "weather 2");
    }

    #[test]
    fn unsafe_css_color_is_rejected() {
        let mut config = Config {
            accent_color: "red; } window { opacity: 0".into(),
            ..Config::default()
        };
        config.migrate_and_validate();
        assert_eq!(config.accent_color, default_accent());
    }

    #[test]
    fn css_class_ids_are_stable() {
        assert_eq!(css_class_id("CPU Package #1"), "cpu-package--1");
    }
}

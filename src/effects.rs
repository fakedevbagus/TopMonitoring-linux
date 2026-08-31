//! Visual effect engine for the TopMonitoring bar.
//!
//! Architecture rules (MAJOR_UPGRADE_V4_PLAN.md Step 0.2):
//! - Effects are drawn by a frame-clock-driven `DrawingArea` layered in a
//!   `GtkOverlay` above the bar content. CSS providers are never rebuilt per
//!   frame; only this layer is redrawn.
//! - The layer is non-interactive (`set_can_target(false)`), so pointer
//!   events keep reaching the modules underneath.
//! - Pure state (kind parsing, parameter clamping, fps-capped phase
//!   advancement, power policy) lives in types without GTK dependencies and
//!   is covered by unit tests; the GTK glue is a thin executor.
//! - The tick loop pauses automatically when the window is hidden/unmapped
//!   because GTK only drives tick callbacks for mapped widgets.

use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Default frame cap; keeps the engine far below the <3 ms p95 frame budget.
pub const DEFAULT_FPS_CAP: u32 = 30;
/// One full animation loop in phase milliseconds before wrapping.
const PHASE_PERIOD_MS: f64 = 60_000.0;
/// Largest elapsed time a single tick may absorb (long hides must not jump).
const MAX_FRAME_DELTA_MS: f64 = 250.0;
/// How often the power policy is re-evaluated from the frame clock timeline.
const BATTERY_RECHECK_MS: f64 = 5_000.0;

/// The available visual effects. Unknown config strings fall back to `Off`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EffectKind {
    #[default]
    Off,
    OrbitGlow,
    EmberOrbit,
    AuroraSweep,
    CriticalBeacon,
    StaticFallback,
}

impl EffectKind {
    /// Canonical config token for this kind. The Theme Studio UI (Step 0.4)
    /// round-trips effect selection through this string.
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            EffectKind::Off => "off",
            EffectKind::OrbitGlow => "orbit-glow",
            EffectKind::EmberOrbit => "ember-orbit",
            EffectKind::AuroraSweep => "aurora-sweep",
            EffectKind::CriticalBeacon => "critical-beacon",
            EffectKind::StaticFallback => "static-fallback",
        }
    }

    /// Unknown or empty values intentionally resolve to `Off`, which keeps
    /// malformed configs safe instead of enabling an animation.
    pub fn parse(value: &str) -> EffectKind {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => EffectKind::Off,
            "orbit-glow" | "orbit_glow" => EffectKind::OrbitGlow,
            "ember-orbit" | "ember_orbit" => EffectKind::EmberOrbit,
            "aurora-sweep" | "aurora_sweep" => EffectKind::AuroraSweep,
            "critical-beacon" | "critical_beacon" => EffectKind::CriticalBeacon,
            "static-fallback" | "static_fallback" => EffectKind::StaticFallback,
            _ => EffectKind::Off,
        }
    }
}

/// Bounded effect parameters. Every numeric value is clamped so a hand-edited
/// config cannot push the engine outside its frame budget.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectParams {
    pub enabled: bool,
    pub kind: EffectKind,
    pub fps_cap: u32,
    pub intensity: f64,
    pub trail: f64,
    pub thickness: f64,
    pub speed: f64,
    /// Accent color as linear RGB 0..1 (parsed from the config accent token).
    pub accent: [f64; 3],
    /// Critical color as linear RGB 0..1 (used by the beacon effect).
    pub critical: [f64; 3],
}

impl Default for EffectParams {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: EffectKind::Off,
            fps_cap: DEFAULT_FPS_CAP,
            intensity: 0.6,
            trail: 0.5,
            thickness: 3.0,
            speed: 1.0,
            accent: parse_rgb("#4C8DFF"),
            critical: parse_rgb("#FF4C4C"),
        }
    }
}

impl EffectParams {
    /// Build clamped parameters from the flat config fields. Missing or
    /// out-of-range values are always reduced to a safe default.
    pub fn from_config(config: &crate::config::Config) -> EffectParams {
        let mut params = EffectParams {
            enabled: config.effects_enabled,
            kind: EffectKind::parse(&config.effects_kind),
            fps_cap: config.effects_fps_cap,
            intensity: config.effects_intensity,
            trail: config.effects_trail,
            thickness: config.effects_thickness,
            speed: config.effects_speed,
            accent: parse_rgb(&config.accent_color),
            critical: parse_rgb(&config.critical_color),
        };
        params.clamp();
        params
    }

    pub fn clamp(&mut self) {
        self.fps_cap = self.fps_cap.clamp(1, 60);
        self.intensity = self.intensity.clamp(0.0, 2.0);
        self.trail = self.trail.clamp(0.0, 1.0);
        self.thickness = self.thickness.clamp(1.0, 12.0);
        self.speed = self.speed.clamp(0.1, 4.0);
    }

    /// Effective kind after all policies:
    /// - disabled or `Off` stays off;
    /// - `reduce_motion` forces the static fallback (no animation);
    /// - running on battery auto-disables continuous animation.
    pub fn effective_kind(&self, reduce_motion: bool, on_battery: bool) -> EffectKind {
        if !self.enabled || self.kind == EffectKind::Off {
            return EffectKind::Off;
        }
        if on_battery {
            return EffectKind::Off;
        }
        if reduce_motion {
            return EffectKind::StaticFallback;
        }
        self.kind
    }
}

/// Pure, GTK-free animation state. `phase` is the driver for every effect.
#[derive(Clone, Copy, Debug)]
pub struct EffectState {
    phase_ms: f64,
    last_frame_ms: Option<f64>,
}

impl Default for EffectState {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectState {
    pub fn new() -> Self {
        Self {
            phase_ms: 0.0,
            last_frame_ms: None,
        }
    }

    /// Advance the phase using the frame clock timestamp (milliseconds).
    /// Returns `Some` only when the fps cap allows a new frame; `None` means
    /// the caller must skip the redraw entirely.
    pub fn advance(&mut self, frame_ms: f64, fps_cap: u32, speed: f64) -> Option<f64> {
        let fps = fps_cap.max(1) as f64;
        let min_interval_ms = 1_000.0 / fps;
        let elapsed = match self.last_frame_ms {
            Some(last) => (frame_ms - last).clamp(0.0, MAX_FRAME_DELTA_MS),
            None => min_interval_ms,
        };
        if elapsed + f64::EPSILON < min_interval_ms {
            return None;
        }
        self.last_frame_ms = Some(frame_ms);
        self.phase_ms = (self.phase_ms + elapsed * speed) % PHASE_PERIOD_MS;
        Some(self.phase_ms)
    }

    pub fn reset(&mut self) {
        *self = EffectState::new();
    }
}

/// Reads the power-supply sysfs tree. `true` means at least one battery is
/// discharging; absence of the tree (desktops/VMs) is treated as AC power.
pub fn is_on_battery() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return false;
    };
    for entry in entries.flatten() {
        let directory = entry.path();
        let kind = std::fs::read_to_string(directory.join("type")).unwrap_or_default();
        if kind.trim().eq_ignore_ascii_case("battery") {
            let status = std::fs::read_to_string(directory.join("status")).unwrap_or_default();
            if status.trim().eq_ignore_ascii_case("discharging") {
                return true;
            }
        }
    }
    false
}

/// Shared engine state handed to both the tick callback and the draw func.
struct EffectShared {
    params: RefCell<EffectParams>,
    reduce_motion: Cell<bool>,
    on_battery: Cell<bool>,
    battery_checked_at: Cell<f64>,
    state: RefCell<EffectState>,
    plan: Cell<Option<f64>>,
    active_kind: Cell<EffectKind>,
}

impl EffectShared {
    /// One frame-clock tick. Returns `true` when the layer must be redrawn.
    fn tick(&self, now_ms: f64) -> bool {
        // The power policy is re-evaluated rarely; sysfs I/O must not run per
        // frame. Absent hardware keeps the cached value.
        if now_ms - self.battery_checked_at.get() >= BATTERY_RECHECK_MS {
            self.battery_checked_at.set(now_ms);
            self.on_battery.set(is_on_battery());
        }
        let params = self.params.borrow();
        let kind = params.effective_kind(self.reduce_motion.get(), self.on_battery.get());
        self.active_kind.set(kind);
        if kind == EffectKind::Off {
            self.plan.set(None);
            return false;
        }
        match self
            .state
            .borrow_mut()
            .advance(now_ms, params.fps_cap, params.speed)
        {
            Some(phase) => {
                self.plan.set(Some(phase));
                true
            }
            None => false,
        }
    }
}

/// GTK executor for the effect layer. Owns no widget, so the closures held by
/// the `DrawingArea` cannot form a reference cycle.
#[derive(Clone)]
pub struct EffectEngine {
    shared: Rc<EffectShared>,
}

impl EffectEngine {
    /// Create the non-interactive drawing layer and attach it to `overlay`.
    /// The returned engine keeps parameters in sync with the live config.
    pub fn attach(
        overlay: &gtk::Overlay,
        params: EffectParams,
        reduce_motion: bool,
    ) -> EffectEngine {
        let area = gtk::DrawingArea::new();
        area.set_can_target(false);
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_halign(gtk::Align::Fill);
        area.set_valign(gtk::Align::Fill);
        area.set_css_classes(&["effect-layer"]);

        let shared = Rc::new(EffectShared {
            params: RefCell::new(params),
            reduce_motion: Cell::new(reduce_motion),
            on_battery: Cell::new(false),
            battery_checked_at: Cell::new(f64::NEG_INFINITY),
            state: RefCell::new(EffectState::new()),
            plan: Cell::new(None),
            active_kind: Cell::new(EffectKind::Off),
        });

        let tick_shared = shared.clone();
        area.add_tick_callback(move |area, frame_clock| {
            let now_ms = frame_clock.frame_time() as f64 / 1_000.0;
            if tick_shared.tick(now_ms) {
                area.queue_draw();
            }
            glib::ControlFlow::Continue
        });

        let draw_shared = shared.clone();
        area.set_draw_func(move |_area, context, width, height| {
            draw_frame(&draw_shared, context, width as f64, height as f64);
        });

        overlay.add_overlay(&area);
        EffectEngine { shared }
    }

    /// Re-read policy inputs from the config. Called from the main polling
    /// loop, so config changes take effect without rebuilding the bar.
    pub fn update_params(&self, params: EffectParams, reduce_motion: bool) {
        let mut current = self.shared.params.borrow_mut();
        if current.kind != params.kind {
            self.shared.state.borrow_mut().reset();
        }
        *current = params;
        drop(current);
        self.shared.reduce_motion.set(reduce_motion);
    }
}

/// Paint the current frame. A missing plan (paused or off) leaves the layer
/// fully transparent.
fn draw_frame(shared: &EffectShared, context: &gtk::cairo::Context, width: f64, height: f64) {
    let Some(phase) = shared.plan.get() else {
        return;
    };
    let kind = shared.active_kind.get();
    if kind == EffectKind::Off {
        return;
    }
    let params = shared.params.borrow();
    let _ = context.save();
    let result = match kind {
        EffectKind::Off => Ok(()),
        EffectKind::StaticFallback => draw_static_fallback(&params, context, width, height),
        EffectKind::OrbitGlow => draw_orbit_glow(&params, context, width, height, phase),
        EffectKind::EmberOrbit => draw_ember_orbit(&params, context, width, height, phase),
        EffectKind::AuroraSweep => draw_aurora_sweep(&params, context, width, height, phase),
        EffectKind::CriticalBeacon => draw_critical_beacon(&params, context, width, height, phase),
    };
    let _ = context.restore();
    // Cairo errors are not fatal for the bar; the frame is simply dropped.
    let _ = result;
}

fn intensity_alpha(base: f64, intensity: f64) -> f64 {
    (base * intensity).clamp(0.0, 0.85)
}

fn rounded_rect_path(
    context: &gtk::cairo::Context,
    width: f64,
    height: f64,
    radius: f64,
) -> Result<(), gtk::cairo::Error> {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    context.new_sub_path();
    context.arc(
        width - radius,
        radius,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    context.arc(
        width - radius,
        height - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    context.arc(
        radius,
        height - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    context.arc(
        radius,
        radius,
        radius,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    context.close_path();
    Ok(())
}

fn set_rgb(context: &gtk::cairo::Context, rgb: &[f64; 3], alpha: f64) {
    context.set_source_rgba(rgb[0], rgb[1], rgb[2], alpha);
}

/// Reduce-motion / battery fallback: a calm, static accent border.
fn draw_static_fallback(
    params: &EffectParams,
    context: &gtk::cairo::Context,
    width: f64,
    height: f64,
) -> Result<(), gtk::cairo::Error> {
    let inset = params.thickness / 2.0;
    rounded_rect_path(context, width - inset, height - inset, 8.0)?;
    context.translate(inset, inset);
    set_rgb(
        context,
        &params.accent,
        intensity_alpha(0.28, params.intensity),
    );
    context.set_line_width(params.thickness);
    let _ = context.stroke();
    Ok(())
}

/// A soft glow blob orbiting the bar.
fn draw_orbit_glow(
    params: &EffectParams,
    context: &gtk::cairo::Context,
    width: f64,
    height: f64,
    phase: f64,
) -> Result<(), gtk::cairo::Error> {
    let angle = phase / 12_000.0 * std::f64::consts::TAU;
    let margin = height;
    let cx = margin + (width - 2.0 * margin) * (0.5 + 0.5 * angle.sin());
    let cy = height * (0.5 + 0.35 * angle.cos());
    let radius = (height * (0.9 + 0.4 * params.intensity)).max(8.0);
    let glow = gtk::cairo::RadialGradient::new(cx, cy, radius * 0.1, cx, cy, radius);
    let alpha = intensity_alpha(0.35, params.intensity);
    glow.add_color_stop_rgba(
        0.0,
        params.accent[0],
        params.accent[1],
        params.accent[2],
        alpha,
    );
    glow.add_color_stop_rgba(
        1.0,
        params.accent[0],
        params.accent[1],
        params.accent[2],
        0.0,
    );
    context.set_source(&glow)?;
    context.rectangle(0.0, 0.0, width, height);
    let _ = context.fill();
    Ok(())
}

/// Small particles orbiting the bar.
fn draw_ember_orbit(
    params: &EffectParams,
    context: &gtk::cairo::Context,
    width: f64,
    height: f64,
    phase: f64,
) -> Result<(), gtk::cairo::Error> {
    let count = 3 + (params.trail * 6.0).round() as i32;
    for index in 0..count {
        let offset = index as f64 / count as f64;
        let angle = phase / 9_000.0 * std::f64::consts::TAU + offset * std::f64::consts::TAU;
        let cx = width * (0.5 + 0.5 * angle.cos());
        let cy = height * (0.5 + 0.5 * angle.sin());
        let alpha = intensity_alpha(0.45 * (1.0 - offset * params.trail * 0.7), params.intensity);
        set_rgb(context, &params.accent, alpha);
        context.arc(
            cx,
            cy,
            (params.thickness * 0.9).max(1.5),
            0.0,
            std::f64::consts::TAU,
        );
        let _ = context.fill();
    }
    Ok(())
}

/// A translucent aurora band sweeping across the bar.
fn draw_aurora_sweep(
    params: &EffectParams,
    context: &gtk::cairo::Context,
    width: f64,
    height: f64,
    phase: f64,
) -> Result<(), gtk::cairo::Error> {
    let band = width * 0.35;
    let progress = (phase % 10_000.0) / 10_000.0;
    let x0 = -band + progress * (width + 2.0 * band);
    let gradient = gtk::cairo::LinearGradient::new(x0, 0.0, x0 + band, 0.0);
    let alpha = intensity_alpha(0.30, params.intensity);
    gradient.add_color_stop_rgba(
        0.0,
        params.accent[0],
        params.accent[1],
        params.accent[2],
        0.0,
    );
    gradient.add_color_stop_rgba(
        0.5,
        params.accent[0],
        params.accent[1],
        params.accent[2],
        alpha,
    );
    gradient.add_color_stop_rgba(
        1.0,
        params.accent[0],
        params.accent[1],
        params.accent[2],
        0.0,
    );
    context.set_source(&gradient)?;
    context.rectangle(0.0, 0.0, width, height);
    let _ = context.fill();
    Ok(())
}

/// A pulsing critical border.
fn draw_critical_beacon(
    params: &EffectParams,
    context: &gtk::cairo::Context,
    width: f64,
    height: f64,
    phase: f64,
) -> Result<(), gtk::cairo::Error> {
    let pulse = 0.5 + 0.5 * (phase / 1_400.0 * std::f64::consts::TAU).sin();
    let inset = params.thickness / 2.0;
    rounded_rect_path(context, width - inset, height - inset, 8.0)?;
    context.translate(inset, inset);
    set_rgb(
        context,
        &params.critical,
        intensity_alpha(0.2 + 0.4 * pulse, params.intensity),
    );
    context.set_line_width(params.thickness * (0.8 + 0.4 * pulse));
    let _ = context.stroke();
    Ok(())
}

/// Parse `#RGB`, `#RRGGBB`, and `rgb()/rgba()` tokens into linear RGB.
/// Unparseable values fall back to a safe blue.
fn parse_rgb(value: &str) -> [f64; 3] {
    let trimmed = value.trim();
    if let Some(hex) = trimmed.strip_prefix('#') {
        let digits: String = hex
            .chars()
            .filter(|character| character.is_ascii_hexdigit())
            .collect();
        if digits.len() == 3 {
            let channels: Vec<f64> = digits
                .chars()
                .filter_map(|character| character.to_digit(16))
                .map(|digit| (digit * 17) as f64 / 255.0)
                .collect();
            if channels.len() == 3 {
                return [channels[0], channels[1], channels[2]];
            }
        } else if digits.len() >= 6 {
            let bytes: Vec<f64> = (0..3)
                .filter_map(|index| u8::from_str_radix(&digits[index * 2..index * 2 + 2], 16).ok())
                .map(|byte| byte as f64 / 255.0)
                .collect();
            if bytes.len() == 3 {
                return [bytes[0], bytes[1], bytes[2]];
            }
        }
    }
    if trimmed.starts_with("rgba(") || trimmed.starts_with("rgb(") {
        let inner = trimmed
            .trim_start_matches("rgba(")
            .trim_start_matches("rgb(")
            .trim_end_matches(')');
        let parts: Vec<f64> = inner
            .split(',')
            .filter_map(|part| part.trim().parse::<f64>().ok())
            .collect();
        if parts.len() >= 3 {
            return [
                (parts[0] / 255.0).clamp(0.0, 1.0),
                (parts[1] / 255.0).clamp(0.0, 1.0),
                (parts[2] / 255.0).clamp(0.0, 1.0),
            ];
        }
    }
    [0.30, 0.55, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_kind_falls_back_to_off() {
        assert_eq!(EffectKind::parse("aurora-sweep"), EffectKind::AuroraSweep);
        assert_eq!(EffectKind::parse(" EMBER_ORBIT "), EffectKind::EmberOrbit);
        assert_eq!(EffectKind::parse("nonsense"), EffectKind::Off);
        assert_eq!(EffectKind::parse(""), EffectKind::Off);
    }

    #[test]
    fn params_from_config_are_clamped() {
        let mut config = crate::config::Config {
            effects_enabled: true,
            effects_kind: "aurora-sweep".into(),
            effects_fps_cap: 500,
            effects_intensity: 9.0,
            effects_trail: -2.0,
            effects_thickness: 99.0,
            effects_speed: 0.0,
            ..crate::config::Config::default()
        };
        config.migrate_and_validate();
        let params = EffectParams::from_config(&config);
        assert_eq!(params.fps_cap, 60);
        assert_eq!(params.intensity, 2.0);
        assert_eq!(params.trail, 0.0);
        assert_eq!(params.thickness, 12.0);
        assert_eq!(params.speed, 0.1);
    }

    #[test]
    fn policy_forces_static_or_off() {
        let mut params = EffectParams {
            enabled: true,
            kind: EffectKind::OrbitGlow,
            ..EffectParams::default()
        };
        assert_eq!(params.effective_kind(false, false), EffectKind::OrbitGlow);
        assert_eq!(
            params.effective_kind(true, false),
            EffectKind::StaticFallback
        );
        assert_eq!(params.effective_kind(false, true), EffectKind::Off);
        params.kind = EffectKind::Off;
        assert_eq!(params.effective_kind(false, false), EffectKind::Off);
        params.enabled = false;
        params.kind = EffectKind::AuroraSweep;
        assert_eq!(params.effective_kind(false, false), EffectKind::Off);
    }

    #[test]
    fn advance_respects_fps_cap() {
        let mut state = EffectState::new();
        assert!(state.advance(0.0, 30, 1.0).is_some());
        assert!(state.advance(10.0, 30, 1.0).is_none());
        assert!(state.advance(40.0, 30, 1.0).is_some());
    }

    #[test]
    fn advance_scales_phase_with_speed() {
        let mut state = EffectState::new();
        let first = state.advance(0.0, 60, 2.0).expect("frame due");
        let phase = state.advance(200.0, 60, 2.0).expect("frame due");
        assert!((phase - first - 400.0).abs() < 1.0);
    }

    #[test]
    fn advance_ignores_giant_time_gaps() {
        let mut state = EffectState::new();
        let first = state.advance(0.0, 30, 1.0).expect("frame due");
        let phase = state
            .advance(3_600_000.0, 30, 1.0)
            .expect("frame due after gap");
        assert!(phase - first <= MAX_FRAME_DELTA_MS + 1.0);
    }

    #[test]
    fn rgb_parser_handles_tokens_and_fallbacks() {
        assert_eq!(parse_rgb("#FF8000"), [1.0, 128.0 / 255.0, 0.0]);
        assert_eq!(parse_rgb("#F80"), [1.0, 136.0 / 255.0, 0.0]);
        assert_eq!(parse_rgb("rgba(255, 0, 0, 0.5)"), [1.0, 0.0, 0.0]);
        assert_eq!(parse_rgb("junk"), [0.30, 0.55, 1.0]);
    }
}

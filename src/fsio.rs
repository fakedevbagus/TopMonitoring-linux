//! Pure filesystem/hwmon helpers shared between the backend collector and
//! the bar UI. Extracted from `main.rs` during v4 Fase A (Step A.1) to break
//! the `backend -> main` dependency and to shrink `main.rs` (< 1200 LOC).
//!
//! Every function here is GTK-free and panic-free: absent sysfs nodes, format
//! errors, and permission failures all return a fallback (`None`, empty vec,
//! or "n/a") instead of unwrapping.

use std::cell::RefCell;
use std::path::Path;

/// Aggregated disk information for one mount point.
#[derive(Clone, Debug, PartialEq)]
pub struct DiskSummary {
    pub name: String,
    pub mount: String,
    pub file_system: String,
    pub used_percent: f64,
    pub available_gb: f64,
    pub total_gb: f64,
}

fn compact_disk_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    let mut compact: String = name.chars().take(max_chars.saturating_sub(1)).collect();
    compact.push('…');
    compact
}

fn disk_bar_entries(entries: &[DiskSummary]) -> Vec<&DiskSummary> {
    let root = entries.iter().find(|entry| entry.mount == "/");
    let worst_other = entries
        .iter()
        .filter(|entry| entry.mount != "/")
        .max_by(|left, right| {
            left.used_percent
                .partial_cmp(&right.used_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let mut selected = Vec::with_capacity(2);
    if let Some(root) = root {
        selected.push(root);
    }
    if let Some(other) = worst_other {
        selected.push(other);
    }
    if selected.is_empty() {
        if let Some(worst) = entries.iter().max_by(|left, right| {
            left.used_percent
                .partial_cmp(&right.used_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            selected.push(worst);
        }
    }
    selected
}

/// Root + fullest volume, `name XX% · name YY%`.
pub fn format_disk_bar(entries: &[DiskSummary]) -> String {
    disk_bar_entries(entries)
        .iter()
        .map(|entry| {
            format!(
                "{} {:.0}%",
                compact_disk_name(&entry.name, 8),
                entry.used_percent
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Single fullest volume at compact width.
pub fn format_disk_compact(entries: &[DiskSummary]) -> String {
    disk_bar_entries(entries)
        .into_iter()
        .max_by(|left, right| {
            left.used_percent
                .partial_cmp(&right.used_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|entry| {
            format!(
                "{} {:.0}%",
                compact_disk_name(&entry.name, 12),
                entry.used_percent
            )
        })
        .unwrap_or_else(|| "n/a".into())
}

/// Bounded percentage badge for tiny width.
pub fn format_disk_tiny(entries: &[DiskSummary]) -> String {
    entries
        .iter()
        .map(|entry| entry.used_percent)
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
        .map(|used| format!("{used:.0}%"))
        .unwrap_or_else(|| "n/a".into())
}

/// Largest fan RPM across all `hwmon` fan inputs; `None` when no fan is found.
pub fn read_fan_rpm() -> Option<u32> {
    let mut max = 0u32;
    for entry in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let base = entry.path();
        for i in 1..=8 {
            if let Ok(s) = std::fs::read_to_string(base.join(format!("fan{i}_input"))) {
                if let Ok(v) = s.trim().parse::<u32>() {
                    if v > max {
                        max = v;
                    }
                }
            }
        }
    }
    (max > 0).then_some(max)
}

/// First battery capacity (%) and status string.
pub fn read_battery() -> Option<(u8, String)> {
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let p = entry.path();
        if let Ok(t) = std::fs::read_to_string(p.join("type")) {
            if t.trim() == "Battery" {
                if let Ok(c) = std::fs::read_to_string(p.join("capacity")) {
                    if let Ok(cap) = c.trim().parse::<u8>() {
                        let status = std::fs::read_to_string(p.join("status"))
                            .map(|s| s.trim().to_string())
                            .unwrap_or_default();
                        return Some((cap, status));
                    }
                }
            }
        }
    }
    None
}

/// AMD GPU busy percentage and junction temperature (millidegree -> degree).
pub fn read_amd_gpu(index: u32) -> Option<(u32, Option<u32>)> {
    let dev = amd_card_device(index)?;
    let busy = std::fs::read_to_string(dev.join("gpu_busy_percent"))
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()?;
    Some((
        busy,
        amd_hwmon_read(&dev, "temp1_input").map(|v| (v / 1000.0) as u32),
    ))
}

/// Returns the Nth (0-based) AMD GPU device directory, for multi-GPU setups.
pub fn amd_card_device(index: u32) -> Option<std::path::PathBuf> {
    let mut seen = 0u32;
    for entry in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("card") && !name.contains('-') {
            let dev = entry.path().join("device");
            if dev.join("gpu_busy_percent").exists() {
                if seen == index {
                    return Some(dev);
                }
                seen += 1;
            }
        }
    }
    None
}

/// Read a hwmon sensor file (millidegrees/millivolts) under the GPU device.
pub fn amd_hwmon_read(dev: &Path, file: &str) -> Option<f64> {
    for e in std::fs::read_dir(dev.join("hwmon")).ok()?.flatten() {
        if let Ok(s) = std::fs::read_to_string(e.path().join(file)) {
            if let Ok(v) = s.trim().parse::<f64>() {
                return Some(v);
            }
        }
    }
    None
}

/// Parse the currently-active GPU clock from `pp_dpm_sclk` / `pp_dpm_mclk`.
pub fn amd_active_clock(dev: &Path, file: &str) -> Option<u32> {
    let s = std::fs::read_to_string(dev.join(file)).ok()?;
    for line in s.lines() {
        if line.trim_end().ends_with('*') {
            for tok in line.split_whitespace() {
                let t = tok.to_lowercase();
                if t.ends_with("mhz") {
                    let num: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(v) = num.parse::<u32>() {
                        return Some(v);
                    }
                }
            }
        }
    }
    None
}

/// Best-effort Intel iGPU clock from `gt_cur_freq_mhz`.
pub fn read_intel_gpu_clock(index: u32) -> Option<u32> {
    let mut seen = 0u32;
    for entry in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("card") && !name.contains('-')) {
            continue;
        }
        let dev_dir = entry.path();
        let device = dev_dir.join("device");
        let Ok(vendor) = std::fs::read_to_string(device.join("vendor")) else {
            continue;
        };
        if vendor.trim() != "0x8086" {
            continue;
        }
        if seen != index {
            seen += 1;
            continue;
        }
        for rel in [
            "gt/gt0/gt_act_freq_mhz",
            "gt_act_freq_mhz",
            "gt_cur_freq_mhz",
        ] {
            if let Ok(s) = std::fs::read_to_string(dev_dir.join(rel)) {
                if let Ok(v) = s.trim().parse::<u32>() {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Convert a byte count into a compact human string.
pub fn human_bytes(bytes: u64) -> String {
    let value = bytes as f64;
    if value >= 1e9 {
        format!("{:.1}GB", value / 1e9)
    } else if value >= 1e6 {
        format!("{:.0}MB", value / 1e6)
    } else {
        format!("{:.0}KB", value / 1e3)
    }
}

/// Format uptime seconds as `XhYYm`.
pub fn human_uptime(secs: u64) -> String {
    format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
}

thread_local! {
    static RAPL_PREV: RefCell<(u64, std::time::Instant)> = RefCell::new((0, std::time::Instant::now()));
}

/// Read the current CPU power from RAPL sysfs. Returns watts, or `None`.
pub fn read_cpu_power() -> Option<f64> {
    let energy_uj = std::fs::read_to_string("/sys/class/powercap/intel-rapl:0/energy_uj")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    RAPL_PREV.with(|prev| {
        let mut prev = prev.borrow_mut();
        let dt = prev.1.elapsed().as_secs_f64().max(0.001);
        let dj = energy_uj.saturating_sub(prev.0) as f64 / 1e6;
        let first = prev.0 == 0;
        *prev = (energy_uj, std::time::Instant::now());
        if first {
            None
        } else {
            Some(dj / dt)
        }
    })
}

/// Disk temperatures from `nvme` and `drivetemp` hwmon sensors.
pub fn disk_temps() -> Vec<(String, f64)> {
    let mut results = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/hwmon") else {
        return results;
    };
    for entry in dir.flatten() {
        let base = entry.path();
        let Ok(chip) = std::fs::read_to_string(base.join("name")) else {
            continue;
        };
        let chip = chip.trim().to_lowercase();
        if !(chip.contains("nvme") || chip.contains("drivetemp")) {
            continue;
        }
        for index in 1..=4 {
            if let Ok(raw) = std::fs::read_to_string(base.join(format!("temp{index}_input"))) {
                if let Ok(milli) = raw.trim().parse::<f64>() {
                    let label = std::fs::read_to_string(base.join(format!("temp{index}_label")))
                        .map(|text| text.trim().to_string())
                        .unwrap_or_else(|_| chip.clone());
                    results.push((label, milli / 1000.0));
                }
            }
        }
    }
    results
}

// ---- Network rate formatting ------------------------------------------------

/// Format bytes as a human-readable rate (e.g., "1.5 MB/s").
pub fn human_rate(bytes: u64) -> String {
    format!("{}/s", human_bytes(bytes))
}

// ---- Disk I/O breakdown -----------------------------------------------------

/// Read per-disk I/O statistics from `/proc/diskstats`.
pub fn read_disk_io_breakdown() -> Vec<(String, u64, u64)> {
    let mut results = Vec::new();
    if let Ok(content) = std::fs::read_to_string("/proc/diskstats") {
        for line in content.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 14 {
                let name = fields[2].to_string();
                // Skip partition entries (they have digits in the name).
                if name.chars().last().is_some_and(|c| c.is_ascii_digit()) {
                    continue;
                }
                let sectors_read: u64 = fields[5].parse().unwrap_or(0);
                let sectors_written: u64 = fields[9].parse().unwrap_or(0);
                results.push((name, sectors_read * 512, sectors_written * 512));
            }
        }
    }
    results
}

// ---- Process count ----------------------------------------------------------

/// Count running processes by counting directories in `/proc`.
pub fn read_proc_count() -> Option<usize> {
    let mut count = 0;
    let entries = std::fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.chars().all(|c| c.is_ascii_digit()))
        {
            count += 1;
        }
    }
    Some(count)
}

// ---- CPU voltage ------------------------------------------------------------

/// Read CPU core voltage from hwmon (AMD) or MSR (Intel).
pub fn read_vcore() -> Option<f64> {
    if let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(name) = std::fs::read_to_string(path.join("name")) {
                if name.trim() == "k10temp" || name.trim() == "coretemp" {
                    if let Some(voltage) = amd_hwmon_read(&path, "in0_input") {
                        return Some(voltage / 1000.0);
                    }
                }
            }
        }
    }
    None
}

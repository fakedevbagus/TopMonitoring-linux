//! History dashboard and CSV sample management.
//!
//! Extracted from `main.rs` during v4 Fase A (Step A.1). Owns the history
//! window, CSV parsing, and the async sample-append helper.

use std::collections::VecDeque;
use std::path::PathBuf;

use gtk::glib;

/// Path to the CSV history log, `~/.config/topmonitoring/history.csv`.
pub fn history_log_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("topmonitoring");
    let _ = std::fs::create_dir_all(&path);
    path.push("history.csv");
    path
}

/// Aggregated statistics parsed from the history CSV.
#[derive(Clone, Debug, Default)]
pub struct HistorySummary {
    pub sample_count: usize,
    pub first_timestamp: String,
    pub last_timestamp: String,
    pub cpu_average: f64,
    pub cpu_maximum: f64,
    pub memory_average: f64,
    pub memory_maximum: f64,
    pub temperature_average: f64,
    pub temperature_maximum: f64,
    pub samples: Vec<(f64, f64, f64)>,
}

/// Parse the raw CSV into a summary with up to 180 recent samples.
pub fn summarize_history(raw: &str) -> HistorySummary {
    let mut summary = HistorySummary::default();
    let mut recent_samples = VecDeque::with_capacity(180);
    let mut cpu_total = 0.0;
    let mut memory_total = 0.0;
    let mut temperature_total = 0.0;
    for line in raw.lines() {
        let mut fields = line.splitn(4, ',');
        let timestamp = fields.next().unwrap_or_default().trim();
        let Some(cpu) = fields
            .next()
            .and_then(|value| value.trim().parse::<f64>().ok())
        else {
            continue;
        };
        let Some(memory) = fields
            .next()
            .and_then(|value| value.trim().parse::<f64>().ok())
        else {
            continue;
        };
        let Some(temperature) = fields
            .next()
            .and_then(|value| value.trim().parse::<f64>().ok())
        else {
            continue;
        };
        if timestamp.is_empty()
            || !cpu.is_finite()
            || !memory.is_finite()
            || !temperature.is_finite()
        {
            continue;
        }
        if summary.sample_count == 0 {
            summary.first_timestamp = timestamp.to_string();
        }
        summary.last_timestamp = timestamp.to_string();
        summary.sample_count += 1;
        cpu_total += cpu;
        memory_total += memory;
        temperature_total += temperature;
        summary.cpu_maximum = summary.cpu_maximum.max(cpu);
        summary.memory_maximum = summary.memory_maximum.max(memory);
        summary.temperature_maximum = summary.temperature_maximum.max(temperature);
        if recent_samples.len() >= 180 {
            recent_samples.pop_front();
        }
        recent_samples.push_back((cpu, memory, temperature));
    }
    summary.samples = recent_samples.into_iter().collect();
    if summary.sample_count > 0 {
        let count = summary.sample_count as f64;
        summary.cpu_average = cpu_total / count;
        summary.memory_average = memory_total / count;
        summary.temperature_average = temperature_total / count;
    }
    summary
}

/// Human-readable summary line for the dashboard header.
pub fn history_summary_text(summary: &HistorySummary) -> String {
    if summary.sample_count == 0 {
        return "No history samples yet.".into();
    }
    format!(
        "{} samples · {} → {}\nCPU avg {:.1}% / max {:.1}% · RAM avg {:.1}% / max {:.1}% · Temp avg {:.1}°C / max {:.1}°C",
        summary.sample_count,
        summary.first_timestamp,
        summary.last_timestamp,
        summary.cpu_average,
        summary.cpu_maximum,
        summary.memory_average,
        summary.memory_maximum,
        summary.temperature_average,
        summary.temperature_maximum,
    )
}

/// Queues one history row without blocking the GTK main loop.
pub fn append_history_values_async(
    cpu: f32,
    memory_percent: f64,
    temperature: f32,
    max_megabytes: u32,
) {
    let timestamp = glib::DateTime::now_local()
        .ok()
        .and_then(|date| date.format("%Y-%m-%d %H:%M:%S").ok())
        .map(|value| value.to_string())
        .unwrap_or_default();
    std::thread::spawn(move || {
        let path = history_log_path();
        let max_bytes = u64::from(max_megabytes.clamp(1, 1_024)) * 1_048_576;
        if path
            .metadata()
            .ok()
            .map(|metadata| metadata.len())
            .unwrap_or(0)
            >= max_bytes
        {
            let rotated = path.with_extension("csv.1");
            let _ = std::fs::remove_file(&rotated);
            let _ = std::fs::rename(&path, &rotated);
        }
        let mut body = match std::fs::read_to_string(&path) {
            Ok(body) => body,
            Err(_) => "timestamp,cpu,memory,temperature\n".into(),
        };
        body.push_str(&format!(
            "{},{:.1},{:.1},{:.1}\n",
            timestamp, cpu, memory_percent, temperature
        ));
        let _ = std::fs::write(&path, body);
    });
}

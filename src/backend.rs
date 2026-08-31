use super::{
    amd_active_clock, amd_card_device, amd_hwmon_read, disk_temps, format_disk_bar,
    format_disk_compact, format_disk_tiny, human_bytes, human_rate, human_uptime, read_amd_gpu,
    read_battery, read_cpu_power, read_disk_io_breakdown, read_fan_rpm, read_intel_gpu_clock,
    read_proc_count, read_vcore, DiskSummary,
};
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{mpsc, Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Components, Disks, Networks, System};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum MetricLevel {
    #[default]
    None,
    Higher {
        value: f64,
        default_warn: f64,
        default_crit: f64,
        key: &'static str,
        title: &'static str,
    },
    Lower {
        value: f64,
        default_warn: f64,
        default_crit: f64,
        key: &'static str,
        title: &'static str,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricSample {
    pub value: String,
    pub compact_value: Option<String>,
    pub tiny_value: Option<String>,
    pub tooltip: Option<String>,
    pub level: MetricLevel,
    pub dimmed: bool,
}

impl MetricSample {
    fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            ..Self::default()
        }
    }

    fn unavailable() -> Self {
        Self::new("n/a")
    }

    fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    fn with_width_values(
        mut self,
        compact_value: impl Into<String>,
        tiny_value: impl Into<String>,
    ) -> Self {
        self.compact_value = Some(compact_value.into());
        self.tiny_value = Some(tiny_value.into());
        self
    }

    fn with_higher_level(
        mut self,
        value: f64,
        default_warn: f64,
        default_crit: f64,
        key: &'static str,
        title: &'static str,
    ) -> Self {
        self.level = MetricLevel::Higher {
            value,
            default_warn,
            default_crit,
            key,
            title,
        };
        self
    }

    fn with_lower_level(
        mut self,
        value: f64,
        default_warn: f64,
        default_crit: f64,
        key: &'static str,
        title: &'static str,
    ) -> Self {
        self.level = MetricLevel::Lower {
            value,
            default_warn,
            default_crit,
            key,
            title,
        };
        self
    }

    fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoreGraphValues {
    pub cpu: f64,
    pub memory: f64,
    pub network_down: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoreSnapshot {
    pub metrics: BTreeMap<String, MetricSample>,
    pub graphs: CoreGraphValues,
    pub interface_names: Vec<String>,
    pub cpu_percent: f32,
    pub memory_percent: f64,
    pub temperature_celsius: f32,
    pub generation: u64,
    pub collected_at_epoch_secs: u64,
    pub duration_ms: u128,
    pub stale_after_secs: u64,
}

impl CoreSnapshot {
    pub fn sample(&self, id: &str) -> Option<&MetricSample> {
        self.metrics.get(id)
    }

    pub fn graph_value(&self, id: &str) -> Option<f64> {
        match id {
            "cpu_graph" => Some(self.graphs.cpu),
            "ram_graph" => Some(self.graphs.memory),
            "net_graph" => Some(self.graphs.network_down),
            _ => None,
        }
    }

    pub fn metric_changed(&self, id: &str, previous: Option<&Self>) -> bool {
        previous
            .map(|previous| previous.metrics.get(id) != self.metrics.get(id))
            .unwrap_or(true)
    }

    pub fn graph_changed(&self, id: &str, previous: Option<&Self>) -> bool {
        previous
            .map(|previous| previous.graph_value(id) != self.graph_value(id))
            .unwrap_or(true)
    }

    pub fn sanitized_diagnostics(&self) -> String {
        let age = if self.generation > 0 {
            Some(epoch_seconds().saturating_sub(self.collected_at_epoch_secs))
        } else {
            None
        };
        let status = if self.generation == 0 {
            "unavailable"
        } else if age.unwrap_or_default() > self.stale_after_secs {
            "stale"
        } else {
            "available"
        };
        format!(
            "core={} lane=core age={} duration={}ms generation={} stale_after={}s metrics={} interfaces={}",
            status,
            age.map(|seconds| format!("{seconds}s"))
                .unwrap_or_else(|| "never".into()),
            self.duration_ms,
            self.generation,
            self.stale_after_secs,
            self.metrics.len(),
            self.interface_names.len(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreCollectionConfig {
    pub interval_ms: u64,
    pub net_iface: String,
    pub gpu_index: u32,
}

impl CoreCollectionConfig {
    pub fn new(interval_ms: u64, net_iface: impl Into<String>, gpu_index: u32) -> Self {
        Self {
            interval_ms: interval_ms.clamp(250, 10_000),
            net_iface: net_iface.into(),
            gpu_index,
        }
    }
}

impl Default for CoreCollectionConfig {
    fn default() -> Self {
        Self::new(1_000, "", 0)
    }
}

#[derive(Debug)]
enum CoreRequest {
    Configure(CoreCollectionConfig),
}

#[derive(Clone)]
pub struct CoreCollector {
    state: Arc<RwLock<CoreSnapshot>>,
    sender: mpsc::Sender<CoreRequest>,
}

impl CoreCollector {
    pub fn start(config: CoreCollectionConfig) -> Self {
        let state = Arc::new(RwLock::new(CoreSnapshot::default()));
        let (sender, receiver) = mpsc::channel();
        let worker_state = state.clone();
        thread::Builder::new()
            .name("topmonitoring-core-collector".into())
            .spawn(move || core_worker_loop(worker_state, receiver, config))
            .expect("failed to start TopMonitoring core collector");
        Self { state, sender }
    }

    pub fn configure(&self, config: CoreCollectionConfig) {
        let _ = self.sender.send(CoreRequest::Configure(config));
    }

    pub fn snapshot(&self) -> CoreSnapshot {
        self.state
            .read()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_default()
    }
}

struct NativeCollector {
    system: System,
    components: Components,
    networks: Networks,
    disks: Disks,
    nvml: Option<Nvml>,
    last_collection: Instant,
}

impl NativeCollector {
    fn new() -> Self {
        Self {
            system: System::new_all(),
            components: Components::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list(),
            nvml: Nvml::init().ok(),
            last_collection: Instant::now(),
        }
    }

    fn collect(&mut self, config: &CoreCollectionConfig, generation: u64) -> CoreSnapshot {
        let started = Instant::now();
        let elapsed_secs = started
            .duration_since(self.last_collection)
            .as_secs_f64()
            .max(0.001);
        self.last_collection = started;

        self.system.refresh_cpu_all();
        self.system.refresh_memory();
        self.components.refresh(true);
        self.networks.refresh(true);
        self.disks.refresh(true);

        let mut snapshot = build_snapshot(
            &self.system,
            &self.components,
            &self.networks,
            &self.disks,
            &self.nvml,
            config,
            elapsed_secs,
        );
        snapshot.generation = generation;
        snapshot.collected_at_epoch_secs = epoch_seconds();
        snapshot.duration_ms = started.elapsed().as_millis();
        snapshot.stale_after_secs = config.interval_ms.saturating_mul(3).div_ceil(1_000).max(2);
        snapshot
    }
}

fn core_worker_loop(
    state: Arc<RwLock<CoreSnapshot>>,
    receiver: mpsc::Receiver<CoreRequest>,
    mut config: CoreCollectionConfig,
) {
    let mut collector = NativeCollector::new();
    let mut next_collection = Instant::now();
    let mut generation = 0_u64;
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(CoreRequest::Configure(next)) => {
                if next != config {
                    config = next;
                    next_collection = Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(CoreRequest::Configure(next)) = receiver.try_recv() {
            if next != config {
                config = next;
                next_collection = Instant::now();
            }
        }
        if Instant::now() < next_collection {
            continue;
        }
        generation = generation.wrapping_add(1);
        let snapshot = collector.collect(&config, generation);
        if let Ok(mut current) = state.write() {
            *current = snapshot;
        }
        next_collection = Instant::now() + Duration::from_millis(config.interval_ms);
    }
}

fn build_snapshot(
    system: &System,
    components: &Components,
    networks: &Networks,
    disks: &Disks,
    nvml: &Option<Nvml>,
    config: &CoreCollectionConfig,
    elapsed_secs: f64,
) -> CoreSnapshot {
    let mut metrics = BTreeMap::new();

    let cpu = system.global_cpu_usage() as f64;
    let mut cpu_tooltip = String::from("Per core:");
    for (index, core) in system.cpus().iter().enumerate() {
        cpu_tooltip.push_str(&format!("\ncore{index}: {:.0}%", core.cpu_usage()));
    }
    metrics.insert(
        "cpu".into(),
        MetricSample::new(format!("{cpu:>3.0}%"))
            .with_tooltip(cpu_tooltip)
            .with_higher_level(cpu, 70.0, 90.0, "cpu", "CPU"),
    );

    let cpu_model = system
        .cpus()
        .first()
        .map(|core| core.brand().trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "n/a".into());
    metrics.insert("cpumodel".into(), MetricSample::new(cpu_model));

    let average_mhz = if system.cpus().is_empty() {
        None
    } else {
        Some(
            system
                .cpus()
                .iter()
                .map(|core| core.frequency() as f64)
                .sum::<f64>()
                / system.cpus().len() as f64,
        )
    };
    metrics.insert(
        "freq".into(),
        average_mhz
            .map(|mhz| {
                MetricSample::new(format!("{:.1}GHz", mhz / 1_000.0))
                    .with_tooltip("Average current frequency across logical CPUs")
            })
            .unwrap_or_else(MetricSample::unavailable),
    );

    metrics.insert(
        "cpu_power".into(),
        read_cpu_power()
            .map(|watts| MetricSample::new(format!("{watts:.0}W")))
            .unwrap_or_else(MetricSample::unavailable),
    );
    metrics.insert(
        "vcore".into(),
        read_vcore()
            .map(|volts| MetricSample::new(format!("{volts:.2}V")))
            .unwrap_or_else(MetricSample::unavailable),
    );

    let used_memory = system.used_memory() as f64;
    let total_memory = system.total_memory() as f64;
    let memory_percent = if total_memory > 0.0 {
        used_memory / total_memory * 100.0
    } else {
        0.0
    };
    metrics.insert(
        "memory".into(),
        MetricSample::new(format!(
            "{:.1}/{:.0}G",
            used_memory / 1e9,
            total_memory / 1e9
        ))
        .with_tooltip(format!(
            "RAM: {:.0}/{:.0} MB\nSwap: {:.0}/{:.0} MB",
            used_memory / 1e6,
            total_memory / 1e6,
            system.used_swap() as f64 / 1e6,
            system.total_swap() as f64 / 1e6,
        ))
        .with_higher_level(memory_percent, 80.0, 92.0, "mem", "RAM"),
    );
    metrics.insert(
        "memavail".into(),
        MetricSample::new(format!("{:.1}G", system.available_memory() as f64 / 1e9)),
    );
    let total_swap = system.total_swap().max(1) as f64;
    metrics.insert(
        "swap".into(),
        MetricSample::new(format!(
            "{:>3.0}%",
            system.used_swap() as f64 / total_swap * 100.0
        )),
    );

    let temperature = components
        .iter()
        .filter_map(|component| component.temperature())
        .fold(0.0_f32, f32::max);
    let mut temperature_tooltip = String::from("Sensors:");
    for component in components.iter() {
        if let Some(value) = component.temperature() {
            temperature_tooltip.push_str(&format!("\n{}: {:.0}\u{b0}C", component.label(), value));
        }
    }
    metrics.insert(
        "temp".into(),
        MetricSample::new(format!("{temperature:>2.0}\u{b0}C"))
            .with_tooltip(temperature_tooltip)
            .with_higher_level(temperature as f64, 75.0, 90.0, "temp", "Temperature"),
    );
    metrics.insert(
        "fan".into(),
        read_fan_rpm()
            .map(|rpm| MetricSample::new(format!("{rpm}rpm")))
            .unwrap_or_else(MetricSample::unavailable),
    );

    collect_gpu_metrics(&mut metrics, nvml, config.gpu_index);

    let disk_entries = collect_disks(disks);
    if disk_entries.is_empty() {
        metrics.insert("disk".into(), MetricSample::unavailable());
    } else {
        let worst = disk_entries
            .iter()
            .map(|entry| entry.used_percent)
            .fold(0.0_f64, f64::max);
        let mut tooltip =
            String::from("Mounted disks (bar shows root and the fullest additional volume):");
        for entry in &disk_entries {
            tooltip.push_str(&format!(
                "\n{} ({}): {:.1}/{:.0} GB free \u{2014} {} \u{2014} {:.0}% used",
                entry.mount,
                entry.name,
                entry.available_gb,
                entry.total_gb,
                entry.file_system,
                entry.used_percent,
            ));
        }
        let temperatures = disk_temps();
        if !temperatures.is_empty() {
            tooltip.push_str("\nDisk temperatures:");
            for (label, value) in temperatures {
                tooltip.push_str(&format!("\n{label}: {value:.0}\u{b0}C"));
            }
        }
        metrics.insert(
            "disk".into(),
            MetricSample::new(format_disk_bar(&disk_entries))
                .with_width_values(
                    format_disk_compact(&disk_entries),
                    format_disk_tiny(&disk_entries),
                )
                .with_tooltip(tooltip)
                .with_higher_level(worst, 85.0, 95.0, "disk", "Disk"),
        );
    }

    let disk_io = read_disk_io_breakdown();
    if disk_io.is_empty() {
        metrics.insert("diskio".into(), MetricSample::unavailable());
    } else {
        let busiest = disk_io.iter().max_by(|left, right| {
            (left.1 + left.2)
                .partial_cmp(&(right.1 + right.2))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let value = busiest
            .map(|(name, read, write)| {
                format!(
                    "{name} R{} W{}",
                    human_rate(*read as u64),
                    human_rate(*write as u64)
                )
            })
            .unwrap_or_else(|| "n/a".into());
        let mut tooltip = String::from("Per-disk throughput:");
        for (name, read, write) in &disk_io {
            tooltip.push_str(&format!(
                "\n{name}: R{} W{}",
                human_rate(*read as u64),
                human_rate(*write as u64)
            ));
        }
        metrics.insert(
            "diskio".into(),
            MetricSample::new(value).with_tooltip(tooltip),
        );
    }

    let mut interface_names: Vec<String> = networks.keys().cloned().collect();
    interface_names.sort();
    let mut received_per_second = 0.0_f64;
    let mut transmitted_per_second = 0.0_f64;
    let mut network_tooltip = if config.net_iface.is_empty() {
        String::from("Interfaces:")
    } else {
        format!("Interface: {}", config.net_iface)
    };
    let mut found_interface = false;
    let mut total_received = 0_u64;
    let mut total_transmitted = 0_u64;
    let mut totals_tooltip = String::from("Total since boot:");
    let mut vpn_active = false;
    for (name, data) in networks.iter() {
        let normalized = name.to_lowercase();
        if normalized.starts_with("tun")
            || normalized.starts_with("tap")
            || normalized.starts_with("wg")
            || normalized.contains("proton")
            || normalized.contains("nordlynx")
        {
            vpn_active = true;
        }
        if !config.net_iface.is_empty() && name != &config.net_iface {
            continue;
        }
        found_interface = true;
        let received = data.received() as f64 / elapsed_secs.max(0.001);
        let transmitted = data.transmitted() as f64 / elapsed_secs.max(0.001);
        received_per_second += received;
        transmitted_per_second += transmitted;
        total_received = total_received.saturating_add(data.total_received());
        total_transmitted = total_transmitted.saturating_add(data.total_transmitted());
        network_tooltip.push_str(&format!(
            "\n{name}: down {} / up {}",
            human_rate(received as u64),
            human_rate(transmitted as u64)
        ));
        totals_tooltip.push_str(&format!(
            "\n{name}: down {} / up {}",
            human_bytes(data.total_received()),
            human_bytes(data.total_transmitted())
        ));
    }
    metrics.insert(
        "network".into(),
        if found_interface {
            MetricSample::new(format!(
                "\u{2193}{} \u{2191}{}",
                human_rate(received_per_second as u64),
                human_rate(transmitted_per_second as u64)
            ))
            .with_tooltip(network_tooltip)
        } else {
            MetricSample::unavailable()
        },
    );
    metrics.insert(
        "netttl".into(),
        MetricSample::new(format!(
            "\u{3a3}\u{2193}{} \u{2191}{}",
            human_bytes(total_received),
            human_bytes(total_transmitted)
        ))
        .with_tooltip(totals_tooltip),
    );
    metrics.insert(
        "vpn".into(),
        MetricSample::new(if vpn_active { "on" } else { "off" }).dimmed(!vpn_active),
    );

    metrics.insert(
        "battery".into(),
        read_battery()
            .map(|(capacity, status)| {
                let icon = if status == "Charging" {
                    "\u{26a1}"
                } else {
                    "\u{1f50b}"
                };
                MetricSample::new(format!("{icon}{capacity}%"))
                    .with_tooltip(format!("Status: {status}"))
                    .with_lower_level(capacity as f64, 20.0, 10.0, "battery", "Battery")
            })
            .unwrap_or_else(MetricSample::unavailable),
    );
    metrics.insert(
        "procs".into(),
        MetricSample::new(read_proc_count().to_string()),
    );
    metrics.insert(
        "uptime".into(),
        MetricSample::new(human_uptime(System::uptime())),
    );
    let load = System::load_average();
    metrics.insert(
        "load".into(),
        MetricSample::new(format!("{:.2}", load.one)).with_tooltip(format!(
            "1m {:.2}  5m {:.2}  15m {:.2}",
            load.one, load.five, load.fifteen
        )),
    );
    metrics.insert(
        "host".into(),
        MetricSample::new(System::host_name().unwrap_or_else(|| "n/a".into())),
    );
    metrics.insert(
        "kernel".into(),
        MetricSample::new(System::kernel_version().unwrap_or_else(|| "n/a".into())),
    );
    let os = format!(
        "{} {}",
        System::name().unwrap_or_default(),
        System::os_version().unwrap_or_default()
    );
    metrics.insert(
        "os".into(),
        MetricSample::new(if os.trim().is_empty() {
            "n/a"
        } else {
            os.trim()
        }),
    );

    CoreSnapshot {
        metrics,
        graphs: CoreGraphValues {
            cpu,
            memory: memory_percent,
            network_down: received_per_second,
        },
        interface_names,
        cpu_percent: cpu as f32,
        memory_percent,
        temperature_celsius: temperature,
        ..CoreSnapshot::default()
    }
}

fn collect_gpu_metrics(
    metrics: &mut BTreeMap<String, MetricSample>,
    nvml: &Option<Nvml>,
    gpu_index: u32,
) {
    let mut gpu = None;
    if let Some(nvml) = nvml {
        if let Ok(device) = nvml.device_by_index(gpu_index) {
            let utilization = device
                .utilization_rates()
                .map(|value| value.gpu)
                .unwrap_or(0);
            let temperature = device.temperature(TemperatureSensor::Gpu).unwrap_or(0);
            let memory = device.memory_info().ok();
            let vram = memory
                .as_ref()
                .map(|memory| {
                    format!(
                        " {:.1}/{:.0}G",
                        memory.used as f64 / 1e9,
                        memory.total as f64 / 1e9
                    )
                })
                .unwrap_or_default();
            let mut tooltip =
                format!("Utilization: {utilization}%\nTemperature: {temperature}\u{b0}C");
            if let Some(memory) = memory {
                tooltip.push_str(&format!(
                    "\nVRAM: {:.0}/{:.0} MB",
                    memory.used as f64 / 1e6,
                    memory.total as f64 / 1e6
                ));
            }
            if let Ok(power) = device.power_usage() {
                tooltip.push_str(&format!("\nPower: {:.0} W", power as f64 / 1000.0));
            }
            gpu = Some(
                MetricSample::new(format!("{utilization}% {temperature}\u{b0}C{vram}"))
                    .with_tooltip(tooltip)
                    .with_higher_level(utilization as f64, 80.0, 92.0, "gpu", "GPU"),
            );
        }
    }
    if gpu.is_none() {
        gpu = read_amd_gpu(gpu_index).map(|(utilization, temperature)| {
            let suffix = temperature
                .map(|temperature| format!(" {temperature}\u{b0}C"))
                .unwrap_or_default();
            MetricSample::new(format!("{utilization}%{suffix}"))
                .with_tooltip(format!("Utilization: {utilization}%"))
                .with_higher_level(utilization as f64, 80.0, 92.0, "gpu", "GPU")
        });
    }
    metrics.insert("gpu".into(), gpu.unwrap_or_else(MetricSample::unavailable));

    let power = nvml
        .as_ref()
        .and_then(|nvml| nvml.device_by_index(gpu_index).ok())
        .and_then(|device| device.power_usage().ok())
        .map(|milliwatts| milliwatts as f64 / 1000.0)
        .or_else(|| {
            amd_card_device(gpu_index)
                .and_then(|device| {
                    amd_hwmon_read(&device, "power1_average")
                        .or_else(|| amd_hwmon_read(&device, "power1_input"))
                })
                .map(|microwatts| microwatts / 1e6)
        });
    metrics.insert(
        "gpu_power".into(),
        power
            .map(|watts| MetricSample::new(format!("{watts:.0}W")))
            .unwrap_or_else(MetricSample::unavailable),
    );

    let graphics_clock = nvml
        .as_ref()
        .and_then(|nvml| nvml.device_by_index(gpu_index).ok())
        .and_then(|device| device.clock_info(Clock::Graphics).ok())
        .or_else(|| {
            amd_card_device(gpu_index).and_then(|device| amd_active_clock(&device, "pp_dpm_sclk"))
        })
        .or_else(|| read_intel_gpu_clock(gpu_index));
    metrics.insert(
        "gpu_clock".into(),
        graphics_clock
            .map(|clock| MetricSample::new(format!("{clock}MHz")))
            .unwrap_or_else(MetricSample::unavailable),
    );

    let memory_clock = nvml
        .as_ref()
        .and_then(|nvml| nvml.device_by_index(gpu_index).ok())
        .and_then(|device| device.clock_info(Clock::Memory).ok())
        .or_else(|| {
            amd_card_device(gpu_index).and_then(|device| amd_active_clock(&device, "pp_dpm_mclk"))
        });
    metrics.insert(
        "gpu_memclock".into(),
        memory_clock
            .map(|clock| MetricSample::new(format!("{clock}MHz")))
            .unwrap_or_else(MetricSample::unavailable),
    );

    let fan = nvml
        .as_ref()
        .and_then(|nvml| nvml.device_by_index(gpu_index).ok())
        .and_then(|device| device.fan_speed(0).ok())
        .map(|percent| format!("{percent}%"))
        .or_else(|| {
            amd_card_device(gpu_index)
                .and_then(|device| amd_hwmon_read(&device, "fan1_input"))
                .map(|rpm| format!("{rpm:.0}rpm"))
        });
    metrics.insert(
        "gpu_fan".into(),
        fan.map(MetricSample::new)
            .unwrap_or_else(MetricSample::unavailable),
    );
}

fn collect_disks(disks: &Disks) -> Vec<DiskSummary> {
    const VIRTUAL_FILESYSTEMS: &[&str] = &[
        "tmpfs",
        "devtmpfs",
        "overlay",
        "overlayfs",
        "squashfs",
        "proc",
        "sysfs",
        "cgroup",
        "cgroup2",
        "debugfs",
        "tracefs",
        "configfs",
        "fusectl",
        "pstore",
        "bpf",
        "autofs",
        "mqueue",
        "hugetlbfs",
        "securityfs",
        "devpts",
        "binfmt_misc",
        "efivarfs",
        "ramfs",
    ];
    let mut entries = Vec::new();
    for disk in disks.iter() {
        let file_system = disk.file_system().to_string_lossy().to_lowercase();
        if VIRTUAL_FILESYSTEMS.contains(&file_system.as_str()) {
            continue;
        }
        let total = disk.total_space() as f64;
        if total <= 0.0 {
            continue;
        }
        let available = disk.available_space() as f64;
        let used_percent = (total - available) / total * 100.0;
        let mount = disk.mount_point();
        let name = if mount == Path::new("/") {
            "root".to_string()
        } else {
            mount
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| mount.to_string_lossy().to_string())
        };
        entries.push(DiskSummary {
            name,
            mount: mount.to_string_lossy().to_string(),
            file_system,
            used_percent,
            available_gb: available / 1e9,
            total_gb: total / 1e9,
        });
    }
    entries
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_diff_only_flags_changed_sample() {
        let mut previous = CoreSnapshot::default();
        previous
            .metrics
            .insert("cpu".into(), MetricSample::new("10%"));
        previous
            .metrics
            .insert("memory".into(), MetricSample::new("20%"));
        let mut current = previous.clone();
        current
            .metrics
            .insert("cpu".into(), MetricSample::new("11%"));
        assert!(current.metric_changed("cpu", Some(&previous)));
        assert!(!current.metric_changed("memory", Some(&previous)));
    }

    #[test]
    fn graph_diff_uses_owned_snapshot_values() {
        let previous = CoreSnapshot {
            graphs: CoreGraphValues {
                cpu: 10.0,
                memory: 20.0,
                network_down: 30.0,
            },
            ..CoreSnapshot::default()
        };
        let mut current = previous.clone();
        current.graphs.network_down = 31.0;
        assert!(!current.graph_changed("cpu_graph", Some(&previous)));
        assert!(current.graph_changed("net_graph", Some(&previous)));
    }

    #[test]
    fn core_diagnostics_do_not_include_metric_values() {
        let mut snapshot = CoreSnapshot {
            generation: 4,
            collected_at_epoch_secs: epoch_seconds(),
            ..CoreSnapshot::default()
        };
        snapshot
            .metrics
            .insert("host".into(), MetricSample::new("private-workstation-name"));
        let diagnostics = snapshot.sanitized_diagnostics();
        assert!(diagnostics.contains("lane=core"));
        assert!(!diagnostics.contains("private-workstation-name"));
    }

    #[test]
    fn collection_config_clamps_interval() {
        assert_eq!(CoreCollectionConfig::new(1, "", 0).interval_ms, 250);
        assert_eq!(
            CoreCollectionConfig::new(99_999, "eth0", 2).interval_ms,
            10_000
        );
    }

    #[test]
    fn metric_sample_keeps_owned_width_tiers() {
        let sample = MetricSample::new("normal").with_width_values("compact", "tiny");
        assert_eq!(sample.compact_value.as_deref(), Some("compact"));
        assert_eq!(sample.tiny_value.as_deref(), Some("tiny"));
    }
}

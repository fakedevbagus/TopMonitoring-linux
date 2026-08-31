use crate::layout::{module_width_profile, WidthProfile};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModuleCategory {
    Time,
    System,
    Processor,
    Memory,
    Thermal,
    Graphics,
    Storage,
    Network,
    Connectivity,
    Media,
    Power,
    Controls,
    Packages,
}

impl ModuleCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Time => "Time",
            Self::System => "System",
            Self::Processor => "Processor",
            Self::Memory => "Memory",
            Self::Thermal => "Thermal",
            Self::Graphics => "Graphics",
            Self::Storage => "Storage",
            Self::Network => "Network",
            Self::Connectivity => "Connectivity",
            Self::Media => "Media",
            Self::Power => "Power",
            Self::Controls => "Controls",
            Self::Packages => "Packages",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub category: ModuleCategory,
    pub aliases: &'static str,
    pub capability: &'static str,
}

impl ModuleDescriptor {
    pub fn width_profile(&self) -> WidthProfile {
        module_width_profile(self.id)
    }
}

const fn descriptor(
    id: &'static str,
    display_name: &'static str,
    category: ModuleCategory,
    aliases: &'static str,
    capability: &'static str,
) -> ModuleDescriptor {
    ModuleDescriptor {
        id,
        display_name,
        category,
        aliases,
        capability,
    }
}

pub const MODULE_DESCRIPTORS: &[ModuleDescriptor] = &[
    descriptor(
        "clock",
        "Clock",
        ModuleCategory::Time,
        "time hour",
        "local time",
    ),
    descriptor(
        "date",
        "Date",
        ModuleCategory::Time,
        "calendar day",
        "local time",
    ),
    descriptor(
        "host",
        "Host name",
        ModuleCategory::System,
        "computer machine hostname",
        "sysinfo",
    ),
    descriptor(
        "os",
        "Operating system",
        ModuleCategory::System,
        "linux distribution distro",
        "sysinfo",
    ),
    descriptor(
        "kernel",
        "Kernel",
        ModuleCategory::System,
        "linux release version",
        "sysinfo",
    ),
    descriptor(
        "cpu",
        "CPU usage",
        ModuleCategory::Processor,
        "processor core utilization",
        "sysinfo",
    ),
    descriptor(
        "cpu_graph",
        "CPU graph",
        ModuleCategory::Processor,
        "processor history sparkline",
        "sysinfo",
    ),
    descriptor(
        "cpumodel",
        "CPU model",
        ModuleCategory::Processor,
        "processor brand hardware",
        "sysinfo",
    ),
    descriptor(
        "freq",
        "CPU frequency",
        ModuleCategory::Processor,
        "clock ghz mhz speed",
        "sysinfo",
    ),
    descriptor(
        "cpu_power",
        "CPU package power",
        ModuleCategory::Power,
        "processor watts rapl",
        "Intel RAPL",
    ),
    descriptor(
        "vcore",
        "CPU core voltage",
        ModuleCategory::Power,
        "processor voltage volts",
        "hwmon",
    ),
    descriptor(
        "memory",
        "Memory usage",
        ModuleCategory::Memory,
        "ram used total",
        "sysinfo",
    ),
    descriptor(
        "ram_graph",
        "Memory graph",
        ModuleCategory::Memory,
        "ram history sparkline",
        "sysinfo",
    ),
    descriptor(
        "memavail",
        "Available memory",
        ModuleCategory::Memory,
        "ram free unused",
        "sysinfo",
    ),
    descriptor(
        "swap",
        "Swap usage",
        ModuleCategory::Memory,
        "paging virtual memory",
        "sysinfo",
    ),
    descriptor(
        "temp",
        "Temperature",
        ModuleCategory::Thermal,
        "thermal sensor cpu heat",
        "sysinfo components",
    ),
    descriptor(
        "fan",
        "Fan speed",
        ModuleCategory::Thermal,
        "cooling rpm sensor",
        "hwmon",
    ),
    descriptor(
        "gpu",
        "GPU usage",
        ModuleCategory::Graphics,
        "graphics video utilization vram",
        "NVML or DRM sysfs",
    ),
    descriptor(
        "gpu_power",
        "GPU power",
        ModuleCategory::Graphics,
        "graphics watts",
        "NVML or DRM sysfs",
    ),
    descriptor(
        "gpu_clock",
        "GPU clock",
        ModuleCategory::Graphics,
        "graphics core mhz frequency",
        "NVML or DRM sysfs",
    ),
    descriptor(
        "gpu_memclock",
        "GPU memory clock",
        ModuleCategory::Graphics,
        "graphics vram mhz",
        "NVML or DRM sysfs",
    ),
    descriptor(
        "gpu_fan",
        "GPU fan",
        ModuleCategory::Graphics,
        "graphics cooling rpm percent",
        "NVML or DRM sysfs",
    ),
    descriptor(
        "disk",
        "Disk usage",
        ModuleCategory::Storage,
        "hdd ssd drive storage filesystem mount volume",
        "sysinfo disks and hwmon",
    ),
    descriptor(
        "diskio",
        "Disk throughput",
        ModuleCategory::Storage,
        "hdd ssd io read write speed",
        "/proc/diskstats",
    ),
    descriptor(
        "network",
        "Network speed",
        ModuleCategory::Network,
        "ethernet bandwidth upload download traffic",
        "sysinfo networks",
    ),
    descriptor(
        "net_graph",
        "Network graph",
        ModuleCategory::Network,
        "download history bandwidth sparkline",
        "sysinfo networks",
    ),
    descriptor(
        "netttl",
        "Network totals",
        ModuleCategory::Network,
        "lifetime upload download bytes",
        "sysinfo networks",
    ),
    descriptor(
        "media",
        "Media player",
        ModuleCategory::Media,
        "music mpris song playback",
        "playerctl",
    ),
    descriptor(
        "wifi",
        "Wi-Fi",
        ModuleCategory::Connectivity,
        "wireless wlan signal ssid",
        "/proc and NetworkManager",
    ),
    descriptor(
        "vpn",
        "VPN",
        ModuleCategory::Connectivity,
        "tunnel wireguard openvpn proton",
        "network interfaces",
    ),
    descriptor(
        "bluetooth",
        "Bluetooth",
        ModuleCategory::Connectivity,
        "bt wireless device",
        "bluetoothctl",
    ),
    descriptor(
        "battery",
        "Battery",
        ModuleCategory::Power,
        "charge laptop power",
        "power_supply sysfs",
    ),
    descriptor(
        "volume",
        "Audio volume",
        ModuleCategory::Controls,
        "sound speaker mute",
        "PipeWire or PulseAudio",
    ),
    descriptor(
        "brightness",
        "Display brightness",
        ModuleCategory::Controls,
        "screen backlight light",
        "brightnessctl or sysfs",
    ),
    descriptor(
        "pkg_updates",
        "Package updates",
        ModuleCategory::Packages,
        "apt dnf pacman flatpak upgrade",
        "package managers",
    ),
    descriptor(
        "procs",
        "Process count",
        ModuleCategory::System,
        "tasks applications running",
        "/proc",
    ),
    descriptor(
        "uptime",
        "Uptime",
        ModuleCategory::System,
        "running duration boot",
        "sysinfo",
    ),
    descriptor(
        "load",
        "System load",
        ModuleCategory::System,
        "load average one five fifteen",
        "sysinfo",
    ),
];

pub fn module_registry() -> &'static [ModuleDescriptor] {
    MODULE_DESCRIPTORS
}

pub fn module_descriptor(id: &str) -> Option<&'static ModuleDescriptor> {
    MODULE_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.id == id)
}

pub fn module_search_text(id: &str, configured_label: &str, fallback_label: &str) -> String {
    match module_descriptor(id) {
        Some(descriptor) => format!(
            "{} {} {} {} {} {} {}",
            descriptor.id,
            descriptor.display_name,
            descriptor.category.label(),
            descriptor.aliases,
            descriptor.capability,
            configured_label,
            fallback_label,
        ),
        None => format!("{id} {configured_label} {fallback_label} module telemetry monitor"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_ids_are_unique_and_complete() {
        let ids: HashSet<&str> = module_registry()
            .iter()
            .map(|descriptor| descriptor.id)
            .collect();
        assert_eq!(ids.len(), module_registry().len());
        assert_eq!(module_registry().len(), 38);
    }

    #[test]
    fn registry_covers_every_default_metric() {
        for metric in crate::config::default_metrics() {
            assert!(
                module_descriptor(&metric.id).is_some(),
                "missing descriptor for {}",
                metric.id
            );
        }
    }

    #[test]
    fn storage_descriptor_contains_expected_aliases() {
        let search = module_search_text("disk", "DISK", "DISK").to_lowercase();
        assert!(search.contains("hdd"));
        assert!(search.contains("ssd"));
        assert!(search.contains("storage"));
    }

    #[test]
    fn registry_width_profiles_are_monotonic() {
        for descriptor in module_registry() {
            let width = descriptor.width_profile();
            assert!(width.normal_px >= width.compact_px);
            assert!(width.compact_px >= width.tiny_px);
        }
    }
}

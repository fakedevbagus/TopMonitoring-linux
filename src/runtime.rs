use crate::command::{run_program_limited, CommandPolicy};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RELEASE_API: &str =
    "https://api.github.com/repos/fakedevbagus/TopMonitoring-linux/releases/latest";
const RELEASE_PAGE: &str = "https://github.com/fakedevbagus/TopMonitoring-linux/releases/latest";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaState {
    pub status: String,
    pub artist: String,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageState {
    pub apt: u32,
    pub dnf: u32,
    pub pacman: u32,
    pub flatpak: u32,
    pub available_backends: u32,
    pub backend_details: Vec<String>,
}

impl PackageState {
    pub fn total(&self) -> u32 {
        self.apt
            .saturating_add(self.dnf)
            .saturating_add(self.pacman)
            .saturating_add(self.flatpak)
    }

    pub fn tooltip(&self) -> String {
        if self.available_backends == 0 {
            "No supported package backend detected".into()
        } else if self.backend_details.is_empty() {
            format!("{} package backends available", self.available_backends)
        } else {
            self.backend_details.join("\n")
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionFeedback {
    pub request_id: u64,
    pub success: bool,
    pub provider: String,
    pub message: String,
    pub duration_ms: u128,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProviderStatus {
    Available,
    #[default]
    Unavailable,
    InFlight,
    Stale,
    Error,
}

impl ProviderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::InFlight => "in-flight",
            Self::Stale => "stale",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderHealth {
    pub id: String,
    pub lane: String,
    pub status: ProviderStatus,
    pub last_attempt_epoch_secs: u64,
    pub last_success_epoch_secs: Option<u64>,
    pub duration_ms: u128,
    pub consecutive_failures: u32,
    pub stale_after_secs: u64,
    pub generation: u64,
    pub detail: String,
}

impl ProviderHealth {
    pub fn status_at(&self, now_epoch_secs: u64) -> ProviderStatus {
        if self.status == ProviderStatus::Available
            && self
                .last_success_epoch_secs
                .map(|last| now_epoch_secs.saturating_sub(last) > self.stale_after_secs)
                .unwrap_or(true)
        {
            ProviderStatus::Stale
        } else {
            self.status
        }
    }

    pub fn age_secs(&self, now_epoch_secs: u64) -> Option<u64> {
        self.last_success_epoch_secs
            .map(|last| now_epoch_secs.saturating_sub(last))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalSnapshot {
    pub wifi: Option<(String, i32)>,
    pub packages: Option<PackageState>,
    pub media: Option<MediaState>,
    pub volume: Option<(u32, bool)>,
    pub brightness: Option<u32>,
    pub bluetooth: Option<(bool, Vec<String>)>,
    pub update_status: Option<String>,
    pub last_action_status: Option<String>,
    pub last_action_provider: Option<String>,
    pub action_request_id: u64,
    pub action_pending: bool,
    pub action_success: Option<bool>,
    pub action_duration_ms: u128,
    pub action_results: Vec<ActionFeedback>,
    pub provider_health: Vec<ProviderHealth>,
    pub generation: u64,
}

impl ExternalSnapshot {
    pub fn sanitized_diagnostics(&self) -> String {
        let now = epoch_seconds();
        let mut health = self.provider_health.clone();
        health.sort_by(|left, right| left.id.cmp(&right.id));
        let mut lines = vec![
            "TopMonitoring provider diagnostics (redacted)".to_string(),
            format!("generation={}", self.generation),
        ];
        for provider in health {
            let attempt_age = now.saturating_sub(provider.last_attempt_epoch_secs);
            let age = provider
                .age_secs(now)
                .map(|seconds| format!("{seconds}s"))
                .unwrap_or_else(|| "never".into());
            lines.push(format!(
                "{}={} lane={} attempt_age={}s success_age={} duration={}ms failures={} generation={} detail={}",
                provider.id,
                provider.status_at(now).as_str(),
                provider.lane,
                attempt_age,
                age,
                provider.duration_ms,
                provider.consecutive_failures,
                provider.generation,
                provider.detail,
            ));
        }
        lines.join("\n")
    }
}

#[derive(Clone, Copy, Debug)]
pub enum QuickAction {
    Lock,
    Screenshot,
    Shutdown,
}

#[derive(Debug)]
enum SlowRequest {
    CheckUpdates,
}

#[derive(Debug)]
enum ActionRequest {
    SetVolume(u32),
    ToggleMute,
    SetBrightness(u32),
    QuickAction(u64, QuickAction),
    OpenPath(PathBuf),
}

#[derive(Clone)]
pub struct RuntimeServices {
    state: Arc<RwLock<ExternalSnapshot>>,
    slow_sender: mpsc::Sender<SlowRequest>,
    action_sender: mpsc::Sender<ActionRequest>,
    request_counter: Arc<AtomicU64>,
}

impl RuntimeServices {
    pub fn start(check_updates_on_start: bool) -> Self {
        let state = Arc::new(RwLock::new(ExternalSnapshot::default()));
        let fast_state = state.clone();
        thread::Builder::new()
            .name("topmonitoring-fast-providers".into())
            .spawn(move || fast_provider_loop(fast_state))
            .expect("failed to start TopMonitoring fast-provider worker");

        let (slow_sender, slow_receiver) = mpsc::channel();
        let slow_state = state.clone();
        thread::Builder::new()
            .name("topmonitoring-slow-providers".into())
            .spawn(move || slow_provider_loop(slow_state, slow_receiver, check_updates_on_start))
            .expect("failed to start TopMonitoring slow-provider worker");

        let (action_sender, action_receiver) = mpsc::channel();
        let action_state = state.clone();
        thread::Builder::new()
            .name("topmonitoring-actions".into())
            .spawn(move || action_worker_loop(action_state, action_receiver))
            .expect("failed to start TopMonitoring action worker");
        Self {
            state,
            slow_sender,
            action_sender,
            request_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn snapshot(&self) -> ExternalSnapshot {
        self.state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    pub fn set_volume(&self, value: u32) {
        let _ = self
            .action_sender
            .send(ActionRequest::SetVolume(value.min(100)));
    }

    pub fn toggle_mute(&self) {
        let _ = self.action_sender.send(ActionRequest::ToggleMute);
    }

    pub fn set_brightness(&self, value: u32) {
        let _ = self
            .action_sender
            .send(ActionRequest::SetBrightness(value.min(100)));
    }

    pub fn check_updates(&self) {
        let _ = self.slow_sender.send(SlowRequest::CheckUpdates);
    }

    pub fn quick_action(&self, action: QuickAction) -> u64 {
        let request_id = self.request_counter.fetch_add(1, Ordering::Relaxed);
        publish(&self.state, |snapshot| {
            snapshot.action_request_id = request_id;
            snapshot.action_pending = true;
            snapshot.action_success = None;
            snapshot.action_duration_ms = 0;
            snapshot.last_action_provider = None;
            snapshot.last_action_status = Some(format!("{} queued", quick_action_name(action)));
        });
        if self
            .action_sender
            .send(ActionRequest::QuickAction(request_id, action))
            .is_err()
        {
            publish(&self.state, |snapshot| {
                if snapshot.action_request_id == request_id {
                    snapshot.action_pending = false;
                    snapshot.action_success = Some(false);
                    snapshot.last_action_status = Some("Action worker is unavailable".into());
                }
                push_action_result(
                    snapshot,
                    ActionFeedback {
                        request_id,
                        success: false,
                        provider: "action worker".into(),
                        message: "Action worker is unavailable".into(),
                        duration_ms: 0,
                    },
                );
            });
        }
        request_id
    }

    pub fn open_path(&self, path: PathBuf) {
        let _ = self.action_sender.send(ActionRequest::OpenPath(path));
    }
}

fn fast_provider_loop(state: Arc<RwLock<ExternalSnapshot>>) {
    let mut next_audio = Instant::now();
    let mut next_media = Instant::now();
    let mut next_connectivity = Instant::now();
    loop {
        let now = Instant::now();
        if now >= next_audio {
            publish(&state, |snapshot| {
                update_provider_health(
                    snapshot,
                    "audio",
                    ProviderStatus::InFlight,
                    0,
                    8,
                    "collecting",
                );
            });
            let started = Instant::now();
            let volume = read_volume();
            let duration_ms = started.elapsed().as_millis();
            let status = if volume.is_some() {
                ProviderStatus::Available
            } else {
                ProviderStatus::Unavailable
            };
            publish(&state, |snapshot| {
                snapshot.volume = volume;
                update_provider_health(
                    snapshot,
                    "audio",
                    status,
                    duration_ms,
                    8,
                    if status == ProviderStatus::Available {
                        "ready"
                    } else {
                        "backend unavailable"
                    },
                );
            });

            publish(&state, |snapshot| {
                update_provider_health(
                    snapshot,
                    "brightness",
                    ProviderStatus::InFlight,
                    0,
                    8,
                    "collecting",
                );
            });
            let started = Instant::now();
            let brightness = read_brightness();
            let duration_ms = started.elapsed().as_millis();
            let status = if brightness.is_some() {
                ProviderStatus::Available
            } else {
                ProviderStatus::Unavailable
            };
            publish(&state, |snapshot| {
                snapshot.brightness = brightness;
                update_provider_health(
                    snapshot,
                    "brightness",
                    status,
                    duration_ms,
                    8,
                    if status == ProviderStatus::Available {
                        "ready"
                    } else {
                        "backend unavailable"
                    },
                );
            });
            next_audio = Instant::now() + Duration::from_secs(2);
        }
        if now >= next_media {
            publish(&state, |snapshot| {
                update_provider_health(
                    snapshot,
                    "media",
                    ProviderStatus::InFlight,
                    0,
                    20,
                    "collecting",
                );
            });
            let started = Instant::now();
            let media = read_media();
            let duration_ms = started.elapsed().as_millis();
            let status = if media.is_some() {
                ProviderStatus::Available
            } else {
                ProviderStatus::Unavailable
            };
            publish(&state, |snapshot| {
                snapshot.media = media;
                update_provider_health(
                    snapshot,
                    "media",
                    status,
                    duration_ms,
                    20,
                    if status == ProviderStatus::Available {
                        "player active"
                    } else {
                        "idle or unavailable"
                    },
                );
            });
            next_media = Instant::now() + Duration::from_secs(5);
        }
        if now >= next_connectivity {
            publish(&state, |snapshot| {
                update_provider_health(
                    snapshot,
                    "wifi",
                    ProviderStatus::InFlight,
                    0,
                    20,
                    "collecting",
                );
            });
            let started = Instant::now();
            let wifi = read_wifi();
            let duration_ms = started.elapsed().as_millis();
            let status = if wifi.is_some() {
                ProviderStatus::Available
            } else {
                ProviderStatus::Unavailable
            };
            publish(&state, |snapshot| {
                snapshot.wifi = wifi;
                update_provider_health(
                    snapshot,
                    "wifi",
                    status,
                    duration_ms,
                    20,
                    if status == ProviderStatus::Available {
                        "connected"
                    } else {
                        "not connected or unavailable"
                    },
                );
            });

            publish(&state, |snapshot| {
                update_provider_health(
                    snapshot,
                    "bluetooth",
                    ProviderStatus::InFlight,
                    0,
                    20,
                    "collecting",
                );
            });
            let started = Instant::now();
            let bluetooth = read_bluetooth();
            let duration_ms = started.elapsed().as_millis();
            let status = if bluetooth.is_some() {
                ProviderStatus::Available
            } else {
                ProviderStatus::Unavailable
            };
            publish(&state, |snapshot| {
                snapshot.bluetooth = bluetooth;
                update_provider_health(
                    snapshot,
                    "bluetooth",
                    status,
                    duration_ms,
                    20,
                    if status == ProviderStatus::Available {
                        "adapter detected"
                    } else {
                        "backend unavailable"
                    },
                );
            });
            next_connectivity = Instant::now() + Duration::from_secs(5);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn slow_provider_loop(
    state: Arc<RwLock<ExternalSnapshot>>,
    receiver: mpsc::Receiver<SlowRequest>,
    check_updates_on_start: bool,
) {
    let mut next_packages = Instant::now();
    let mut update_pending = check_updates_on_start;
    loop {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(SlowRequest::CheckUpdates) => update_pending = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(SlowRequest::CheckUpdates) = receiver.try_recv() {
            update_pending = true;
        }

        if update_pending {
            update_pending = false;
            publish(&state, |snapshot| {
                update_provider_health(
                    snapshot,
                    "release",
                    ProviderStatus::InFlight,
                    0,
                    86_400,
                    "collecting",
                );
            });
            let started = Instant::now();
            let update_status = check_for_update();
            let duration_ms = started.elapsed().as_millis();
            let provider_status =
                if update_status.contains("unavailable") || update_status.contains("invalid") {
                    ProviderStatus::Error
                } else {
                    ProviderStatus::Available
                };
            publish(&state, |snapshot| {
                snapshot.update_status = Some(update_status);
                update_provider_health(
                    snapshot,
                    "release",
                    provider_status,
                    duration_ms,
                    86_400,
                    if provider_status == ProviderStatus::Available {
                        "completed"
                    } else {
                        "request failed"
                    },
                );
            });
        }

        let now = Instant::now();
        if now >= next_packages {
            publish(&state, |snapshot| {
                update_provider_health(
                    snapshot,
                    "packages",
                    ProviderStatus::InFlight,
                    0,
                    30 * 60,
                    "collecting",
                );
            });
            let started = Instant::now();
            let packages = read_package_updates();
            let duration_ms = started.elapsed().as_millis();
            let backend_count = packages
                .as_ref()
                .map(|state| state.available_backends)
                .unwrap_or(0);
            let provider_status = if packages.is_some() {
                ProviderStatus::Available
            } else {
                ProviderStatus::Unavailable
            };
            let detail = if backend_count > 0 {
                format!("{backend_count} backends")
            } else {
                "no supported backend".into()
            };
            publish(&state, |snapshot| {
                snapshot.packages = packages;
                update_provider_health(
                    snapshot,
                    "packages",
                    provider_status,
                    duration_ms,
                    30 * 60,
                    &detail,
                );
            });
            next_packages = Instant::now() + Duration::from_secs(20 * 60);
        }
    }
}

fn action_worker_loop(
    state: Arc<RwLock<ExternalSnapshot>>,
    receiver: mpsc::Receiver<ActionRequest>,
) {
    while let Ok(request) = receiver.recv() {
        handle_action_request(&state, request);
    }
}

fn handle_action_request(state: &Arc<RwLock<ExternalSnapshot>>, request: ActionRequest) {
    match request {
        ActionRequest::SetVolume(value) => {
            let started = Instant::now();
            let action_status = set_volume(value);
            let volume = read_volume();
            let duration_ms = started.elapsed().as_millis();
            let provider_status = if !action_status.starts_with("Could not") && volume.is_some() {
                ProviderStatus::Available
            } else {
                ProviderStatus::Error
            };
            publish(state, |snapshot| {
                snapshot.volume = volume;
                update_provider_health(
                    snapshot,
                    "audio",
                    provider_status,
                    duration_ms,
                    8,
                    if provider_status == ProviderStatus::Available {
                        "action refresh"
                    } else {
                        "action failed"
                    },
                );
            });
        }
        ActionRequest::ToggleMute => {
            let started = Instant::now();
            let action_status = toggle_mute();
            let volume = read_volume();
            let duration_ms = started.elapsed().as_millis();
            let provider_status = if !action_status.starts_with("Could not") && volume.is_some() {
                ProviderStatus::Available
            } else {
                ProviderStatus::Error
            };
            publish(state, |snapshot| {
                snapshot.volume = volume;
                update_provider_health(
                    snapshot,
                    "audio",
                    provider_status,
                    duration_ms,
                    8,
                    if provider_status == ProviderStatus::Available {
                        "action refresh"
                    } else {
                        "action failed"
                    },
                );
            });
        }
        ActionRequest::SetBrightness(value) => {
            let started = Instant::now();
            let action_status = set_brightness(value);
            let brightness = read_brightness();
            let duration_ms = started.elapsed().as_millis();
            let provider_status = if !action_status.starts_with("Could not") && brightness.is_some()
            {
                ProviderStatus::Available
            } else {
                ProviderStatus::Error
            };
            publish(state, |snapshot| {
                snapshot.brightness = brightness;
                update_provider_health(
                    snapshot,
                    "brightness",
                    provider_status,
                    duration_ms,
                    8,
                    if provider_status == ProviderStatus::Available {
                        "action refresh"
                    } else {
                        "action failed"
                    },
                );
            });
        }
        ActionRequest::QuickAction(request_id, action) => {
            let started = Instant::now();
            let outcome = run_quick_action(action);
            publish(state, |snapshot| {
                let feedback = ActionFeedback {
                    request_id,
                    success: outcome.success,
                    provider: outcome.provider,
                    message: outcome.message,
                    duration_ms: started.elapsed().as_millis(),
                };
                if snapshot.action_request_id == request_id {
                    snapshot.action_pending = false;
                    snapshot.action_success = Some(feedback.success);
                    snapshot.action_duration_ms = feedback.duration_ms;
                    snapshot.last_action_provider = Some(feedback.provider.clone());
                    snapshot.last_action_status = Some(feedback.message.clone());
                }
                push_action_result(snapshot, feedback);
            });
        }
        ActionRequest::OpenPath(path) => {
            let path = path.to_string_lossy().to_string();
            let _ = spawn_reaped("xdg-open", &[&path]);
        }
    }
}

fn publish(state: &Arc<RwLock<ExternalSnapshot>>, update: impl FnOnce(&mut ExternalSnapshot)) {
    if let Ok(mut snapshot) = state.write() {
        update(&mut snapshot);
        snapshot.generation = snapshot.generation.wrapping_add(1);
    }
}

fn push_action_result(snapshot: &mut ExternalSnapshot, feedback: ActionFeedback) {
    const MAX_ACTION_RESULTS: usize = 16;
    if snapshot.action_results.len() >= MAX_ACTION_RESULTS {
        snapshot.action_results.remove(0);
    }
    snapshot.action_results.push(feedback);
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn provider_lane(id: &str) -> &'static str {
    match id {
        "audio" | "brightness" | "media" | "wifi" | "bluetooth" => "fast",
        "packages" | "release" => "slow",
        _ => "unknown",
    }
}

fn update_provider_health(
    snapshot: &mut ExternalSnapshot,
    id: &str,
    status: ProviderStatus,
    duration_ms: u128,
    stale_after_secs: u64,
    detail: &str,
) {
    let now = epoch_seconds();
    let next_generation = snapshot.generation.wrapping_add(1);
    let index = snapshot
        .provider_health
        .iter()
        .position(|provider| provider.id == id)
        .unwrap_or_else(|| {
            snapshot.provider_health.push(ProviderHealth {
                id: id.into(),
                ..ProviderHealth::default()
            });
            snapshot.provider_health.len() - 1
        });
    let provider = &mut snapshot.provider_health[index];
    provider.lane = provider_lane(id).into();
    provider.status = status;
    provider.last_attempt_epoch_secs = now;
    provider.duration_ms = duration_ms;
    provider.stale_after_secs = stale_after_secs.max(1);
    provider.generation = next_generation;
    provider.detail = detail
        .chars()
        .filter(|character| !character.is_control())
        .take(96)
        .collect();
    if status == ProviderStatus::Available {
        provider.last_success_epoch_secs = Some(now);
        provider.consecutive_failures = 0;
    } else if status != ProviderStatus::InFlight {
        provider.consecutive_failures = provider.consecutive_failures.saturating_add(1);
    }
}

fn short_policy() -> CommandPolicy {
    CommandPolicy {
        timeout: Duration::from_millis(900),
        max_output_bytes: 8 * 1024,
    }
}

fn medium_policy() -> CommandPolicy {
    CommandPolicy {
        timeout: Duration::from_secs(6),
        max_output_bytes: 32 * 1024,
    }
}

fn read_volume() -> Option<(u32, bool)> {
    let wpctl = run_program_limited(
        "wpctl",
        &["get-volume", "@DEFAULT_AUDIO_SINK@"],
        short_policy(),
    );
    if wpctl.success {
        let rest = wpctl.stdout.trim().strip_prefix("Volume:")?;
        let muted = rest.contains("MUTED");
        let value = rest.split_whitespace().next()?.parse::<f64>().ok()?;
        return Some((((value * 100.0).round() as u32).min(150), muted));
    }

    let volume = run_program_limited(
        "pactl",
        &["get-sink-volume", "@DEFAULT_SINK@"],
        short_policy(),
    );
    if !volume.success {
        return None;
    }
    let percent_position = volume.stdout.find('%')?;
    let start = volume.stdout[..percent_position]
        .rfind(' ')
        .map(|index| index + 1)
        .unwrap_or(0);
    let value = volume.stdout[start..percent_position].parse::<u32>().ok()?;
    let mute = run_program_limited(
        "pactl",
        &["get-sink-mute", "@DEFAULT_SINK@"],
        short_policy(),
    );
    Some((value.min(150), mute.success && mute.stdout.contains("yes")))
}

fn set_volume(value: u32) -> String {
    let percentage = format!("{}%", value.min(100));
    let wpctl = run_program_limited(
        "wpctl",
        &["set-volume", "@DEFAULT_AUDIO_SINK@", &percentage],
        short_policy(),
    );
    if wpctl.success {
        return format!("Volume set to {percentage}");
    }
    let pactl = run_program_limited(
        "pactl",
        &["set-sink-volume", "@DEFAULT_SINK@", &percentage],
        short_policy(),
    );
    if pactl.success {
        format!("Volume set to {percentage}")
    } else {
        "Could not set volume".into()
    }
}

fn toggle_mute() -> String {
    let wpctl = run_program_limited(
        "wpctl",
        &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
        short_policy(),
    );
    if wpctl.success {
        return "Mute toggled".into();
    }
    let pactl = run_program_limited(
        "pactl",
        &["set-sink-mute", "@DEFAULT_SINK@", "toggle"],
        short_policy(),
    );
    if pactl.success {
        "Mute toggled".into()
    } else {
        "Could not toggle mute".into()
    }
}

fn read_brightness() -> Option<u32> {
    let current = run_program_limited("brightnessctl", &["get"], short_policy());
    let maximum = run_program_limited("brightnessctl", &["max"], short_policy());
    if current.success && maximum.success {
        let current = current.stdout.trim().parse::<f64>().ok()?;
        let maximum = maximum.stdout.trim().parse::<f64>().ok()?;
        if maximum > 0.0 {
            return Some(((current / maximum) * 100.0).round() as u32);
        }
    }
    let device = fs::read_dir("/sys/class/backlight")
        .ok()?
        .flatten()
        .next()?;
    let current = fs::read_to_string(device.path().join("brightness"))
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()?;
    let maximum = fs::read_to_string(device.path().join("max_brightness"))
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()?;
    (maximum > 0.0).then_some(((current / maximum) * 100.0).round() as u32)
}

fn set_brightness(value: u32) -> String {
    let percentage = format!("{}%", value.min(100));
    let result = run_program_limited("brightnessctl", &["set", &percentage], short_policy());
    if result.success {
        return format!("Brightness set to {percentage}");
    }

    let Some(device) = fs::read_dir("/sys/class/backlight")
        .ok()
        .and_then(|mut entries| entries.find_map(Result::ok))
    else {
        return "Could not find a backlight device".into();
    };
    let base = device.path();
    let Some(maximum) = fs::read_to_string(base.join("max_brightness"))
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
    else {
        return "Could not read backlight maximum".into();
    };
    let raw = ((value.min(100) as f64 / 100.0) * maximum).round() as u64;
    match fs::write(base.join("brightness"), raw.to_string()) {
        Ok(_) => format!("Brightness set to {percentage}"),
        Err(error) => format!("Could not set brightness: {error}"),
    }
}

fn read_media() -> Option<MediaState> {
    let status = run_program_limited("playerctl", &["status"], short_policy());
    if !status.success || status.stdout.trim().is_empty() {
        return None;
    }
    let metadata = run_program_limited(
        "playerctl",
        &["metadata", "--format", "{{artist}}\t{{title}}"],
        short_policy(),
    );
    if !metadata.success {
        return None;
    }
    let mut values = metadata.stdout.splitn(2, '\t');
    let artist = values.next().unwrap_or_default().trim().to_string();
    let title = values.next().unwrap_or_default().trim().to_string();
    if artist.is_empty() && title.is_empty() {
        None
    } else {
        Some(MediaState {
            status: status.stdout.trim().to_string(),
            artist,
            title,
        })
    }
}

fn read_wifi() -> Option<(String, i32)> {
    let content = fs::read_to_string("/proc/net/wireless").ok()?;
    for line in content.lines().skip(2) {
        let (interface, values) = line.trim().split_once(':')?;
        let columns: Vec<&str> = values.split_whitespace().collect();
        if columns.len() < 3 {
            continue;
        }
        let level = columns[2].trim_end_matches('.').parse::<f64>().ok()? as i32;
        let ssid = read_wifi_ssid().unwrap_or_else(|| interface.trim().to_string());
        return Some((ssid, level));
    }
    None
}

fn read_wifi_ssid() -> Option<String> {
    let iwgetid = run_program_limited("iwgetid", &["-r"], short_policy());
    if iwgetid.success && !iwgetid.stdout.trim().is_empty() {
        return Some(iwgetid.stdout.trim().to_string());
    }
    let nmcli = run_program_limited(
        "nmcli",
        &["-t", "-f", "active,ssid", "dev", "wifi"],
        short_policy(),
    );
    if !nmcli.success {
        return None;
    }
    nmcli.stdout.lines().find_map(|line| {
        line.strip_prefix("yes:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn read_bluetooth() -> Option<(bool, Vec<String>)> {
    let show = run_program_limited("bluetoothctl", &["show"], short_policy());
    if !show.success || show.stdout.trim().is_empty() {
        return None;
    }
    let powered = show
        .stdout
        .lines()
        .any(|line| line.trim() == "Powered: yes");
    let connected = run_program_limited("bluetoothctl", &["devices", "Connected"], short_policy());
    let devices = if connected.success {
        connected
            .stdout
            .lines()
            .filter_map(|line| line.splitn(3, ' ').nth(2))
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };
    Some((powered, devices))
}

fn read_package_updates() -> Option<PackageState> {
    let mut state = PackageState::default();
    let apt = run_program_limited("apt", &["list", "--upgradable"], medium_policy());
    if apt.success {
        state.available_backends += 1;
        state.apt = apt
            .stdout
            .lines()
            .filter(|line| line.contains('/') && !line.starts_with("Listing"))
            .count() as u32;
        state.backend_details.push(format!("APT: {}", state.apt));
    }

    let dnf = run_program_limited(
        "dnf",
        &["check-update", "--refresh", "--quiet"],
        medium_policy(),
    );
    if dnf.success || dnf.exit_code == Some(100) {
        state.available_backends += 1;
        state.dnf = dnf
            .stdout
            .lines()
            .filter(|line| {
                let line = line.trim();
                !line.is_empty()
                    && !line.starts_with("Last metadata expiration check")
                    && line.split_whitespace().count() >= 3
            })
            .count() as u32;
        state.backend_details.push(format!("DNF: {}", state.dnf));
    }

    let pacman = run_program_limited("checkupdates", &[], medium_policy());
    if pacman.success || pacman.exit_code == Some(2) || !pacman.stdout.trim().is_empty() {
        state.available_backends += 1;
        state.pacman = pacman
            .stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count() as u32;
        state
            .backend_details
            .push(format!("Pacman: {}", state.pacman));
    }

    let flatpak = run_program_limited(
        "flatpak",
        &["remote-ls", "--updates", "--columns=application"],
        medium_policy(),
    );
    if flatpak.success {
        state.available_backends += 1;
        state.flatpak = flatpak
            .stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count() as u32;
        state
            .backend_details
            .push(format!("Flatpak: {}", state.flatpak));
    }
    (state.available_backends > 0).then_some(state)
}

fn check_for_update() -> String {
    let response = run_program_limited(
        "curl",
        &[
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--max-time",
            "8",
            "--user-agent",
            "TopMonitoring/2",
            RELEASE_API,
        ],
        CommandPolicy {
            timeout: Duration::from_secs(10),
            max_output_bytes: 64 * 1024,
        },
    );
    if !response.success {
        return "Update check unavailable".into();
    }
    let Some(tag) = extract_json_string(&response.stdout, "tag_name") else {
        return "Release response was invalid".into();
    };
    let latest = tag.trim_start_matches('v');
    let current = env!("CARGO_PKG_VERSION");
    match compare_versions(latest, current) {
        std::cmp::Ordering::Greater => format!("Update available: v{latest}"),
        std::cmp::Ordering::Equal => format!("Up to date: v{current}"),
        std::cmp::Ordering::Less => format!("Development build: v{current}"),
    }
}

fn extract_json_string(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\"");
    let after_key = input.split_once(&marker)?.1;
    let after_colon = after_key.split_once(':')?.1.trim_start();
    let mut characters = after_colon.chars();
    if characters.next()? != '"' {
        return None;
    }
    let mut output = String::new();
    let mut escaped = false;
    for character in characters {
        if escaped {
            output.push(match character {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Some(output);
        } else {
            output.push(character);
        }
    }
    None
}

pub fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let parse = |value: &str| {
        value
            .trim_start_matches(|character: char| !character.is_ascii_digit())
            .split(['.', '-', '+'])
            .take(4)
            .map(|part| {
                part.chars()
                    .take_while(|character| character.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect::<Vec<_>>()
    };
    let mut left = parse(left);
    let mut right = parse(right);
    let length = left.len().max(right.len()).max(3);
    left.resize(length, 0);
    right.resize(length, 0);
    left.cmp(&right)
}

#[derive(Debug)]
struct ActionOutcome {
    success: bool,
    provider: String,
    message: String,
}

fn quick_action_name(action: QuickAction) -> &'static str {
    match action {
        QuickAction::Lock => "Lock",
        QuickAction::Screenshot => "Screenshot",
        QuickAction::Shutdown => "Power off",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LockCandidate {
    provider: &'static str,
    program: &'static str,
    arguments: Vec<String>,
}

fn lock_candidate(
    provider: &'static str,
    program: &'static str,
    arguments: &[&str],
) -> LockCandidate {
    LockCandidate {
        provider,
        program,
        arguments: arguments.iter().map(|value| (*value).to_string()).collect(),
    }
}

fn lock_candidates(desktop: &str, session_id: Option<&str>) -> Vec<LockCandidate> {
    let desktop = desktop.to_ascii_lowercase();
    let mut candidates = Vec::new();
    if desktop.contains("cinnamon") {
        candidates.push(lock_candidate(
            "Cinnamon",
            "cinnamon-screensaver-command",
            &["-l"],
        ));
    }
    if desktop.contains("gnome") || desktop.contains("unity") || desktop.contains("budgie") {
        candidates.push(lock_candidate(
            "GNOME D-Bus",
            "gdbus",
            &[
                "call",
                "--session",
                "--dest",
                "org.gnome.ScreenSaver",
                "--object-path",
                "/org/gnome/ScreenSaver",
                "--method",
                "org.gnome.ScreenSaver.Lock",
            ],
        ));
    }
    if desktop.contains("kde") || desktop.contains("plasma") {
        candidates.push(lock_candidate(
            "KDE D-Bus",
            "qdbus6",
            &["org.freedesktop.ScreenSaver", "/ScreenSaver", "Lock"],
        ));
        candidates.push(lock_candidate(
            "KDE D-Bus",
            "qdbus",
            &["org.freedesktop.ScreenSaver", "/ScreenSaver", "Lock"],
        ));
    }
    if desktop.contains("xfce") {
        candidates.push(lock_candidate("Xfce", "xflock4", &[]));
    }
    if desktop.contains("mate") {
        candidates.push(lock_candidate("MATE", "mate-screensaver-command", &["-l"]));
    }
    candidates.push(lock_candidate(
        "freedesktop D-Bus",
        "gdbus",
        &[
            "call",
            "--session",
            "--dest",
            "org.freedesktop.ScreenSaver",
            "--object-path",
            "/ScreenSaver",
            "--method",
            "org.freedesktop.ScreenSaver.Lock",
        ],
    ));
    candidates.push(lock_candidate("desktop manager", "dm-tool", &["lock"]));
    candidates.push(lock_candidate(
        "XDG screensaver",
        "xdg-screensaver",
        &["lock"],
    ));
    let mut loginctl = vec!["lock-session".into()];
    if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
        loginctl.push(session_id.into());
    }
    candidates.push(LockCandidate {
        provider: "systemd-logind (unverified)",
        program: "loginctl",
        arguments: loginctl,
    });
    candidates
}

fn run_lock_action() -> ActionOutcome {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let session_id = std::env::var("XDG_SESSION_ID").ok();
    let mut attempted = Vec::new();
    for candidate in lock_candidates(&desktop, session_id.as_deref()) {
        let arguments: Vec<&str> = candidate.arguments.iter().map(String::as_str).collect();
        let result = run_program_limited(
            candidate.program,
            &arguments,
            CommandPolicy {
                timeout: Duration::from_secs(3),
                max_output_bytes: 8 * 1024,
            },
        );
        attempted.push(candidate.provider);
        if result.success {
            return ActionOutcome {
                success: true,
                provider: candidate.provider.into(),
                message: format!("Lock requested through {}", candidate.provider),
            };
        }
    }
    ActionOutcome {
        success: false,
        provider: "lock resolver".into(),
        message: format!(
            "No working screen locker found (tried {})",
            attempted.join(", ")
        ),
    }
}

fn run_quick_action(action: QuickAction) -> ActionOutcome {
    match action {
        QuickAction::Lock => run_lock_action(),
        QuickAction::Screenshot => {
            let candidates: &[(&str, &[&str])] = &[
                ("gnome-screenshot", &["-i"]),
                ("xfce4-screenshooter", &[]),
                ("spectacle", &["-r"]),
                ("flameshot", &["gui"]),
            ];
            for (program, arguments) in candidates {
                if spawn_reaped(program, arguments) {
                    return ActionOutcome {
                        success: true,
                        provider: (*program).into(),
                        message: format!("Started {program}"),
                    };
                }
            }
            ActionOutcome {
                success: false,
                provider: "screenshot resolver".into(),
                message: "No supported screenshot tool found".into(),
            }
        }
        QuickAction::Shutdown => {
            let result = run_program_limited(
                "systemctl",
                &["poweroff"],
                CommandPolicy {
                    timeout: Duration::from_secs(12),
                    max_output_bytes: 4 * 1024,
                },
            );
            if result.success {
                ActionOutcome {
                    success: true,
                    provider: "systemd-logind".into(),
                    message: "Shutdown requested".into(),
                }
            } else {
                ActionOutcome {
                    success: false,
                    provider: "systemd-logind".into(),
                    message: "Shutdown request failed".into(),
                }
            }
        }
    }
}

/// Spawns an interactive desktop tool and reaps it on a dedicated thread.
fn spawn_reaped(program: &str, arguments: &[&str]) -> bool {
    match Command::new(program).args(arguments).spawn() {
        Ok(mut child) => {
            thread::spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

pub fn release_page() -> &'static str {
    RELEASE_PAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_compared_numerically() {
        assert_eq!(
            compare_versions("2.10.0", "2.9.9"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(compare_versions("v2.0.0", "2.0"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn extracts_json_string() {
        assert_eq!(
            extract_json_string(r#"{"tag_name":"v2.1.0"}"#, "tag_name"),
            Some("v2.1.0".into())
        );
    }

    #[test]
    fn package_total_is_saturating() {
        let state = PackageState {
            apt: 3,
            flatpak: 2,
            available_backends: 2,
            ..PackageState::default()
        };
        assert_eq!(state.total(), 5);
    }

    #[test]
    fn cinnamon_lock_backend_precedes_generic_fallbacks() {
        let candidates = lock_candidates("X-Cinnamon", Some("c2"));
        assert_eq!(candidates[0].program, "cinnamon-screensaver-command");
        let loginctl = candidates
            .iter()
            .find(|candidate| candidate.program == "loginctl")
            .expect("loginctl fallback");
        assert_eq!(
            loginctl.arguments,
            vec!["lock-session".to_string(), "c2".to_string()]
        );
    }

    #[test]
    fn action_result_history_is_bounded() {
        let mut snapshot = ExternalSnapshot::default();
        for request_id in 0..20 {
            push_action_result(
                &mut snapshot,
                ActionFeedback {
                    request_id,
                    ..ActionFeedback::default()
                },
            );
        }
        assert_eq!(snapshot.action_results.len(), 16);
        assert_eq!(snapshot.action_results[0].request_id, 4);
        assert_eq!(snapshot.action_results[15].request_id, 19);
    }

    #[test]
    fn provider_health_becomes_stale_after_deadline() {
        let health = ProviderHealth {
            id: "audio".into(),
            status: ProviderStatus::Available,
            last_success_epoch_secs: Some(100),
            stale_after_secs: 10,
            ..ProviderHealth::default()
        };
        assert_eq!(health.status_at(110), ProviderStatus::Available);
        assert_eq!(health.status_at(111), ProviderStatus::Stale);
    }

    #[test]
    fn provider_health_preserves_success_across_failure() {
        let mut snapshot = ExternalSnapshot::default();
        update_provider_health(
            &mut snapshot,
            "packages",
            ProviderStatus::Available,
            12,
            60,
            "ready",
        );
        let last_success = snapshot.provider_health[0].last_success_epoch_secs;
        update_provider_health(
            &mut snapshot,
            "packages",
            ProviderStatus::InFlight,
            0,
            60,
            "collecting",
        );
        assert_eq!(snapshot.provider_health[0].consecutive_failures, 0);
        update_provider_health(
            &mut snapshot,
            "packages",
            ProviderStatus::Error,
            900,
            60,
            "request failed",
        );
        assert_eq!(
            snapshot.provider_health[0].last_success_epoch_secs,
            last_success
        );
        assert_eq!(snapshot.provider_health[0].consecutive_failures, 1);
        assert_eq!(snapshot.provider_health.len(), 1);
    }

    #[test]
    fn slow_jobs_are_classified_outside_fast_provider_lane() {
        assert_eq!(provider_lane("audio"), "fast");
        assert_eq!(provider_lane("media"), "fast");
        assert_eq!(provider_lane("packages"), "slow");
        assert_eq!(provider_lane("release"), "slow");
    }

    #[test]
    fn diagnostics_do_not_include_sensitive_provider_values() {
        let mut snapshot = ExternalSnapshot {
            wifi: Some(("Secret-Network".into(), -42)),
            media: Some(MediaState {
                title: "Private Song".into(),
                artist: "Private Artist".into(),
                status: "Playing".into(),
            }),
            ..ExternalSnapshot::default()
        };
        update_provider_health(
            &mut snapshot,
            "wifi",
            ProviderStatus::Available,
            4,
            20,
            "connected",
        );
        let report = snapshot.sanitized_diagnostics();
        assert!(report.contains("wifi=available"));
        assert!(report.contains("lane=fast"));
        assert!(!report.contains("Secret-Network"));
        assert!(!report.contains("Private Song"));
        assert!(!report.contains("Private Artist"));
    }
}

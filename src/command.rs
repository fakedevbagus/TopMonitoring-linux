use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const MAX_CUSTOM_COMMANDS: usize = 4;
static ACTIVE_CUSTOM_COMMANDS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug)]
pub struct CommandPolicy {
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

impl Default for CommandPolicy {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(2),
            max_output_bytes: 16 * 1024,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub truncated: bool,
    pub duration: Duration,
}

impl CommandResult {
    pub fn display_text(&self) -> String {
        let output = self.stdout.trim();
        if self.success && !output.is_empty() {
            return output.replace('\0', "");
        }
        if self.timed_out {
            return "timeout".into();
        }
        let error = self.stderr.trim();
        if error.is_empty() {
            "error".into()
        } else {
            error.lines().next().unwrap_or("error").to_string()
        }
    }
}

struct CustomCommandGuard;

impl CustomCommandGuard {
    fn acquire() -> Option<Self> {
        ACTIVE_CUSTOM_COMMANDS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_CUSTOM_COMMANDS).then_some(active + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

impl Drop for CustomCommandGuard {
    fn drop(&mut self) {
        ACTIVE_CUSTOM_COMMANDS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn run_shell_limited(command_line: &str, policy: CommandPolicy) -> CommandResult {
    let Some(_guard) = CustomCommandGuard::acquire() else {
        return CommandResult {
            stderr: format!("busy: maximum {MAX_CUSTOM_COMMANDS} custom commands are running"),
            ..CommandResult::default()
        };
    };
    if command_line.trim().is_empty() {
        return CommandResult {
            stderr: "empty command".into(),
            ..CommandResult::default()
        };
    }
    let mut command = Command::new("sh");
    command.arg("-c").arg(command_line);
    execute(command, policy)
}

pub fn run_program_limited(program: &str, args: &[&str], policy: CommandPolicy) -> CommandResult {
    let mut command = Command::new(program);
    command.args(args);
    execute(command, policy)
}

fn execute(mut command: Command, policy: CommandPolicy) -> CommandResult {
    let started = Instant::now();
    let cap = policy.max_output_bytes.clamp(256, 65_536);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return CommandResult {
                stderr: error.to_string(),
                duration: started.elapsed(),
                ..CommandResult::default()
            }
        }
    };

    let stdout_reader = child
        .stdout
        .take()
        .map(|stdout| spawn_capped_reader(stdout, cap));
    let stderr_reader = child
        .stderr
        .take()
        .map(|stderr| spawn_capped_reader(stderr, cap));

    let deadline = started + policy.timeout.max(Duration::from_millis(100));
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), false),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                terminate_process_group(child.id());
                let _ = child.kill();
                break (child.wait().ok(), true);
            }
            Err(error) => {
                terminate_process_group(child.id());
                let _ = child.kill();
                let mut result = collect_result(stdout_reader, stderr_reader, None, false, started);
                result.stderr = error.to_string();
                return result;
            }
        }
    };

    collect_result(stdout_reader, stderr_reader, status, timed_out, started)
}

fn spawn_capped_reader<R>(mut reader: R, cap: usize) -> thread::JoinHandle<(Vec<u8>, bool)>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut stored = Vec::with_capacity(cap.min(4_096));
        let mut buffer = [0_u8; 4_096];
        let mut truncated = false;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    let remaining = cap.saturating_sub(stored.len());
                    let keep = count.min(remaining);
                    stored.extend_from_slice(&buffer[..keep]);
                    if keep < count {
                        truncated = true;
                    }
                }
                Err(_) => break,
            }
        }
        (stored, truncated)
    })
}

fn collect_result(
    stdout_reader: Option<thread::JoinHandle<(Vec<u8>, bool)>>,
    stderr_reader: Option<thread::JoinHandle<(Vec<u8>, bool)>>,
    status: Option<ExitStatus>,
    timed_out: bool,
    started: Instant,
) -> CommandResult {
    let (stdout, stdout_truncated) = stdout_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let (stderr, stderr_truncated) = stderr_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    CommandResult {
        stdout: String::from_utf8_lossy(&stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
        success: status
            .as_ref()
            .map(|status| status.success())
            .unwrap_or(false)
            && !timed_out,
        exit_code: status.as_ref().and_then(|status| status.code()),
        timed_out,
        truncated: stdout_truncated || stderr_truncated,
        duration: started.elapsed(),
    }
}

#[cfg(unix)]
fn terminate_process_group(pid: u32) {
    let group = format!("-{pid}");
    let _ = Command::new("kill").args(["-TERM", "--", &group]).status();
    thread::sleep(Duration::from_millis(40));
    let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
}

#[cfg(not(unix))]
fn terminate_process_group(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_successful_output() {
        let result = run_shell_limited(
            "printf hello",
            CommandPolicy {
                timeout: Duration::from_secs(1),
                max_output_bytes: 128,
            },
        );
        assert!(result.success);
        assert_eq!(result.stdout, "hello");
    }

    #[test]
    fn limits_output() {
        let result = run_shell_limited(
            "yes x | head -c 4096",
            CommandPolicy {
                timeout: Duration::from_secs(1),
                max_output_bytes: 256,
            },
        );
        assert!(result.truncated);
        assert!(result.stdout.len() <= 256);
    }

    #[test]
    fn terminates_timeout() {
        let result = run_shell_limited(
            "sleep 2",
            CommandPolicy {
                timeout: Duration::from_millis(120),
                max_output_bytes: 128,
            },
        );
        assert!(result.timed_out);
    }
}

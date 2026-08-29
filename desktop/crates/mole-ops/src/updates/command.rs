//! Bounded external-command execution for update discovery and actions.
//! stdout/stderr are drained concurrently so a verbose package manager cannot
//! deadlock on a full pipe; retained diagnostics are capped while the readers
//! continue draining.

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Success,
    Failed,
    TimedOut,
    Unavailable,
    Cancelled,
}

#[derive(Debug)]
pub(crate) struct Output {
    pub status: Status,
    /// stdout alone for machine-readable parsers.
    pub stdout: String,
    /// Combined stdout/stderr for diagnostics.
    pub text: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status == Status::Success
    }
}

/// Run one argv without a shell. `cancelled` is checked during the wait;
/// callers pass the same flag used by the Tauri cancel command.
pub(crate) fn run(
    argv: &[String],
    timeout: Duration,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Output {
    let Some((program, arguments)) = argv.split_first() else {
        return Output {
            status: Status::Failed,
            stdout: String::new(),
            text: "empty command".into(),
        };
    };
    let mut child = match Command::new(program)
        .args(arguments)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .env("HOMEBREW_NO_ANALYTICS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Output {
                status: Status::Unavailable,
                stdout: String::new(),
                text: format!("{program} is not available"),
            };
        }
        Err(error) => {
            return Output {
                status: Status::Failed,
                stdout: String::new(),
                text: error.to_string(),
            };
        }
    };

    let stdout_reader = child
        .stdout
        .take()
        .map(|stdout| thread::spawn(move || drain(stdout)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|stderr| thread::spawn(move || drain(stderr)));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            break Status::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(exit)) => {
                break if exit.success() {
                    Status::Success
                } else {
                    Status::Failed
                }
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Status::TimedOut;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Output {
                    status: Status::Failed,
                    stdout: String::new(),
                    text: error.to_string(),
                };
            }
        }
    };

    let stdout_bytes = stdout_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    let stderr_bytes = stderr_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout_bytes).trim().to_string();
    let mut bytes = stdout_bytes;
    bytes.extend(stderr_bytes);
    let mut text = String::from_utf8_lossy(&bytes).trim().to_string();
    if status == Status::TimedOut {
        text = format!("{program} timed out after {}s", timeout.as_secs());
    } else if status == Status::Cancelled {
        text = format!("{program} was cancelled");
    } else if status == Status::Failed && text.is_empty() {
        text = format!("{program} failed without output");
    }
    Output {
        status,
        stdout,
        text,
    }
}

fn drain(mut reader: impl Read) -> Vec<u8> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 8192];
    while let Ok(read) = reader.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let remaining = OUTPUT_LIMIT.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    retained
}

pub(crate) fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn timeout_kills_the_child_and_names_the_cause() {
        let output = run(
            &strings(&["/bin/sleep", "2"]),
            Duration::from_millis(20),
            &AtomicBool::new(false),
        );
        assert_eq!(output.status, Status::TimedOut);
        assert!(output.text.contains("timed out"));
    }

    #[test]
    fn cancellation_is_distinct_from_timeout() {
        let cancelled = AtomicBool::new(true);
        let output = run(
            &strings(&["/bin/sleep", "2"]),
            Duration::from_secs(1),
            &cancelled,
        );
        assert_eq!(output.status, Status::Cancelled);
    }

    #[test]
    fn missing_program_is_unavailable() {
        let output = run(
            &["/definitely/missing/mole-update-tool".into()],
            Duration::from_millis(10),
            &AtomicBool::new(false),
        );
        assert_eq!(output.status, Status::Unavailable);
    }
}

//! Trusted external-process execution for plugin `process.run` calls.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::access::RuntimeAccess;

const MAX_CAPTURE: usize = 16 * 1024 * 1024;

/// A structured process invocation requested by a plugin.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProcessRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub environment: Vec<(String, String)>,
    pub stdin: Vec<u8>,
    pub timeout: Option<Duration>,
}

impl ProcessRequest {
    fn deadline(&self) -> Option<Instant> {
        self.timeout
            .and_then(|timeout| Instant::now().checked_add(timeout))
    }

    fn timed_out(&self) -> String {
        format!(
            "process `{}` timed out after {:?}",
            self.program,
            self.timeout.unwrap_or_default()
        )
    }
}

/// Captured process completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Run `request` under the plugin's access policy. The child inherits nothing
/// from the host environment except variables the policy explicitly grants,
/// and runs in the project root unless the request names a directory.
pub(crate) fn run(
    access: &RuntimeAccess,
    request: ProcessRequest,
) -> Result<ProcessOutput, String> {
    let permitted = access
        .permissions
        .process
        .iter()
        .any(|allowed| allowed == &request.program);
    if !permitted {
        return Err(format!("process `{}` is not permitted", request.program));
    }
    let command = command(access, &request)?;

    #[cfg(unix)]
    {
        unix::run(command, request)
    }
    #[cfg(windows)]
    {
        windows::run(command, request)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = command;
        Err(format!(
            "process `{}` cannot run: process execution is unsupported on this platform",
            request.program
        ))
    }
}

/// The command for `request`, with piped stdio and only the permitted environment.
fn command(access: &RuntimeAccess, request: &ProcessRequest) -> Result<Command, String> {
    let mut command = Command::new(&request.program);
    command
        .args(&request.args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(request.cwd.as_ref().unwrap_or(&access.project_root));
    for name in &access.permissions.environment {
        if let Ok(value) = std::env::var(name) {
            command.env(name, value);
        }
    }
    for (name, value) in &request.environment {
        if !access.permissions.environment.contains(name) {
            return Err(format!("environment variable `{name}` is not permitted"));
        }
        command.env(name, value);
    }
    Ok(command)
}

fn poll_delay(deadline: Option<Instant>) -> Duration {
    deadline.map_or(Duration::from_millis(1), |deadline| {
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(1))
    })
}

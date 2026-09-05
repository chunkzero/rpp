//! Trusted external-process execution for `rpp.process.run`.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::lua::runtime::RuntimeAccess;

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
    let native = access.is_native();
    let permitted = native
        || access
            .permissions
            .process
            .iter()
            .any(|allowed| allowed == &request.program);
    if !permitted {
        return Err(format!("process `{}` is not permitted", request.program));
    }

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
        if !native && !access.permissions.environment.contains(name) {
            return Err(format!("environment variable `{name}` is not permitted"));
        }
        command.env(name, value);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start `{}`: {error}", request.program))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture process stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture process stderr".to_string())?;
    let stdout_thread = thread::spawn(move || read_capped(stdout));
    let stderr_thread = thread::spawn(move || read_capped(stderr));

    if let Some(mut stdin) = child.stdin.take() {
        let input = request.stdin;
        thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }

    let deadline = request.timeout.map(|timeout| Instant::now() + timeout);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    terminate(&mut child);
                    let _ = child.wait();
                    return Err(format!(
                        "process `{}` timed out after {:?}",
                        request.program,
                        request.timeout.unwrap_or_default()
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                terminate(&mut child);
                return Err(format!("failed waiting for `{}`: {error}", request.program));
            }
        }
    };

    let stdout = stdout_thread
        .join()
        .map_err(|_| "stdout reader panicked".to_string())??;
    let stderr = stderr_thread
        .join()
        .map_err(|_| "stderr reader panicked".to_string())??;
    Ok(ProcessOutput {
        status: status.code().unwrap_or(-1),
        stdout,
        stderr,
    })
}

fn read_capped(mut reader: impl Read) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("failed reading process output: {error}"))?;
        if count == 0 {
            return Ok(output);
        }
        let remaining = MAX_CAPTURE.saturating_sub(output.len());
        output.extend_from_slice(&buffer[..count.min(remaining)]);
    }
}

fn terminate(child: &mut std::process::Child) {
    #[cfg(unix)]
    // SAFETY: `kill` on a negative pid targets the process group created by
    // `process_group(0)` above; it has no memory-safety preconditions.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

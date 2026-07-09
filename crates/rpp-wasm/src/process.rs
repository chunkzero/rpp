//! Structured external-process execution.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Permissions, ProcessOutput, ProcessRequest};

const MAX_CAPTURE: usize = 16 * 1024 * 1024;

pub(crate) fn run(
    permissions: &Permissions,
    request: ProcessRequest,
) -> Result<ProcessOutput, String> {
    if !permissions.arbitrary_processes
        && !permissions
            .processes
            .iter()
            .any(|allowed| allowed == &request.program)
    {
        return Err(format!("process `{}` is not permitted", request.program));
    }

    let mut command = Command::new(&request.program);
    command
        .args(&request.args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in &permissions.environment {
        command.env(name, value);
    }
    for (name, value) in &request.environment {
        if permissions.arbitrary_processes
            || permissions
                .environment
                .iter()
                .any(|(allowed, _)| allowed == name)
        {
            command.env(name, value);
        } else {
            return Err(format!("environment variable `{name}` is not permitted"));
        }
    }
    if let Some(cwd) = request
        .cwd
        .as_ref()
        .or(permissions.working_directory.as_ref())
    {
        command.current_dir(cwd);
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
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

//! Trusted external-process execution for `rpp.process.run`.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::lua::runtime::RuntimeAccess;

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
        unix::run(command, request)
    }
    #[cfg(not(unix))]
    {
        Err("process execution is currently supported only on Unix".to_string())
    }
}

#[cfg(unix)]
mod unix {
    use std::io::{self, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{ProcessOutput, ProcessRequest};

    const MAX_CAPTURE: usize = 16 * 1024 * 1024;

    struct Supervisor(Child);

    impl Drop for Supervisor {
        fn drop(&mut self) {
            // SAFETY: the child leads the process group created below. A negative
            // pid targets that group; kill has no memory-safety preconditions.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            self.0.stdin.take();
            self.0.stdout.take();
            self.0.stderr.take();
            let _ = self.0.wait();
        }
    }

    pub(super) fn run(
        mut command: Command,
        request: ProcessRequest,
    ) -> Result<ProcessOutput, String> {
        let deadline = request
            .timeout
            .and_then(|timeout| Instant::now().checked_add(timeout));
        command.process_group(0);
        let mut supervisor = Supervisor(
            command
                .spawn()
                .map_err(|error| format!("failed to start `{}`: {error}", request.program))?,
        );
        let child = &mut supervisor.0;
        nonblocking(child.stdin.as_ref().expect("piped stdin"))?;
        nonblocking(child.stdout.as_ref().expect("piped stdout"))?;
        nonblocking(child.stderr.as_ref().expect("piped stderr"))?;

        let mut input = request.stdin.as_slice();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut status = None;
        loop {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(format!(
                    "process `{}` timed out after {:?}",
                    request.program,
                    request.timeout.unwrap_or_default()
                ));
            }
            if let Some(stdin) = child.stdin.as_mut() {
                if !input.is_empty() {
                    match stdin.write(&input[..input.len().min(8192)]) {
                        Ok(count) => input = &input[count..],
                        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => input = &[],
                        Err(error) if retry(&error) => {}
                        Err(error) => return Err(format!("failed writing process stdin: {error}")),
                    }
                }
                if input.is_empty() {
                    child.stdin.take();
                }
            }
            read_capped(&mut child.stdout, &mut stdout)?;
            read_capped(&mut child.stderr, &mut stderr)?;
            if status.is_none() {
                status = child.try_wait().map_err(|error| {
                    format!("failed waiting for `{}`: {error}", request.program)
                })?;
            }
            if let Some(status) = status {
                if child.stdin.is_none() && child.stdout.is_none() && child.stderr.is_none() {
                    return Ok(ProcessOutput {
                        status: status.code().unwrap_or(-1),
                        stdout,
                        stderr,
                    });
                }
            }
            let delay = deadline.map_or(Duration::from_millis(1), |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(1))
            });
            thread::sleep(delay);
        }
    }

    fn nonblocking(pipe: &impl AsRawFd) -> Result<(), String> {
        let fd = pipe.as_raw_fd();
        // SAFETY: fd belongs to a live pipe; these fcntl operations take integer arguments.
        let result = unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags == -1 {
                -1
            } else {
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK)
            }
        };
        if result == -1 {
            return Err(format!(
                "failed configuring process pipe: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    fn retry(error: &io::Error) -> bool {
        matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
        )
    }

    fn read_capped(reader: &mut Option<impl Read>, output: &mut Vec<u8>) -> Result<(), String> {
        let Some(pipe) = reader.as_mut() else {
            return Ok(());
        };
        let mut buffer = [0u8; 8192];
        // Bound each drain so a continuously writing process cannot starve the deadline.
        for _ in 0..32 {
            match pipe.read(&mut buffer) {
                Ok(0) => {
                    reader.take();
                    return Ok(());
                }
                Ok(count) => {
                    let remaining = MAX_CAPTURE.saturating_sub(output.len());
                    output.extend_from_slice(&buffer[..count.min(remaining)]);
                }
                Err(error) if retry(&error) => return Ok(()),
                Err(error) => return Err(format!("failed reading process output: {error}")),
            }
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::Instant;

    use super::*;

    fn shell(script: &str, stdin: Vec<u8>, timeout: Duration) -> Result<ProcessOutput, String> {
        let mut access = RuntimeAccess::sandboxed(std::env::current_dir().unwrap());
        access.permissions.process.push("/bin/sh".into());
        run(
            &access,
            ProcessRequest {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                stdin,
                timeout: Some(timeout),
                ..Default::default()
            },
        )
    }

    #[test]
    fn descendant_holding_output_pipes_times_out() {
        let started = Instant::now();
        let error = shell("sleep 2 &", Vec::new(), Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn blocked_stdin_times_out_and_terminates_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("survived");
        let script = format!("(sleep 0.4; touch '{}') & sleep 2", marker.display());
        let started = Instant::now();
        let error =
            shell(&script, vec![b'x'; 1024 * 1024], Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
        std::thread::sleep(Duration::from_millis(500));
        assert!(!marker.exists(), "descendant survived process timeout");
    }

    #[test]
    fn captures_both_streams_while_writing_stdin() {
        let input = vec![b'x'; 1024 * 1024];
        let output = shell(
            "cat; printf error >&2; exit 7",
            input.clone(),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(output.status, 7);
        assert_eq!(output.stdout, input);
        assert_eq!(output.stderr, b"error");
    }

    #[test]
    fn capture_limit_still_drains_excess_output() {
        let output = shell(
            "head -c 17000000 /dev/zero",
            Vec::new(),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(output.status, 0);
        assert_eq!(output.stdout.len(), 16 * 1024 * 1024);
    }
}

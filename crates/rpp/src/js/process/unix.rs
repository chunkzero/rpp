//! Unix process supervision: the child leads its own process group, which is killed
//! on drop, and its pipes are polled without blocking.

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command};
use std::thread;
use std::time::Instant;

use super::{poll_delay, ProcessOutput, ProcessRequest, MAX_CAPTURE};

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

pub(super) fn run(mut command: Command, request: ProcessRequest) -> Result<ProcessOutput, String> {
    let deadline = request.deadline();
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
            return Err(request.timed_out());
        }
        write_stdin(&mut child.stdin, &mut input)?;
        read_capped(&mut child.stdout, &mut stdout)?;
        read_capped(&mut child.stderr, &mut stderr)?;
        if status.is_none() {
            status = child
                .try_wait()
                .map_err(|error| format!("failed waiting for `{}`: {error}", request.program))?;
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
        thread::sleep(poll_delay(deadline));
    }
}

/// Write the next chunk of `input`, closing stdin once `input` is exhausted or the
/// child has closed its end.
fn write_stdin(stdin: &mut Option<ChildStdin>, input: &mut &[u8]) -> Result<(), String> {
    let Some(pipe) = stdin.as_mut() else {
        return Ok(());
    };
    let pending = *input;
    if !pending.is_empty() {
        match pipe.write(&pending[..pending.len().min(8192)]) {
            Ok(count) => *input = &pending[count..],
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => *input = &[],
            Err(error) if retry(&error) => {}
            Err(error) => return Err(format!("failed writing process stdin: {error}")),
        }
    }
    if input.is_empty() {
        stdin.take();
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::js::access::RuntimeAccess;
    use crate::js::process::{run, ProcessOutput, ProcessRequest};

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

//! Windows process supervision: the child runs in a kill-on-close job object, and
//! threads feed its stdin and drain its output.

use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use super::{poll_delay, ProcessOutput, ProcessRequest, MAX_CAPTURE};

/// A job whose remaining members are terminated when its last handle closes.
struct Job(HANDLE);

impl Job {
    fn new() -> io::Result<Self> {
        // SAFETY: null attributes and name create an anonymous job with default security.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(handle);
        // SAFETY: the limit struct is plain data and the pointer/length describe it exactly.
        let ok = unsafe {
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    fn assign(&self, child: &Child) -> io::Result<()> {
        // SAFETY: both handles are open; the process handle stays owned by `child`.
        let ok = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateJobObjectW and is closed exactly once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Supervisor {
    child: Child,
    job: Option<Job>,
}

impl Supervisor {
    /// Start `command` and assign it to a new job.
    fn spawn(mut command: Command, program: &str) -> Result<Self, String> {
        let job = Job::new().map_err(|error| format!("failed creating process job: {error}"))?;
        let child = command
            .spawn()
            .map_err(|error| format!("failed to start `{program}`: {error}"))?;
        let supervisor = Supervisor {
            child,
            job: Some(job),
        };
        supervisor
            .job
            .as_ref()
            .expect("job is present until drop")
            .assign(&supervisor.child)
            .map_err(|error| format!("failed assigning `{program}` to its process job: {error}"))?;
        Ok(supervisor)
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        // Closing the last job handle terminates every remaining member, which
        // releases the pipes held by descendants so reader threads reach EOF.
        self.job.take();
        // Assignment may have failed before the child joined the job.
        let _ = self.child.kill();
        self.child.stdin.take();
        self.child.stdout.take();
        self.child.stderr.take();
        let _ = self.child.wait();
    }
}

pub(super) fn run(command: Command, mut request: ProcessRequest) -> Result<ProcessOutput, String> {
    let deadline = request.deadline();
    let mut supervisor = Supervisor::spawn(command, &request.program)?;
    let child = &mut supervisor.child;

    let input = std::mem::take(&mut request.stdin);
    let mut stdin = child.stdin.take().expect("piped stdin");
    let writer = thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let stdout = spawn_reader(child.stdout.take().expect("piped stdout"));
    let stderr = spawn_reader(child.stderr.take().expect("piped stderr"));

    let mut status = None;
    loop {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            // Dropping the supervisor kills the job; the detached threads then finish.
            return Err(request.timed_out());
        }
        if status.is_none() {
            status = child
                .try_wait()
                .map_err(|error| format!("failed waiting for `{}`: {error}", request.program))?;
        }
        if let Some(status) = status {
            if writer.is_finished() && stdout.is_finished() && stderr.is_finished() {
                let _ = writer.join();
                return Ok(ProcessOutput {
                    status: status.code().unwrap_or(-1),
                    stdout: join_reader(stdout)?,
                    stderr: join_reader(stderr)?,
                });
            }
        }
        thread::sleep(poll_delay(deadline));
    }
}

type Reader = JoinHandle<Result<Vec<u8>, String>>;

fn spawn_reader(mut pipe: impl Read + Send + 'static) -> Reader {
    thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => return Ok(output),
                Ok(count) => {
                    let remaining = MAX_CAPTURE.saturating_sub(output.len());
                    output.extend_from_slice(&buffer[..count.min(remaining)]);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(format!("failed reading process output: {error}")),
            }
        }
    })
}

fn join_reader(reader: Reader) -> Result<Vec<u8>, String> {
    reader
        .join()
        .map_err(|_| "process output reader panicked".to_string())?
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::{Job, Supervisor};
    use crate::js::access::RuntimeAccess;
    use crate::js::process::{run, ProcessOutput, ProcessRequest};

    fn cmd(script: &str, stdin: Vec<u8>, timeout: Duration) -> Result<ProcessOutput, String> {
        let mut access = RuntimeAccess::sandboxed(std::env::current_dir().unwrap());
        access.permissions.process.push("cmd".into());
        run(
            &access,
            ProcessRequest {
                program: "cmd".into(),
                args: vec!["/C".into(), script.into()],
                stdin,
                timeout: Some(timeout),
                ..Default::default()
            },
        )
    }

    #[test]
    fn dropping_unassigned_child_terminates_it() {
        let job = Job::new().unwrap();
        let child = Command::new("ping.exe")
            .args(["-n", "6", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // Failed assignment leaves a live child and an empty job.
        let mut supervisor = Supervisor {
            child,
            job: Some(job),
        };
        assert!(supervisor.child.try_wait().unwrap().is_none());

        let started = Instant::now();
        drop(supervisor);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn descendant_holding_output_pipes_times_out() {
        let started = Instant::now();
        let error = cmd(
            "start /B ping -n 4 127.0.0.1",
            Vec::new(),
            Duration::from_millis(200),
        )
        .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn captures_both_streams_while_writing_stdin() {
        let output = cmd(
            r"C:\Windows\System32\findstr.exe x & (echo error) 1>&2 & exit 7",
            b"x\r\n".to_vec(),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(output.status, 7);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "x");
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "error");
    }
}

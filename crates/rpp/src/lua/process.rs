//! Trusted external-process execution for `rpp.process.run`.

use std::path::PathBuf;
use std::process::{Command, Stdio};
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

fn poll_delay(deadline: Option<Instant>) -> Duration {
    deadline.map_or(Duration::from_millis(1), |deadline| {
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(1))
    })
}

#[cfg(unix)]
mod unix {
    use std::io::{self, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
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

    pub(super) fn run(
        mut command: Command,
        request: ProcessRequest,
    ) -> Result<ProcessOutput, String> {
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
            thread::sleep(poll_delay(deadline));
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

#[cfg(windows)]
mod windows {
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

    pub(super) fn run(
        mut command: Command,
        mut request: ProcessRequest,
    ) -> Result<ProcessOutput, String> {
        let deadline = request.deadline();
        let job = Job::new().map_err(|error| format!("failed creating process job: {error}"))?;
        let child = command
            .spawn()
            .map_err(|error| format!("failed to start `{}`: {error}", request.program))?;
        let mut supervisor = Supervisor {
            child,
            job: Some(job),
        };
        supervisor
            .job
            .as_ref()
            .expect("job is present until drop")
            .assign(&supervisor.child)
            .map_err(|error| {
                format!(
                    "failed assigning `{}` to its process job: {error}",
                    request.program
                )
            })?;
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
                status = child.try_wait().map_err(|error| {
                    format!("failed waiting for `{}`: {error}", request.program)
                })?;
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
        use std::process::Stdio;
        use std::time::Duration;

        use super::*;

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

#[cfg(all(test, windows))]
mod tests {
    use std::time::Instant;

    use super::*;

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
            "findstr x & (echo error) 1>&2 & exit 7",
            b"x\r\n".to_vec(),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(output.status, 7);
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "x");
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "error");
    }
}

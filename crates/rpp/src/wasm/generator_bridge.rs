//! Generator-phase host bridge for WASM plugins (spec §5).

use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use rpp_wasm::{HostCallbacks, LogLevel};

use crate::model::GeneratorHost;
use crate::util::path::validate_relative;

/// A request from the guest's host callbacks to the engine's
/// [`GeneratorHost`], carrying a one-shot reply channel where needed.
pub(super) enum HostRequest {
    ListFiles {
        pattern: Option<String>,
        reply: Sender<Vec<String>>,
    },
    ReadFile {
        path: String,
        reply: Sender<Option<Vec<u8>>>,
    },
    ReadSource {
        path: String,
        reply: Sender<Option<Vec<u8>>>,
    },
    Emit {
        path: String,
        contents: Vec<u8>,
        reply: Sender<()>,
    },
    Remove {
        path: String,
        reply: Sender<()>,
    },
}

/// The [`HostCallbacks`] implementation handed to `rpp-wasm` at instantiation.
pub(super) struct ChannelHost {
    pub plugin_id: String,
    pub requests: Sender<HostRequest>,
    pub error: Arc<Mutex<Option<String>>>,
}

impl ChannelHost {
    /// Send a request expecting a reply, returning the default on disconnect.
    fn request<T: Default>(&self, make: impl FnOnce(Sender<T>) -> HostRequest) -> T {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded::<T>(1);
        if self.requests.send(make(reply_tx)).is_err() {
            return T::default();
        }
        reply_rx.recv().unwrap_or_default()
    }

    fn valid_path(&self, path: &str) -> bool {
        match validate_relative(path) {
            Ok(()) => true,
            Err(message) => {
                *self.error.lock() = Some(message);
                false
            }
        }
    }
}

impl HostCallbacks for ChannelHost {
    fn log(&mut self, level: LogLevel, message: &str) {
        log_message(&self.plugin_id, level, message);
    }

    fn list_files(&mut self, pattern: Option<&str>) -> Vec<String> {
        let pattern = pattern.map(str::to_string);
        self.request(|reply| HostRequest::ListFiles { pattern, reply })
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        if !self.valid_path(path) {
            return None;
        }
        let path = path.to_string();
        self.request(|reply| HostRequest::ReadFile { path, reply })
    }

    fn read_source(&mut self, path: &str) -> Option<Vec<u8>> {
        if !self.valid_path(path) {
            return None;
        }
        let path = path.to_string();
        self.request(|reply| HostRequest::ReadSource { path, reply })
    }

    fn emit_file(&mut self, path: &str, contents: Vec<u8>) {
        if !self.valid_path(path) {
            return;
        }
        self.request(|reply| HostRequest::Emit {
            path: path.to_string(),
            contents,
            reply,
        });
    }

    fn remove_file(&mut self, path: &str) {
        if !self.valid_path(path) {
            return;
        }
        self.request(|reply| HostRequest::Remove {
            path: path.to_string(),
            reply,
        });
    }
}

/// Service one host request against the real [`GeneratorHost`].
pub(super) fn serve_request(host: &mut dyn GeneratorHost, req: HostRequest) {
    match req {
        HostRequest::ListFiles { pattern, reply } => {
            let _ = reply.send(host.list_files(pattern.as_deref()));
        }
        HostRequest::ReadFile { path, reply } => {
            let _ = reply.send(host.read_file(&path));
        }
        HostRequest::ReadSource { path, reply } => {
            let _ = reply.send(host.read_source(&path));
        }
        HostRequest::Emit {
            path,
            contents,
            reply,
        } => {
            host.emit(&path, contents);
            let _ = reply.send(());
        }
        HostRequest::Remove { path, reply } => {
            host.remove(&path);
            let _ = reply.send(());
        }
    }
}

/// Run the guest's `generate` on a scoped thread while servicing host requests.
pub(super) fn run_generate<F>(
    plugin_id: &str,
    requests: &Receiver<HostRequest>,
    host_error: Arc<Mutex<Option<String>>>,
    host: &mut dyn GeneratorHost,
    generate: F,
) -> crate::error::Result<()>
where
    F: FnOnce() -> Result<(), rpp_wasm::Error> + Send,
{
    std::thread::scope(|scope| {
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        let worker = scope.spawn(move || {
            let result = generate();
            let _ = done_tx.send(());
            result
        });

        loop {
            crossbeam_channel::select! {
                recv(requests) -> req => match req {
                    Ok(req) => serve_request(host, req),
                    Err(_) => break,
                },
                recv(done_rx) -> _ => {
                    for req in requests.try_iter() {
                        serve_request(host, req);
                    }
                    break;
                },
            }
        }

        let result = match worker.join() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(crate::error::Error::Generator {
                plugin: plugin_id.to_string(),
                message: e.to_string(),
            }),
            Err(_) => Err(crate::error::Error::Generator {
                plugin: plugin_id.to_string(),
                message: "generator worker thread panicked".into(),
            }),
        };
        if let Some(message) = host_error.lock().take() {
            return Err(crate::error::Error::Generator {
                plugin: plugin_id.to_string(),
                message,
            });
        }
        result
    })
}

/// Route a guest log message to `tracing` (when enabled) or stderr.
#[allow(unused_variables)]
fn log_message(plugin_id: &str, level: LogLevel, message: &str) {
    #[cfg(feature = "tracing")]
    {
        match level {
            LogLevel::Debug => tracing::debug!(plugin = plugin_id, "{message}"),
            LogLevel::Info => tracing::info!(plugin = plugin_id, "{message}"),
            LogLevel::Warn => tracing::warn!(plugin = plugin_id, "{message}"),
            LogLevel::Error => tracing::error!(plugin = plugin_id, "{message}"),
        }
    }
    #[cfg(not(feature = "tracing"))]
    {
        let label = match level {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        };
        eprintln!("[{label}] {plugin_id}: {message}");
    }
}

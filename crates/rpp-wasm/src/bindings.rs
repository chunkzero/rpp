//! Generated component bindings and the host-side trait implementation.

use wasmtime::component::bindgen;

bindgen!({
    world: "rpp-plugin",
    path: "wit",
    // Host import functions may return an error to trap the guest; we keep
    // them infallible here (they never trap) for a simpler surface.
});

pub use self::exports::rpp::plugin::guest;
pub use self::rpp::plugin::host::LogLevel;

use crate::StoreData;

impl rpp::plugin::host::Host for StoreData {
    fn log(&mut self, level: LogLevel, message: String) {
        let level = match level {
            LogLevel::Debug => crate::LogLevel::Debug,
            LogLevel::Info => crate::LogLevel::Info,
            LogLevel::Warn => crate::LogLevel::Warn,
            LogLevel::Error => crate::LogLevel::Error,
        };
        self.host.log(level, &message);
    }

    fn list_files(&mut self, pattern: Option<String>) -> Vec<String> {
        if !self.in_generate {
            return Vec::new();
        }
        self.host.list_files(pattern.as_deref())
    }

    fn read_file(&mut self, path: String) -> Option<Vec<u8>> {
        if !self.in_generate {
            return None;
        }
        self.host.read_file(&path)
    }

    fn read_source(&mut self, path: String) -> Option<Vec<u8>> {
        if !self.in_generate {
            return None;
        }
        self.host.read_source(&path)
    }

    fn emit_file(&mut self, path: String, contents: Vec<u8>) {
        if !self.in_generate {
            return;
        }
        self.host.emit_file(&path, contents);
    }

    fn remove_file(&mut self, path: String) {
        if !self.in_generate {
            return;
        }
        self.host.remove_file(&path);
    }
}

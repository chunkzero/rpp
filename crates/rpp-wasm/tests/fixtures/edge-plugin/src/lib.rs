//! Test fixture guest for the rpp-wasm host.
//!
//! Declares several trivial processors that each exercise one host edge case:
//!
//! * `keep`    — returns `Unchanged`.
//! * `drop`    — returns `Dropped`.
//! * `fail`    — returns `Err("boom")`.
//! * `probe`   — calls every generator-phase host function during the
//!               *process* phase and reports, via a guest error string, what
//!               the host returned (used to verify out-of-generate suppression).
//! * `spin`    — loops forever (used to exercise the epoch deadline / timeout).
//! * `bomb`    — allocates a huge `Vec` (used to exercise the memory limit).
//!
//! `generate` emits two files so the host emit path can be verified.

wit_bindgen::generate!({
    world: "rpp-plugin",
    path: "../../../wit",
});

use exports::rpp::plugin::guest::{FileData, Guest, PluginInfo, ProcessResult, ProcessorDef};
use rpp::plugin::host;
use rpp::plugin::host::LogLevel;

struct Component;

fn proc(name: &str) -> ProcessorDef {
    ProcessorDef {
        name: name.to_string(),
        patterns: vec!["**/*".to_string()],
        priority: 0,
    }
}

impl Guest for Component {
    fn configure(_options_json: String) {}

    fn get_info() -> PluginInfo {
        PluginInfo {
            id: "edge".to_string(),
            version: "0.1.0".to_string(),
            processors: vec![
                proc("keep"),
                proc("drop"),
                proc("fail"),
                proc("probe"),
                proc("spin"),
                proc("bomb"),
            ],
            has_generator: true,
        }
    }

    fn process(processor: String, file: FileData) -> Result<ProcessResult, String> {
        match processor.as_str() {
            "keep" => Ok(ProcessResult::Unchanged),
            "drop" => Ok(ProcessResult::Dropped),
            "fail" => Err("boom".to_string()),
            "probe" => {
                // All of these run OUTSIDE the generate phase, so the host
                // must suppress them: empty list, none, no-op writes.
                let listed = host::list_files(None).len();
                let read = host::read_file("anything").is_some();
                let src = host::read_source("anything").is_some();
                host::emit_file("should_not_appear", b"x");
                host::remove_file("should_not_appear");
                // log is always permitted.
                host::log(LogLevel::Debug, "probe ran");
                Err(format!("probe: listed={listed} read={read} src={src}"))
            }
            "spin" => {
                #[allow(clippy::empty_loop)]
                loop {
                    std::hint::spin_loop();
                }
            }
            "bomb" => {
                // Try to allocate far beyond the configured memory cap. The
                // memory growth should trap (or the allocator should fail) under
                // a small StoreLimits cap. The size is derived from a runtime
                // value so it is not const-folded, and stays under the wasm32
                // address space so it does not overflow at compile time.
                let huge: usize = 3_500_000_000; // ~3.3 GiB
                let mut v: Vec<u8> = Vec::new();
                v.try_reserve(huge).map_err(|_| "alloc-failed".to_string())?;
                // Touch the pages so the growth is real, not lazy.
                v.resize(huge, file.contents.first().copied().unwrap_or(7));
                Ok(ProcessResult::Modified(FileData {
                    path: file.path,
                    contents: vec![v[0]],
                }))
            }
            other => Err(format!("unknown processor: {other}")),
        }
    }

    fn generate() -> Result<(), String> {
        // Exercise the host read/list paths (results depend on the host) and
        // emit two files.
        let _ = host::list_files(Some("**/*.json"));
        host::emit_file("gen_a.txt", b"alpha");
        host::emit_file("gen_b.txt", b"beta");
        host::log(LogLevel::Info, "generate emitted two files");
        Ok(())
    }
}

export!(Component);

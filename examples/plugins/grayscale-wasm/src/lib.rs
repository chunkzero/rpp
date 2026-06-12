//! Example rpp WASM plugin.
//!
//! Implements a single processor, `grayscale`, that matches `**/*.gray.png`,
//! decodes the PNG, converts it to grayscale, re-encodes it, and renames the
//! file to strip the `.gray` infix (so `foo.gray.png` becomes `foo.png`).
//!
//! It also exports a generator that writes `grayscale_report.json` listing how
//! many files were processed in this instance.

// Generate bindings for the `rpp-plugin` world from the host crate's WIT.
wit_bindgen::generate!({
    world: "rpp-plugin",
    path: "../../../crates/rpp-wasm/wit",
});

use std::cell::Cell;
use std::io::Cursor;

use exports::rpp::plugin::guest::{Guest, PluginInfo, ProcessResult, ProcessorDef};
use rpp::plugin::host;
use rpp::plugin::host::LogLevel;

use image::ImageFormat;

// Count of files this instance has converted. The component model gives each
// instance its own copy of this static, so no synchronization is needed (the
// guest is single-threaded).
thread_local! {
    static PROCESSED: Cell<u32> = const { Cell::new(0) };
}

struct Component;

impl Guest for Component {
    fn configure(_options_json: String) {
        // This plugin has no configurable options.
    }

    fn get_info() -> PluginInfo {
        PluginInfo {
            id: "grayscale".to_string(),
            version: "0.1.0".to_string(),
            processors: vec![ProcessorDef {
                name: "grayscale".to_string(),
                patterns: vec!["**/*.gray.png".to_string()],
                priority: 0,
            }],
            has_generator: true,
        }
    }

    fn process(
        processor: String,
        file: exports::rpp::plugin::guest::FileData,
    ) -> Result<ProcessResult, String> {
        if processor != "grayscale" {
            return Err(format!("unknown processor: {processor}"));
        }

        let image = image::load_from_memory_with_format(&file.contents, ImageFormat::Png)
            .map_err(|e| format!("failed to decode PNG {}: {e}", file.path))?;

        let gray = image::DynamicImage::ImageLuma8(image.to_luma8());

        let mut out = Cursor::new(Vec::new());
        gray.write_to(&mut out, ImageFormat::Png)
            .map_err(|e| format!("failed to encode PNG {}: {e}", file.path))?;

        let new_path = strip_gray_suffix(&file.path);
        host::log(
            LogLevel::Info,
            &format!("grayscale: {} -> {}", file.path, new_path),
        );

        PROCESSED.with(|c| c.set(c.get() + 1));

        Ok(ProcessResult::Modified(
            exports::rpp::plugin::guest::FileData {
                path: new_path,
                contents: out.into_inner(),
            },
        ))
    }

    fn generate() -> Result<(), String> {
        let count = PROCESSED.with(|c| c.get());
        let report = format!("{{\"processed\":{count}}}\n");
        host::emit_file("grayscale_report.json", report.as_bytes());
        host::log(
            LogLevel::Info,
            &format!("grayscale: wrote report ({count} files)"),
        );
        Ok(())
    }
}

/// Turn `dir/foo.gray.png` into `dir/foo.png`. If the `.gray.png` suffix is
/// absent the path is returned unchanged.
fn strip_gray_suffix(path: &str) -> String {
    match path.strip_suffix(".gray.png") {
        Some(stem) => format!("{stem}.png"),
        None => path.to_string(),
    }
}

export!(Component);

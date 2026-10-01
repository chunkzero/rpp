use std::fmt::Write;
use std::path::PathBuf;
use std::{env, fs};

use deno_core::snapshot::{create_snapshot, CreateSnapshotOptions};

#[path = "src/extensions.rs"]
mod extensions;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/extensions.rs");
    let extensions = extensions::web();
    let mut sources = Vec::new();
    for extension in &extensions {
        for (kind, files) in [
            ("JS", &extension.lazy_loaded_js_files),
            ("ESM", &extension.lazy_loaded_esm_files),
        ] {
            for file in files.iter() {
                let source = file.load().expect("load extension source").to_string();
                sources.push((kind, file.specifier, source));
            }
        }
    }
    sources.sort_unstable_by_key(|(kind, specifier, _)| (*kind, *specifier));
    let snapshot = create_snapshot(
        CreateSnapshotOptions {
            cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
            startup_snapshot: None,
            skip_op_registration: false,
            extensions,
            extension_transpiler: None,
            with_runtime_cb: None,
        },
        None,
    )
    .expect("create JavaScript runtime snapshot");
    for path in snapshot.files_loaded_during_snapshot {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    fs::write(output.join("snapshot.bin"), snapshot.output).expect("write runtime snapshot");
    let mut residual = String::new();
    for kind in ["JS", "ESM"] {
        writeln!(residual, "pub const {kind}: &[(&str, &str)] = &[").unwrap();
        for (_, specifier, source) in sources.iter().filter(|(source_kind, specifier, _)| {
            *source_kind == kind
                && !snapshot
                    .consumed_lazy_specifiers
                    .iter()
                    .any(|consumed| consumed == specifier)
        }) {
            writeln!(residual, "({specifier:?}, {source:?}),").unwrap();
        }
        residual.push_str("];\n");
    }
    fs::write(output.join("snapshot_sources.rs"), residual).expect("write lazy extension sources");
}

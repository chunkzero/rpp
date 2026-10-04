//! End-to-end test of the checked-in grayscale WASM example plugin, which needs the
//! `wasm32-wasip2` target to build its guest component.

mod common;

use std::path::{Path, PathBuf};

use common::{build, build_wasm_guest, copy_dir, examples_dir, stderr, wasip2_available, write};

fn encode_png(
    color: png::ColorType,
    size: (u32, u32),
    data: &[u8],
    trns: Option<Vec<u8>>,
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size.0, size.1);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    if let Some(trns) = trns {
        encoder.set_trns(trns);
    }
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(data).unwrap();
    writer.finish().unwrap();
    out
}

fn decode_gray_alpha(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    let mut reader = png::Decoder::new(bytes.as_slice()).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::GrayscaleAlpha);
    pixels.truncate(info.buffer_size());
    pixels
}

/// Copies the example plugin into a fresh project under `tmp` with its guest built, and
/// seeds two textures. Returns the project root.
fn grayscale_project(tmp: &Path) -> PathBuf {
    let root = tmp.join("project");
    let plugin = root.join("plugins/grayscale-wasm");
    copy_dir(&examples_dir().join("plugins/grayscale-wasm"), &plugin);
    let guest = build_wasm_guest(
        &examples_dir().join("plugins/grayscale-wasm/guest"),
        &tmp.join("guest-target"),
        "grayscale_wasm_guest.wasm",
        true,
        &[],
    );
    std::fs::copy(guest, plugin.join("grayscale.wasm")).unwrap();

    write(
        &root,
        "rpp.json",
        br#"{ "dependencies": { "grayscale-wasm": "path:plugins/grayscale-wasm" } }"#,
    );
    write(
        &root,
        "rpp.config.ts",
        br##"import { defineConfig, plugin } from "#rpp/config";

export default defineConfig({
  pack: { name: "gray", format: 34 },
  build: { workers: 1 },
  plugins: [plugin("grayscale-wasm")],
});
"##,
    );
    let (red, translucent_blue) = ([255, 0, 0, 255], [0, 0, 255, 128]);
    write(
        &root,
        "src/assets/minecraft/textures/block/stone.png",
        encode_png(
            png::ColorType::Rgba,
            (2, 1),
            &[red, translucent_blue].concat(),
            None,
        ),
    );
    write(
        &root,
        "src/assets/minecraft/textures/block/transparent.png",
        encode_png(
            png::ColorType::Rgb,
            (2, 1),
            &[255, 0, 0, 0, 255, 0],
            Some(vec![0, 255, 0, 0, 0, 0]),
        ),
    );
    root
}

#[test]
fn grayscale_example_converts_textures() {
    if !wasip2_available() {
        eprintln!("SKIP: wasm32-wasip2 target is unavailable");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = grayscale_project(tmp.path());

    let out = build(&root, &["--no-squash"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let textures = root.join("dist/assets/minecraft/textures/block");
    // luma(red) = 76, alpha kept; luma(blue) = 29, alpha 128.
    assert_eq!(
        decode_gray_alpha(&textures.join("stone.png")),
        [76, 255, 29, 128]
    );
    assert_eq!(
        decode_gray_alpha(&textures.join("transparent.png")),
        [76, 0, 149, 255]
    );

    let out = build(&root, &["--no-squash"]);
    assert!(stderr(&out).contains("processed 0"), "{}", stderr(&out));
}

//! Builds the `examples/plugins/grayscale-wasm` guest crate and runs the
//! example plugin through the engine over a real PNG.

use std::path::{Path, PathBuf};
use std::process::Command;

use rpp_cli::project::Project;

fn wasip2_available() -> bool {
    let Ok(output) = Command::new("rustc").args(["--print", "sysroot"]).output() else {
        return false;
    };
    Path::new(String::from_utf8_lossy(&output.stdout).trim())
        .join("lib/rustlib/wasm32-wasip2/lib")
        .is_dir()
}

fn example_plugin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/grayscale-wasm")
}

fn build_guest(target_dir: &Path) -> PathBuf {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "build",
            "--locked",
            "--release",
            "--target",
            "wasm32-wasip2",
        ])
        .env("CARGO_TARGET_DIR", target_dir)
        .current_dir(example_plugin().join("guest"))
        .output()
        .expect("build grayscale guest");
    assert!(
        output.status.success(),
        "guest build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    target_dir.join("wasm32-wasip2/release/grayscale_wasm_guest.wasm")
}

fn encode_png(pixels: &[[u8; 4]], width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let data: Vec<u8> = pixels.iter().flatten().copied().collect();
        writer.write_image_data(&data).unwrap();
    }
    out
}

fn encode_rgb_with_transparency() -> Vec<u8> {
    let mut input = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut input, 2, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_trns(vec![0, 255, 0, 0, 0, 0]);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 0, 255, 0]).unwrap();
    }
    input
}

#[test]
fn grayscale_example_converts_textures() {
    if !wasip2_available() {
        eprintln!("SKIP: wasm32-wasip2 target is unavailable");
        return;
    }
    let temporary = tempfile::tempdir().unwrap();
    let wasm = build_guest(&temporary.path().join("guest-target"));

    // A copy of the example plugin with the built component dropped in.
    let plugin = temporary.path().join("grayscale-wasm");
    for name in ["plugin.toml", "init.lua"] {
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::copy(example_plugin().join(name), plugin.join(name)).unwrap();
    }
    std::fs::copy(wasm, plugin.join("grayscale.wasm")).unwrap();

    let root = temporary.path().join("project");
    let textures = root.join("src/assets/minecraft/textures/block");
    std::fs::create_dir_all(&textures).unwrap();
    std::fs::write(
        root.join("rpp.toml"),
        "[pack]\nname = \"gray\"\npack_format = 34\n\n[build]\nworkers = 1\n\n[[plugin]]\nsource = \"path:../grayscale-wasm\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/pack.mcmeta"),
        r#"{"pack":{"pack_format":34,"description":""}}"#,
    )
    .unwrap();
    let red = [255, 0, 0, 255];
    let translucent_blue = [0, 0, 255, 128];
    std::fs::write(
        textures.join("stone.png"),
        encode_png(&[red, translucent_blue], 2, 1),
    )
    .unwrap();
    std::fs::write(
        textures.join("transparent.png"),
        encode_rgb_with_transparency(),
    )
    .unwrap();
    let mut project = Project::discover_isolated(&root).unwrap();
    project.config.build.workers = 1;
    let result = project.build_engine().unwrap().build().unwrap();
    assert_eq!(result.processed, 3);

    let out = std::fs::read(root.join("dist/assets/minecraft/textures/block/stone.png")).unwrap();
    let decoder = png::Decoder::new(out.as_slice());
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::GrayscaleAlpha);
    // luma(red) = 76, alpha kept; luma(blue) = 29, alpha 128.
    assert_eq!(&pixels[..info.buffer_size()], &[76, 255, 29, 128]);

    let out =
        std::fs::read(root.join("dist/assets/minecraft/textures/block/transparent.png")).unwrap();
    let mut reader = png::Decoder::new(out.as_slice()).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::GrayscaleAlpha);
    assert_eq!(&pixels[..info.buffer_size()], &[76, 0, 149, 255]);

    // Warm build replays from cache.
    let warm = project.build_engine().unwrap().build().unwrap();
    assert_eq!(warm.processed, 0);
}

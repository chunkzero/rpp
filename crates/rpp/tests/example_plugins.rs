//! End-to-end test of the checked-in example pack and plugins.
//!
//! This drives the build [`Engine`] directly (the CLI is out of scope here)
//! against the real `examples/pack` source and the three example Lua plugins in
//! `examples/plugins/`, using the shared-plugin options the example `rpp.toml` declares.
//! The CLI integration test additionally covers the pack-local catalog plugin.
//! It proves the examples actually work:
//!
//!   * json-minify collapses whitespace-heavy JSON in the output;
//!   * mcmeta-validate runs (an invalid pack.mcmeta makes the build fail);
//!   * hash-rename renames `custom/` textures and emits a consistent
//!     `rename_map.json`;
//!   * `.rppignore`d files never reach the output;
//!   * the incremental cache works: a clean second build, and editing one file
//!     reprocesses only that file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rpp::config::Config;
use rpp::engine::{BuildResult, Engine};
use rpp::lua::{LuaPluginFactory, LuaPluginLimits, PackInfo, RuntimeAccess};
use rpp::model::PluginFactory;

use tempfile::TempDir;

/// Absolute path to the repo root (the workspace dir containing `examples/`).
fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is `<root>/crates/rpp`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root is two levels above crates/rpp")
        .to_path_buf()
}

fn examples_dir() -> PathBuf {
    repo_root().join("examples")
}

/// Recursively copy `from` into `to`.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

/// A scratch project that copies `examples/pack/src` into an isolated tempdir so
/// the build's cache and output never touch the checked-in tree.
struct PackProject {
    dir: TempDir,
}

impl PackProject {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        copy_dir(
            &examples_dir().join("pack").join("src"),
            &dir.path().join("src"),
        );
        PackProject { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn out(&self, rel: &str) -> PathBuf {
        self.root().join("dist").join(rel)
    }

    fn read_out(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.out(rel)).ok()
    }

    fn out_exists(&self, rel: &str) -> bool {
        self.out(rel).exists()
    }

    fn write_src(&self, rel: &str, contents: &str) {
        let path = self.root().join("src").join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }
}

/// Load one example plugin into a factory with the given options TOML, using the
/// example pack's metadata (name + pack_format 34, matching `pack.mcmeta`).
fn plugin(name: &str, options_toml: &str) -> Arc<dyn PluginFactory> {
    let dir = examples_dir().join("plugins").join(name);
    let options: toml::Value = toml::from_str(options_toml).unwrap();
    let factory = LuaPluginFactory::load(
        &dir,
        options,
        PackInfo {
            name: "rpp-example-pack".into(),
            description: Some(
                "A tiny but complete Minecraft resource pack, built end-to-end by rpp.".into(),
            ),
            format: Some(34),
        },
        LuaPluginLimits::default(),
        RuntimeAccess::sandboxed(".".into()),
    )
    .unwrap_or_else(|e| panic!("load plugin {name}: {e}"));
    Arc::new(factory)
}

/// The three shared plugins with the options from `examples/pack/rpp.toml`.
fn example_plugins() -> Vec<Arc<dyn PluginFactory>> {
    vec![
        plugin("json-minify", "pretty = false"),
        plugin("mcmeta-validate", ""),
        plugin(
            "hash-rename",
            "files = [\"assets/*/textures/custom/**/*.png\"]",
        ),
    ]
}

/// A minimal config for the shared plugins (squash is irrelevant here: the
/// Engine does not run the squash crate, so JSON minification is the plugin's).
fn config() -> Config {
    Config::parse(
        "[pack]\nname = \"rpp-example-pack\"\npack_format = 34\n",
        "rpp.toml",
    )
    .unwrap()
}

fn build(project: &PackProject, plugins: Vec<Arc<dyn PluginFactory>>) -> BuildResult {
    Engine::builder(config())
        .project_root(project.root())
        .plugins(plugins)
        .build_engine()
        .unwrap()
        .build()
        .unwrap()
}

/// The hashed name hash-rename should produce for `custom/gem.png`, recomputed
/// here independently from the raw source bytes (first 8 hex chars of xxh3).
fn expected_gem_hash(project: &PackProject) -> String {
    let bytes = std::fs::read(
        project
            .root()
            .join("src/assets/minecraft/textures/custom/gem.png"),
    )
    .unwrap();
    // Matches `rpp.hash.xxh3(bytes):sub(1, 8)` in the plugin: the first 8 hex
    // chars of the 16-char lowercase xxh3-64 digest.
    let hash = format!("{:016x}", twox_hash::XxHash3_64::oneshot(&bytes));
    hash[..8].to_string()
}

#[test]
fn example_pack_builds_with_all_three_plugins() {
    let project = PackProject::new();
    let result = build(&project, example_plugins());

    // Something was processed and the generator emitted the rename map.
    assert!(result.processed > 0, "expected processed files");
    assert!(
        result.generated >= 1,
        "expected the rename-map generator output"
    );

    // --- json-minify: the whitespace-heavy block model is now compact. ---
    let block_model = project
        .read_out("assets/minecraft/models/block/rpp_bricks.json")
        .expect("block model in output");
    assert!(
        !block_model.contains('\n') && !block_model.contains("  "),
        "block model should be minified, got: {block_model:?}"
    );
    // And still valid, semantically intact JSON.
    let parsed: serde_json::Value = serde_json::from_str(&block_model).unwrap();
    assert_eq!(parsed["parent"], "minecraft:block/cube_all");
    assert_eq!(parsed["textures"]["all"], "minecraft:block/rpp_bricks");

    // pack.mcmeta is also minified by the plugin.
    let mcmeta = project.read_out("pack.mcmeta").expect("pack.mcmeta output");
    assert!(!mcmeta.contains('\n'), "pack.mcmeta should be minified");
    let mcmeta_json: serde_json::Value = serde_json::from_str(&mcmeta).unwrap();
    assert_eq!(mcmeta_json["pack"]["pack_format"], 34);

    // --- hash-rename: the custom texture is renamed; vanilla ones are not. ---
    let hash = expected_gem_hash(&project);
    let hashed = format!("assets/minecraft/textures/custom/gem.{hash}.png");
    assert!(
        project.out_exists(&hashed),
        "expected hashed texture {hashed}"
    );
    assert!(
        !project.out_exists("assets/minecraft/textures/custom/gem.png"),
        "original custom texture name should be gone"
    );
    // Vanilla textures keep their fixed names (models reference them).
    assert!(project.out_exists("assets/minecraft/textures/item/diamond_sword.png"));
    assert!(project.out_exists("assets/minecraft/textures/block/rpp_bricks.png"));

    // --- rename_map.json: emitted and consistent with the renamed output. ---
    let map_text = project
        .read_out("rename_map.json")
        .expect("rename_map.json emitted");
    let map: serde_json::Value = serde_json::from_str(&map_text).unwrap();
    assert_eq!(
        map["assets/minecraft/textures/custom/gem.png"],
        serde_json::Value::String(hashed.clone()),
        "rename map must point at the actually-emitted hashed path"
    );

    // --- .rppignore: the design notes never reach the output. ---
    assert!(
        !project.out_exists("notes/design.txt"),
        ".rppignore should exclude *.txt"
    );

    // --- mcmeta-validate emits no files but must have run without failing. ---
    // (A separate test proves it fails on bad input.)
    assert!(
        project.out_exists("assets/minecraft/textures/block/ember.png.mcmeta"),
        "animation mcmeta should pass through to output"
    );
}

#[test]
fn second_build_is_fully_cached() {
    let project = PackProject::new();

    let first = build(&project, example_plugins());
    assert!(first.processed > 0);
    assert_eq!(first.cached, 0, "first build processes everything");

    let second = build(&project, example_plugins());
    assert_eq!(second.processed, 0, "second build should reprocess nothing");
    assert_eq!(
        second.cached, first.processed,
        "every previously-processed file should now be a cache hit"
    );
    // No source change => the generator reuses its cached output, nothing written.
    assert!(
        second.changes.written.is_empty(),
        "a no-op rebuild should write nothing, wrote: {:?}",
        second.changes.written
    );
}

#[test]
fn touching_one_file_reprocesses_only_it() {
    let project = PackProject::new();
    build(&project, example_plugins());

    // Edit exactly one source file (still valid JSON).
    project.write_src(
        "assets/minecraft/lang/en_us.json",
        "{ \"pack.rpp.example.title\" : \"Edited Title\" }",
    );

    let second = build(&project, example_plugins());
    assert_eq!(second.processed, 1, "only the edited file reprocesses");
    assert!(second.cached >= 1, "the rest stay cached");

    // The edit took effect and was minified.
    let lang = project
        .read_out("assets/minecraft/lang/en_us.json")
        .unwrap();
    assert_eq!(lang, "{\"pack.rpp.example.title\":\"Edited Title\"}");
}

#[test]
fn mcmeta_validate_fails_on_pack_format_mismatch() {
    let project = PackProject::new();
    // Rewrite pack.mcmeta with a pack_format that disagrees with rpp.toml (34).
    project.write_src(
        "pack.mcmeta",
        "{ \"pack\": { \"pack_format\": 9, \"description\": \"wrong\" } }",
    );

    let result = Engine::builder(config())
        .project_root(project.root())
        .plugins(example_plugins())
        .build_engine()
        .unwrap()
        .build();

    let err = result.expect_err("validation should fail the build");
    let msg = err.to_string();
    assert!(
        msg.contains("pack_format") || msg.contains("mcmeta validation failed"),
        "error should mention the validation failure, got: {msg}"
    );
}

#[test]
fn mcmeta_validate_fails_on_bad_animation() {
    let project = PackProject::new();
    // frametime must be a positive integer; 0 is invalid.
    project.write_src(
        "assets/minecraft/textures/block/ember.png.mcmeta",
        "{ \"animation\": { \"frametime\": 0 } }",
    );

    let result = Engine::builder(config())
        .project_root(project.root())
        .plugins(example_plugins())
        .build_engine()
        .unwrap()
        .build();

    let err = result.expect_err("invalid animation should fail the build");
    assert!(
        err.to_string().contains("frametime"),
        "error should mention frametime, got: {err}"
    );
}

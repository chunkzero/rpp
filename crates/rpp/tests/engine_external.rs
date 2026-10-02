//! External (non-pack) outputs and destination boundary tests with mock plugins.

mod common;

#[cfg(unix)]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::path::Path;
use std::sync::Arc;

use common::mock::{cache_key, MockFactory};
use common::Project;
#[cfg(unix)]
use rpp::config::{PluginConfig, PluginPermissions, SecurityMode};
use rpp::engine::Engine;
use rpp::model::{GeneratorHost, PluginFactory, ProcessOutcome};

type Generate = dyn Fn(&mut dyn GeneratorHost) + Send + Sync;

/// A plugin whose generator writes to the `code` output root at `root`.
fn external(
    id: &str,
    key: u64,
    root: &str,
    run: impl Fn(&mut dyn GeneratorHost) + Send + Sync + 'static,
) -> Arc<dyn PluginFactory> {
    Arc::new(
        MockFactory::new(id, key, |_, _| ProcessOutcome::Unchanged)
            .with_output_root("code", root)
            .with_generator(move |host| {
                run(host);
                Ok(())
            }),
    )
}

fn build(project: &Project, plugins: Vec<Arc<dyn PluginFactory>>) -> rpp::engine::BuildResult {
    engine(project, plugins).build().unwrap()
}

fn engine(project: &Project, plugins: Vec<Arc<dyn PluginFactory>>) -> Engine {
    Engine::builder(project.config())
        .project_root(project.root())
        .plugins(plugins)
        .build_engine()
        .unwrap()
}

fn clean(project: &Project) -> rpp::Result<()> {
    rpp::engine::clean_project_artifacts(&project.config(), project.root())
}

fn gen_kt() -> Arc<Generate> {
    Arc::new(|host| {
        let version = host.read_source("v.txt").unwrap_or_default();
        host.emit_output("code", "Gen.kt", version);
        host.emit("a.txt", b"generated".to_vec());
    })
}

fn codegen(run: &Arc<Generate>) -> Arc<dyn PluginFactory> {
    let run = Arc::clone(run);
    external("codegen", 1, "generated", move |host| run(host))
}

#[test]
fn external_outputs_have_durable_stale_ownership_and_clean_support() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    let first = build(
        &project,
        vec![external("codegen", 1, "generated", |host| {
            host.emit_output("code", "Keep.kt", b"keep-v1".to_vec());
            host.emit_output("code", "Stale.kt", b"stale".to_vec());
        })],
    );
    assert_eq!(first.changes.external.written.len(), 2);
    std::fs::write(project.root().join("generated/Manual.kt"), "manual").unwrap();

    // Simulate `rpp build --no-cache`: ownership deliberately lives outside
    // this directory and must still remove Stale.kt.
    std::fs::remove_dir_all(project.root().join(".rpp/cache")).unwrap();
    let keep_v2 = || {
        external("codegen", 2, "generated", |host| {
            host.emit_output("code", "Keep.kt", b"keep-v2".to_vec());
        })
    };
    let second = build(&project, vec![keep_v2()]);
    assert_eq!(
        std::fs::read_to_string(project.root().join("generated/Keep.kt")).unwrap(),
        "keep-v2"
    );
    assert!(!project.root().join("generated/Stale.kt").exists());
    assert!(project.root().join("generated/Manual.kt").exists());
    assert_eq!(second.changes.external.removed.len(), 1);

    clean(&project).unwrap();
    assert!(!project.root().join("generated/Keep.kt").exists());
    assert!(project.root().join("generated/Manual.kt").exists());
}

#[test]
fn colliding_external_outputs_fail_with_plugin_attribution() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    build(&project, Vec::new());
    project.write_src("a.txt", "changed");
    let make = |id: &str| {
        external(id, cache_key(id), "generated", |host| {
            host.emit_output("code", "Same.kt", b"generated".to_vec());
        })
    };
    let error = engine(&project, vec![make("first"), make("second")])
        .build()
        .unwrap_err()
        .to_string();
    assert!(error.contains("`first` and `second`"), "{error}");
    assert!(error.contains("Same.kt"), "{error}");
    assert_eq!(project.read_out("a.txt").as_deref(), Some("a"));
    assert!(!project.root().join("generated/Same.kt").exists());
}

#[test]
fn ownership_conflict_publishes_nothing() {
    let project = Project::new();
    project.write_src("v.txt", "v1");
    let plugin = gen_kt();
    build(&project, vec![codegen(&plugin)]);
    let manifest = project.root().join(".rpp/cache/manifest.bin");
    let manifest_before = std::fs::read(&manifest).unwrap();

    project.write_src("v.txt", "v2");
    project.write_src("a.txt", "handwritten");
    let error = engine(&project, vec![codegen(&plugin)])
        .build()
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot emit `a.txt`"), "{error}");
    assert_eq!(project.read_out("a.txt").as_deref(), Some("generated"));
    assert_eq!(
        std::fs::read_to_string(project.root().join("generated/Gen.kt")).unwrap(),
        "v1"
    );
    assert_eq!(std::fs::read(&manifest).unwrap(), manifest_before);
}

#[test]
fn external_output_refuses_unowned_handwritten_file() {
    let project = Project::new();
    project.write_src("v.txt", "v1");
    std::fs::create_dir(project.root().join("generated")).unwrap();
    std::fs::write(project.root().join("generated/Gen.kt"), "handwritten").unwrap();
    let error = engine(&project, vec![codegen(&gen_kt())])
        .build()
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("refusing to overwrite unowned file"),
        "{error}"
    );
    assert!(error.contains("Gen.kt"), "{error}");
    assert_eq!(
        std::fs::read_to_string(project.root().join("generated/Gen.kt")).unwrap(),
        "handwritten"
    );
    assert!(!project.out_exists("a.txt"));
}

#[test]
fn external_output_adopts_identical_unowned_file() {
    let project = Project::new();
    project.write_src("v.txt", "v1");
    std::fs::create_dir(project.root().join("generated")).unwrap();
    std::fs::write(project.root().join("generated/Gen.kt"), "v1").unwrap();
    let plugin = gen_kt();
    build(&project, vec![codegen(&plugin)]);
    clean(&project).unwrap();
    assert!(!project.root().join("generated/Gen.kt").exists());
}

#[cfg(unix)]
#[test]
fn destination_boundaries_reject_ancestor_symlinks_before_build_and_clean() {
    use std::os::unix::fs::symlink;

    let project = Project::new();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(outside.path().join("victim")).unwrap();
    let keep = outside.path().join("victim/keep.txt");
    std::fs::write(&keep, "keep").unwrap();
    let mut config = project.config();
    config.build.output = "alias/victim".into();
    let engine = Engine::builder(config.clone())
        .project_root(project.root())
        .build_engine()
        .unwrap();
    symlink(outside.path(), project.root().join("alias")).unwrap();
    assert!(engine.build().is_err());
    assert!(rpp::engine::clean_project_artifacts(&config, project.root()).is_err());
    assert_eq!(std::fs::read_to_string(keep).unwrap(), "keep");
    assert!(!project.root().join(".rpp").exists());
}

#[cfg(unix)]
#[test]
fn destination_boundaries_reject_external_aliases_and_protected_roots() {
    use std::os::unix::fs::symlink;

    let project = Project::new();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), "keep").unwrap();
    symlink(outside.path(), project.root().join("alias")).unwrap();
    symlink(project.src(), project.root().join("source-alias")).unwrap();
    for root in [
        "alias",
        "source-alias",
        "src/generated",
        "dist/generated",
        ".rpp/generated",
    ] {
        let mut config = project.config();
        config.plugins.push(PluginConfig {
            package: "plugin".into(),
            options: serde_json::json!({}),
            security: SecurityMode::Sandboxed,
            permissions: PluginPermissions::default(),
            outputs: BTreeMap::from([("code".to_string(), root.into())]),
        });
        assert!(
            Engine::builder(config.clone())
                .project_root(project.root())
                .build_engine()
                .is_err(),
            "{root}"
        );
        assert!(
            rpp::engine::clean_project_artifacts(&config, project.root()).is_err(),
            "{root}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(outside.path().join("keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn destination_boundaries_allow_sibling_external_outputs_and_clean() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    let plugin = external("sibling", 1, "../sibling", |host| {
        host.emit_output("code", "nested/generated.txt", b"generated".to_vec());
    });
    let engine = Engine::builder(Project::new().config())
        .project_root(&project)
        .plugin(plugin)
        .build_engine()
        .unwrap();
    engine.build().unwrap();
    let generated = parent.path().join("sibling/nested/generated.txt");
    assert_eq!(std::fs::read_to_string(&generated).unwrap(), "generated");
    std::fs::write(parent.path().join("sibling/keep.txt"), "keep").unwrap();
    rpp::engine::clean_project_artifacts(&Project::new().config(), &project).unwrap();
    assert!(!generated.exists());
    assert_eq!(
        std::fs::read_to_string(parent.path().join("sibling/keep.txt")).unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[test]
fn destination_boundaries_reject_retargeted_owned_external_parent() {
    let project = Project::new();
    let plugin = external("external", 1, "generated", |host| {
        host.emit_output("code", "nested/keep.txt", b"generated".to_vec());
    });
    let engine = engine(&project, vec![plugin]);
    engine.build().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), "keep").unwrap();
    std::fs::remove_dir_all(project.root().join("generated/nested")).unwrap();
    std::os::unix::fs::symlink(outside.path(), project.root().join("generated/nested")).unwrap();
    assert!(engine.build().is_err());
    assert!(clean(&project).is_err());
    assert_eq!(
        std::fs::read_to_string(outside.path().join("keep.txt")).unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[test]
fn destination_boundaries_allow_symlinked_source_and_sibling_parents() {
    use std::os::unix::fs::symlink;

    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(parent.path().join("real-src")).unwrap();
    std::fs::write(parent.path().join("real-src/a.txt"), "a").unwrap();
    symlink(parent.path().join("real-src"), project.join("src")).unwrap();
    std::fs::create_dir_all(parent.path().join("real-server")).unwrap();
    symlink(
        parent.path().join("real-server"),
        parent.path().join("server"),
    )
    .unwrap();

    let factory = |root: &str| {
        external("sibling", 1, root, |host| {
            host.emit_output("code", "generated.txt", b"generated".to_vec());
        })
    };
    let build_in = |project: &Path, root: &str| {
        Engine::builder(Project::new().config())
            .project_root(project)
            .plugin(factory(root))
            .build_engine()
    };
    build_in(&project, "../server/generated")
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(parent.path().join("real-server/generated/generated.txt")).unwrap(),
        "generated"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("dist/a.txt")).unwrap(),
        "a"
    );

    // A symlinked parent that resolves into the pack output is still rejected.
    symlink(project.join("dist"), parent.path().join("alias")).unwrap();
    assert!(build_in(&project, "../alias/generated").is_err());
}

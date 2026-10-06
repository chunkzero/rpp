//! Recovery from cache objects lost or truncated by a power failure.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::engine::build;
use common::mock::MockFactory;
use common::Project;
use rpp::model::{PluginFactory, ProcessOutcome};

fn codegen() -> Arc<dyn PluginFactory> {
    Arc::new(
        MockFactory::new("codegen", 1, |_, file| {
            file.contents = file.contents.to_ascii_uppercase();
            ProcessOutcome::Modified
        })
        .with_output_root("code", "generated")
        .with_generator(|host| {
            host.emit_output("code", "Gen.kt", b"object Gen".to_vec());
            host.emit("gen.txt", b"generated".to_vec());
            Ok(())
        }),
    )
}

fn objects(project: &Project) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(project.root().join(".rpp/cache/objects"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn assert_outputs(project: &Project) {
    assert_eq!(project.read_out("a.txt").as_deref(), Some("A"));
    assert_eq!(project.read_out("gen.txt").as_deref(), Some("generated"));
    let kotlin = std::fs::read_to_string(project.root().join("generated/Gen.kt")).unwrap();
    assert_eq!(kotlin, "object Gen");
}

fn remove_outputs(root: &Path) {
    std::fs::remove_dir_all(root.join("dist")).unwrap();
    std::fs::remove_file(root.join("generated/Gen.kt")).unwrap();
}

#[test]
fn truncated_and_missing_objects_are_rebuilt_and_repaired() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    build(&project, vec![codegen()]);

    // Intact pack outputs are reused without their damaged objects.
    let objects_before = objects(&project);
    assert!(objects_before.len() > 1);
    for object in &objects_before {
        std::fs::File::create(object).unwrap();
    }
    std::fs::remove_file(&objects_before[0]).unwrap();
    let hit = build(&project, vec![codegen()]);
    assert_eq!(hit.processed, 0);
    assert_outputs(&project);

    // Without the outputs, damaged objects force rebuilding, which repairs them.
    for object in objects(&project) {
        std::fs::File::create(object).unwrap();
    }
    remove_outputs(project.root());
    let rebuilt = build(&project, vec![codegen()]);
    assert_eq!((rebuilt.processed, rebuilt.generated), (1, 1));
    assert_outputs(&project);

    remove_outputs(project.root());
    let replayed = build(&project, vec![codegen()]);
    assert_eq!((replayed.processed, replayed.generated), (0, 0));
    assert_outputs(&project);
}

#[test]
fn torn_outputs_are_rewritten_on_a_cache_hit() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    build(&project, vec![codegen()]);

    let root = project.root();
    std::fs::File::create(root.join("dist/a.txt")).unwrap();
    std::fs::write(root.join("dist/gen.txt"), "gen").unwrap();
    std::fs::File::create(root.join("generated/Gen.kt")).unwrap();
    let hit = build(&project, vec![codegen()]);
    assert_eq!((hit.processed, hit.generated), (0, 0));
    assert_eq!(hit.changes.written.len(), 2);
    assert_eq!(hit.changes.external.written.len(), 1);
    assert_outputs(&project);
}

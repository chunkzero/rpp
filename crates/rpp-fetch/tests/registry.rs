//! Offline tests for registry dependency resolution against a mock registry.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use rpp_fetch::registry::{
    parse_dependencies, resolve, PackageLock, Registry, ResolvedPackage, Update,
};
use rpp_fetch::Error;
use semver::Version;

mod common;

use common::{Canned, MockServer};

const RPP: &str = "0.2.0";

fn json(value: &serde_json::Value) -> Canned {
    Canned {
        status: 200,
        content_type: "application/json",
        body: serde_json::to_vec(value).unwrap(),
    }
}

/// A gzipped tar with `rpp.json` for `name` `version` plus `files`, at the archive root.
fn archive(name: &str, version: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let manifest = format!(r#"{{"name": "{name}", "version": "{version}"}}"#);
    let mut builder = tar::Builder::new(Vec::new());
    let all = std::iter::once(("rpp.json", manifest.as_bytes())).chain(files.iter().copied());
    for (path, contents) in all {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, contents).unwrap();
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&builder.into_inner().unwrap()).unwrap();
    encoder.finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Serve `plugins/window.json` listing `versions` as `(version, rpp, yanked)`, each
/// backed by an archive of the same version.
fn serve_window(server: &MockServer, versions: &[(&str, &str, bool)]) {
    let listed: Vec<_> = versions
        .iter()
        .map(|(version, rpp, yanked)| {
            let bytes = archive("window", version, &[("data/x.txt", version.as_bytes())]);
            let path = format!("/archives/window-{version}.tgz");
            server.set(
                path.clone(),
                Canned {
                    status: 200,
                    content_type: "application/gzip",
                    body: bytes.clone(),
                },
            );
            serde_json::json!({
                "version": version,
                "url": format!("{}{path}", server.base()),
                "sha256": sha256(&bytes),
                "rpp": rpp,
                "yanked": yanked,
            })
        })
        .collect();
    server.set(
        "/plugins/window.json",
        json(&serde_json::json!({
            "name": "window",
            "repository": "https://example.com/window",
            "versions": listed,
        })),
    );
}

fn standard_versions() -> [(&'static str, &'static str, bool); 4] {
    [
        ("0.1.0", ">=0.1", false),
        ("0.1.1", ">=0.1", false),
        ("0.1.2", ">=0.1", true),
        ("0.1.3", ">=9.0", false),
    ]
}

fn registry(server: &MockServer, cache: &Path) -> Registry {
    Registry::new(server.config(), cache)
}

fn run(
    registry: &Registry,
    project: &Path,
    manifest: &str,
    lock: &mut PackageLock,
    update: &Update,
) -> rpp_fetch::Result<Vec<ResolvedPackage>> {
    let deps = parse_dependencies(manifest).unwrap();
    resolve(
        registry,
        project,
        &deps,
        lock,
        &Version::parse(RPP).unwrap(),
        update,
    )
}

fn window_manifest(spec: &str) -> String {
    format!(r#"{{"dependencies": {{"window": "{spec}"}}}}"#)
}

#[test]
fn installs_and_locks_newest_compatible() {
    let server = MockServer::start(HashMap::new());
    serve_window(&server, &standard_versions());
    let (cache, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut lock = PackageLock::new();

    let resolved = run(
        &registry(&server, cache.path()),
        project.path(),
        &window_manifest("^0.1"),
        &mut lock,
        &Update::None,
    )
    .unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].version, Some(Version::new(0, 1, 1)));
    assert_eq!(
        std::fs::read_to_string(resolved[0].root.join("data/x.txt")).unwrap(),
        "0.1.1"
    );
    let pin = lock.get("window").unwrap();
    assert_eq!(pin.version, Version::new(0, 1, 1));
    assert_eq!(pin.requested, "^0.1");
}

#[test]
fn locked_resolution_with_warm_cache_makes_no_requests() {
    let server = MockServer::start(HashMap::new());
    serve_window(&server, &standard_versions());
    let (cache, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let registry = registry(&server, cache.path());
    let manifest = window_manifest("^0.1");
    let mut lock = PackageLock::new();
    let first = run(
        &registry,
        project.path(),
        &manifest,
        &mut lock,
        &Update::None,
    )
    .unwrap();

    let hits = server.hits();
    let second = run(
        &registry,
        project.path(),
        &manifest,
        &mut lock,
        &Update::None,
    )
    .unwrap();

    assert_eq!(server.hits(), hits);
    assert_eq!(first, second);
}

#[test]
fn hash_mismatch_fails_and_caches_nothing() {
    let server = MockServer::start(HashMap::new());
    serve_window(&server, &[("0.1.0", ">=0.1", false)]);
    let url = format!("{}/archives/window-0.1.0.tgz", server.base());
    let cache = tempfile::tempdir().unwrap();

    let err = registry(&server, cache.path())
        .install("window", &Version::new(0, 1, 0), &url, &"ab".repeat(32))
        .unwrap_err();

    assert!(matches!(err, Error::HashMismatch { .. }), "{err:?}");
    assert_eq!(std::fs::read_dir(cache.path()).unwrap().count(), 0);
}

#[test]
fn archive_with_wrong_manifest_is_rejected() {
    let server = MockServer::start(HashMap::new());
    let bytes = archive("other", "0.1.0", &[]);
    server.set(
        "/a.tgz",
        Canned {
            status: 200,
            content_type: "application/gzip",
            body: bytes.clone(),
        },
    );
    let cache = tempfile::tempdir().unwrap();

    let err = registry(&server, cache.path())
        .install(
            "window",
            &Version::new(0, 1, 0),
            &format!("{}/a.tgz", server.base()),
            &sha256(&bytes),
        )
        .unwrap_err();

    match err {
        Error::PackageMismatch {
            expected, found, ..
        } => {
            assert_eq!(expected, "window 0.1.0");
            assert_eq!(found, "other 0.1.0");
        }
        other => panic!("unexpected {other:?}"),
    }
    let package_dir = cache.path().join("window");
    assert!(!package_dir.exists() || std::fs::read_dir(package_dir).unwrap().count() == 0);
}

#[test]
fn yanked_pin_still_installs() {
    let server = MockServer::start(HashMap::new());
    serve_window(&server, &standard_versions());
    let project = tempfile::tempdir().unwrap();
    let manifest = window_manifest("^0.1");
    let mut lock = PackageLock::new();
    run(
        &registry(&server, tempfile::tempdir().unwrap().path()),
        project.path(),
        &manifest,
        &mut lock,
        &Update::None,
    )
    .unwrap();

    serve_window(
        &server,
        &[
            ("0.1.0", ">=0.1", false),
            ("0.1.1", ">=0.1", true),
            ("0.1.2", ">=0.1", false),
        ],
    );
    let fresh = tempfile::tempdir().unwrap();
    let resolved = run(
        &registry(&server, fresh.path()),
        project.path(),
        &manifest,
        &mut lock,
        &Update::None,
    )
    .unwrap();

    assert_eq!(resolved[0].version, Some(Version::new(0, 1, 1)));
}

#[test]
fn changed_request_reselects() {
    let server = MockServer::start(HashMap::new());
    serve_window(
        &server,
        &[("0.1.0", ">=0.1", false), ("0.2.0", ">=0.1", false)],
    );
    let (cache, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let registry = registry(&server, cache.path());
    let mut lock = PackageLock::new();

    run(
        &registry,
        project.path(),
        &window_manifest("^0.1"),
        &mut lock,
        &Update::None,
    )
    .unwrap();
    assert_eq!(lock.get("window").unwrap().version, Version::new(0, 1, 0));

    let resolved = run(
        &registry,
        project.path(),
        &window_manifest("^0.2"),
        &mut lock,
        &Update::None,
    )
    .unwrap();
    assert_eq!(resolved[0].version, Some(Version::new(0, 2, 0)));
    assert_eq!(lock.get("window").unwrap().requested, "^0.2");
}

#[test]
fn path_dependency_is_not_locked() {
    let server = MockServer::start(HashMap::new());
    let (cache, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let local = project.path().join("vendor/window");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(
        local.join("rpp.json"),
        r#"{"name": "window", "version": "1.0.0"}"#,
    )
    .unwrap();
    let mut lock = PackageLock::new();

    let resolved = run(
        &registry(&server, cache.path()),
        project.path(),
        &window_manifest("path:vendor/window"),
        &mut lock,
        &Update::None,
    )
    .unwrap();

    assert_eq!(resolved[0].root, local.canonicalize().unwrap());
    assert_eq!(resolved[0].version, None);
    assert!(lock.packages().is_empty());
    assert_eq!(server.hits(), 0);
}

#[test]
fn unknown_plugin_is_reported() {
    let server = MockServer::start(HashMap::new());
    let cache = tempfile::tempdir().unwrap();

    let err = registry(&server, cache.path()).entry("ghost").unwrap_err();

    assert!(matches!(err, Error::UnknownPackage(name) if name == "ghost"));
}

#[test]
fn search_filters_index() {
    let server = MockServer::start(HashMap::new());
    server.set(
        "/index.json",
        json(&serde_json::json!([
            {"name": "zoom", "repository": "https://example.com/zoom", "latest": "1.0.0",
             "description": "Camera WINDOW zoom"},
            {"name": "window", "repository": "https://example.com/window", "latest": "0.1.0"},
            {"name": "atlas", "repository": "https://example.com/atlas", "latest": "2.0.0",
             "description": "Texture atlases"},
        ])),
    );
    let cache = tempfile::tempdir().unwrap();

    let hits = registry(&server, cache.path()).search("Window").unwrap();

    let names: Vec<_> = hits.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, ["window", "zoom"]);
}

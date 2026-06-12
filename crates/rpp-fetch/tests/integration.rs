//! Offline integration tests for `rpp-fetch`.
//!
//! These spin up a tiny `std::net::TcpListener`-backed HTTP server that serves
//! canned GitHub API/codeload responses (including a real gzipped tarball built
//! in-process with `tar` + `flate2`). No network access is required.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use flate2::write::GzEncoder;
use flate2::Compression;

use rpp_fetch::{HttpConfig, LockedPlugin, Lockfile, PluginSource, Resolver};

/// A canned response for a given request path.
#[derive(Clone)]
struct Canned {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

/// A minimal single-threaded mock HTTP server. Maps request path (ignoring the
/// query string for API routes, but matching exactly for the search route via a
/// prefix) to a canned response, and counts total requests served.
struct MockServer {
    base: String,
    hits: Arc<AtomicUsize>,
    shutdown: Arc<Mutex<bool>>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    fn start(routes: HashMap<String, Canned>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr");
        let base = format!("http://{addr}");

        let hits = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(Mutex::new(false));

        let hits_t = Arc::clone(&hits);
        let shutdown_t = Arc::clone(&shutdown);
        let routes = Arc::new(routes);

        let handle = std::thread::spawn(move || loop {
            if *shutdown_t.lock().unwrap() {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    handle_conn(stream, &routes, &hits_t);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => break,
            }
        });

        MockServer {
            base,
            hits,
            shutdown,
            handle: Some(handle),
        }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    fn config(&self) -> HttpConfig {
        HttpConfig::with_base(self.base.clone())
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        *self.shutdown.lock().unwrap() = true;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn handle_conn(mut stream: TcpStream, routes: &HashMap<String, Canned>, hits: &AtomicUsize) {
    stream.set_nonblocking(false).ok();
    // Read the request head (until CRLFCRLF). We only need the request line.
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let head = String::from_utf8_lossy(&buf);
    let request_line = head.lines().next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let _method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");

    hits.fetch_add(1, Ordering::SeqCst);

    // Match exact path; for the search route, match by prefix (ignore query).
    let path_only = target.split('?').next().unwrap_or(target);
    let canned = routes
        .get(target)
        .or_else(|| routes.get(path_only))
        .or_else(|| {
            // Prefix match for search route keyed as "/search/repositories".
            routes
                .iter()
                .find(|(k, _)| path_only.starts_with(k.as_str()) && k.contains("search"))
                .map(|(_, v)| v)
        });

    let response = match canned {
        Some(c) => c.clone(),
        None => Canned {
            status: 404,
            content_type: "text/plain",
            body: b"not found".to_vec(),
        },
    };

    let status_text = match response.status {
        200 => "OK",
        404 => "Not Found",
        _ => "Status",
    };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        status_text,
        response.content_type,
        response.body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&response.body);
    let _ = stream.flush();
}

/// Build a gzipped tarball whose entries are wrapped in `top` (mimicking
/// GitHub's `<repo>-<sha>/` wrapper). `entries` is a list of (relative-path,
/// contents). If `extra` is provided it is appended verbatim (used to inject a
/// path-traversal entry).
fn build_tarball(top: &str, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar_builder = tar::Builder::new(Vec::new());
    for (path, contents) in entries {
        let full = format!("{top}/{path}");
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar_builder
            .append_data(&mut header, full, *contents)
            .unwrap();
    }
    let tar_bytes = tar_builder.into_inner().unwrap();

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&tar_bytes).unwrap();
    encoder.finish().unwrap()
}

/// Build a tarball containing a single entry whose path escapes the root.
///
/// The `tar` crate's high-level API refuses to write a path containing `..`, so
/// we craft the 512-byte ustar header by hand to inject a malicious path the way
/// a hostile archive would.
fn build_evil_tarball(top: &str) -> Vec<u8> {
    let evil = b"pwned";
    let name = format!("{top}/../../escape.txt");

    let mut header = [0u8; 512];
    // name (offset 0, len 100)
    let name_bytes = name.as_bytes();
    header[..name_bytes.len()].copy_from_slice(name_bytes);
    // mode (offset 100, len 8), octal ASCII + NUL
    write_octal(&mut header[100..108], 0o644);
    // uid / gid (offset 108 / 116)
    write_octal(&mut header[108..116], 0);
    write_octal(&mut header[116..124], 0);
    // size (offset 124, len 12)
    write_octal(&mut header[124..136], evil.len() as u64);
    // mtime (offset 136, len 12)
    write_octal(&mut header[136..148], 0);
    // typeflag (offset 156): '0' = regular file
    header[156] = b'0';
    // ustar magic (offset 257) + version (263)
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    // checksum (offset 148, len 8): spaces while computing
    for b in &mut header[148..156] {
        *b = b' ';
    }
    let sum: u32 = header.iter().map(|&b| b as u32).sum();
    // 6 octal digits, NUL, space
    let cksum = format!("{sum:06o}\0 ");
    header[148..156].copy_from_slice(cksum.as_bytes());

    let mut tar_bytes = Vec::new();
    tar_bytes.extend_from_slice(&header);
    tar_bytes.extend_from_slice(evil);
    // Pad data to a 512-byte boundary, then two zero blocks to end the archive.
    let pad = (512 - (evil.len() % 512)) % 512;
    tar_bytes.extend(std::iter::repeat_n(0u8, pad));
    tar_bytes.extend(std::iter::repeat_n(0u8, 1024));

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&tar_bytes).unwrap();
    encoder.finish().unwrap()
}

/// Write a value as NUL-terminated octal ASCII into `field`.
fn write_octal(field: &mut [u8], value: u64) {
    let digits = field.len() - 1;
    let s = format!("{value:0>width$o}", width = digits);
    field[..digits].copy_from_slice(&s.as_bytes()[..digits]);
    field[digits] = 0;
}

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const MANIFEST: &[u8] = b"[plugin]\nid = \"atlas\"\nversion = \"1.2.0\"\nruntime = \"lua\"\n";

fn commits_route() -> (String, Canned) {
    let body = serde_json::to_vec(&serde_json::json!({ "sha": SHA })).unwrap();
    (
        "/repos/example/rpp-plugins/commits/v1.2.0".to_string(),
        Canned {
            status: 200,
            content_type: "application/json",
            body,
        },
    )
}

fn tarball_route(body: Vec<u8>) -> (String, Canned) {
    (
        format!("/example/rpp-plugins/tar.gz/{SHA}"),
        Canned {
            status: 200,
            content_type: "application/gzip",
            body,
        },
    )
}

#[test]
fn resolves_github_extracts_and_returns_subdir() {
    let top = format!("rpp-plugins-{SHA}");
    let tarball = build_tarball(
        &top,
        &[
            ("README.md", b"# repo"),
            ("plugins/atlas/plugin.toml", MANIFEST),
            ("plugins/atlas/init.lua", b"-- atlas"),
        ],
    );

    let mut routes = HashMap::new();
    let (k, v) = commits_route();
    routes.insert(k, v);
    let (k, v) = tarball_route(tarball);
    routes.insert(k, v);

    let server = MockServer::start(routes);
    let cache = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    let resolver = Resolver::new(project.path())
        .unwrap()
        .with_cache_root(cache.path())
        .with_http_config(server.config());

    let source = PluginSource::parse(
        "github:example/rpp-plugins",
        Some("v1.2.0"),
        Some("plugins/atlas"),
    )
    .unwrap();

    let resolved = resolver.resolve(&source, None).unwrap();

    // Extraction layout: <cache>/github/example/rpp-plugins/<sha>/...
    let expected_root = cache
        .path()
        .join("github")
        .join("example")
        .join("rpp-plugins")
        .join(SHA)
        .join("plugins/atlas");
    assert_eq!(resolved.root, expected_root);
    assert!(resolved.root.join("plugin.toml").is_file());
    assert!(resolved.root.join("init.lua").is_file());

    let pin = resolved.pinned.expect("github source should be pinned");
    assert_eq!(pin.ref_, "v1.2.0");
    assert_eq!(pin.commit, SHA);

    // Manifest summary reads id/version from the resolved dir.
    let summary = rpp_fetch::parse_manifest_summary(&resolved.root).unwrap();
    assert_eq!(summary.id, "atlas");
    assert_eq!(summary.version, "1.2.0");
}

#[test]
fn pin_short_circuit_makes_zero_network_calls() {
    let top = format!("rpp-plugins-{SHA}");
    let tarball = build_tarball(&top, &[("plugin.toml", MANIFEST)]);

    let mut routes = HashMap::new();
    let (k, v) = commits_route();
    routes.insert(k, v);
    let (k, v) = tarball_route(tarball);
    routes.insert(k, v);

    let server = MockServer::start(routes);
    let cache = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    let resolver = Resolver::new(project.path())
        .unwrap()
        .with_cache_root(cache.path())
        .with_http_config(server.config());

    let source = PluginSource::parse("github:example/rpp-plugins", Some("v1.2.0"), None).unwrap();

    // First resolution: warms the cache (some network hits).
    resolver.resolve(&source, None).unwrap();
    assert!(server.hits() > 0);

    let warm = server.hits();

    // Now supply a lock pinning the cached commit. Must do zero further hits.
    let lock = LockedPlugin {
        source: "github:example/rpp-plugins".to_string(),
        ref_: "v1.2.0".to_string(),
        commit: SHA.to_string(),
        subdir: None,
    };
    let resolved = resolver.resolve(&source, Some(&lock)).unwrap();
    assert_eq!(
        server.hits(),
        warm,
        "pin short-circuit must not hit network"
    );
    assert!(resolved.root.join("plugin.toml").is_file());
    assert_eq!(resolved.pinned.unwrap().commit, SHA);
}

#[test]
fn rejects_path_traversal_in_tarball() {
    let top = format!("rpp-plugins-{SHA}");
    let evil = build_evil_tarball(&top);

    let mut routes = HashMap::new();
    let (k, v) = commits_route();
    routes.insert(k, v);
    let (k, v) = tarball_route(evil);
    routes.insert(k, v);

    let server = MockServer::start(routes);
    let cache = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    let resolver = Resolver::new(project.path())
        .unwrap()
        .with_cache_root(cache.path())
        .with_http_config(server.config());

    let source = PluginSource::parse("github:example/rpp-plugins", Some("v1.2.0"), None).unwrap();
    let err = resolver.resolve(&source, None).unwrap_err();
    assert!(
        matches!(err, rpp_fetch::Error::UnsafeTarEntry(_)),
        "expected UnsafeTarEntry, got {err:?}"
    );

    // The escaping file must not have been written anywhere.
    assert!(!cache.path().join("escape.txt").exists());
    assert!(!cache.path().parent().unwrap().join("escape.txt").exists());
}

#[test]
fn search_against_mock() {
    let body = serde_json::to_vec(&serde_json::json!({
        "items": [
            {
                "name": "rpp-plugins",
                "full_name": "example/rpp-plugins",
                "description": "Atlas + minify",
                "stargazers_count": 7,
                "html_url": "https://github.com/example/rpp-plugins"
            }
        ]
    }))
    .unwrap();

    let mut routes = HashMap::new();
    routes.insert(
        "/search/repositories".to_string(),
        Canned {
            status: 200,
            content_type: "application/json",
            body,
        },
    );

    let server = MockServer::start(routes);
    let hits = rpp_fetch::search_with_config("atlas", server.config()).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "rpp-plugins");
    assert_eq!(hits[0].full_name, "example/rpp-plugins");
    assert_eq!(hits[0].stars, 7);
    assert_eq!(hits[0].description.as_deref(), Some("Atlas + minify"));
}

#[test]
fn path_source_resolves_against_project_root() {
    let project = tempfile::tempdir().unwrap();
    let plugin_dir = project.path().join("plugins/json-minify");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.toml"), MANIFEST).unwrap();

    let resolver = Resolver::new(project.path()).unwrap();
    let source = PluginSource::parse("path:plugins/json-minify", None, None).unwrap();
    let resolved = resolver.resolve(&source, None).unwrap();

    assert_eq!(resolved.root, plugin_dir);
    assert!(resolved.pinned.is_none());
}

#[test]
fn path_source_missing_manifest_errors() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("plugins/empty")).unwrap();

    let resolver = Resolver::new(project.path()).unwrap();
    let source = PluginSource::parse("path:plugins/empty", None, None).unwrap();
    assert!(matches!(
        resolver.resolve(&source, None),
        Err(rpp_fetch::Error::MissingManifest(_))
    ));
}

#[test]
fn lockfile_round_trip_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rpp.lock");

    let mut lock = Lockfile::new();
    lock.upsert(LockedPlugin {
        source: "github:example/rpp-plugins".to_string(),
        ref_: "v1.2.0".to_string(),
        commit: SHA.to_string(),
        subdir: Some("plugins/atlas".to_string()),
    });
    lock.save(&path).unwrap();

    let loaded = Lockfile::load(&path).unwrap();
    let pin = loaded.get("github:example/rpp-plugins").unwrap();
    assert_eq!(pin.commit, SHA);
    assert_eq!(pin.subdir.as_deref(), Some("plugins/atlas"));
}

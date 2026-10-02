//! Tests for installing a packed plugin archive from a registry.

mod common;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde_json::json;

use common::packed_plugin::{pack, plugin};
use common::{stderr, write};

/// A static HTTP server on a loopback port.
struct Server {
    base: String,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (routes, stop) = (Arc::clone(&routes), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => respond(stream, &routes),
                        Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                    }
                }
            })
        };
        Server {
            base,
            routes,
            stop,
            handle: Some(handle),
        }
    }

    fn serve(&self, path: &str, body: Vec<u8>) {
        self.routes.lock().unwrap().insert(path.to_string(), body);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

fn respond(mut stream: TcpStream, routes: &Mutex<HashMap<String, Vec<u8>>>) {
    stream.set_nonblocking(false).unwrap();
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&head);
    let path = head.split_whitespace().nth(1).unwrap_or("");
    let body = routes.lock().unwrap().get(path).cloned();
    let (status, body) = match body {
        Some(body) => ("200 OK", body),
        None => ("404 Not Found", Vec::new()),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(&body);
}

/// Packs the plugin, serves it from a registry, and returns a project depending on it.
fn consumer(dir: &Path, server: &Server, source: &str) {
    let plugin_dir = dir.join("plugin");
    plugin(&plugin_dir);
    let packed = pack(&plugin_dir, "out");
    let archive = std::fs::read(plugin_dir.join("out/packed-1.2.3.rpp.tgz")).unwrap();
    server.serve("/archives/packed-1.2.3.rpp.tgz", archive);
    server.serve(
        "/plugins/packed.json",
        serde_json::to_vec(&json!({
            "name": "packed",
            "repository": "https://example.com/packed",
            "description": "A packed plugin",
            "versions": [{
                "version": "1.2.3",
                "url": format!("{}/archives/packed-1.2.3.rpp.tgz", server.base),
                "sha256": packed["sha256"],
                "rpp": ">=0.1",
            }],
        }))
        .unwrap(),
    );

    let project = dir.join("project");
    write(
        &project,
        "rpp.json",
        r#"{ "dependencies": { "packed": "^1.2.3" } }"#,
    );
    write(
        &project,
        "rpp.config.ts",
        r##"import { defineConfig } from "#rpp/config";
import packed from "#plugins/packed";

export default defineConfig({
  pack: { name: "p" },
  plugins: [packed({ text: "x" })],
});
"##,
    );
    write(&project, "src/a.txt", source);
}

#[test]
fn packed_plugin_installs_from_registry() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start();
    consumer(dir.path(), &server, "hello");
    let project = dir.path().join("project");

    let out = common::command(&project)
        .env("RPP_REGISTRY", &server.base)
        .args(["build", "--no-squash"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(project.join("dist/a.txt")).unwrap(),
        "hello!"
    );
}

#[test]
fn packed_plugin_error_maps_to_original_source() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start();
    consumer(dir.path(), &server, "explode");
    let project = dir.path().join("project");

    let out = common::command(&project)
        .env("RPP_REGISTRY", &server.base)
        .args(["build", "--no-squash"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    let message = stderr(&out);
    assert!(message.contains("boom from helper"), "{message}");
    assert!(message.contains("src/helper.ts:3:"), "{message}");
    assert!(!message.contains("dist/"), "{message}");
}

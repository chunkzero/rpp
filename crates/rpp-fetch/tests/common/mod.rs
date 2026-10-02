//! Mock HTTP server shared by the integration tests.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use rpp_fetch::HttpConfig;

/// A canned response for a given request path.
#[derive(Clone)]
pub struct Canned {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

/// A minimal single-threaded mock HTTP server. Maps request path (ignoring the
/// query string for API routes, but matching exactly for the search route via a
/// prefix) to a canned response, and counts total requests served.
pub struct MockServer {
    base: String,
    routes: Arc<Mutex<HashMap<String, Canned>>>,
    hits: Arc<AtomicUsize>,
    shutdown: Arc<Mutex<bool>>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    pub fn start(routes: HashMap<String, Canned>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr");
        let base = format!("http://{addr}");

        let hits = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(Mutex::new(false));

        let hits_t = Arc::clone(&hits);
        let shutdown_t = Arc::clone(&shutdown);
        let routes = Arc::new(Mutex::new(routes));
        let routes_t = Arc::clone(&routes);

        let handle = std::thread::spawn(move || loop {
            if *shutdown_t.lock().unwrap() {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    handle_conn(stream, &routes_t, &hits_t);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => break,
            }
        });

        MockServer {
            base,
            routes,
            hits,
            shutdown,
            handle: Some(handle),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Add or replace the response for `path`.
    pub fn set(&self, path: impl Into<String>, canned: Canned) {
        self.routes.lock().unwrap().insert(path.into(), canned);
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub fn config(&self) -> HttpConfig {
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

fn handle_conn(mut stream: TcpStream, routes: &Mutex<HashMap<String, Canned>>, hits: &AtomicUsize) {
    stream.set_nonblocking(false).ok();
    let target = read_request_target(&mut stream);
    hits.fetch_add(1, Ordering::SeqCst);
    let response = find_route(&routes.lock().unwrap(), &target);
    write_response(&mut stream, &response);
}

/// Reads the request head (until CRLFCRLF) and returns the request-line target.
fn read_request_target(stream: &mut TcpStream) -> String {
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
    parts.next().unwrap_or("/").to_string()
}

/// Matches the exact target, then the path without its query; the search route also matches
/// by prefix. Unmatched targets get a 404.
fn find_route(routes: &HashMap<String, Canned>, target: &str) -> Canned {
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

    match canned {
        Some(c) => c.clone(),
        None => Canned {
            status: 404,
            content_type: "text/plain",
            body: b"not found".to_vec(),
        },
    }
}

fn write_response(stream: &mut TcpStream, response: &Canned) {
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

//! Integration tests for the HTTP↔JRG gateway crate.
//!
//! These tests spin up real network services: SearchEngine, HTTP→JRG gateway,
//! JrgToHttpResolver, and a tiny HTTP server, then communicate over the wire
//! to verify end-to-end behaviour.

use std::io::{Read, Write};
use std::net::TcpListener as StdTcpListener;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find a free TCP port on localhost by binding to port 0, reading the
/// assigned port, then releasing the socket.  A tiny race window exists
/// (between releasing and the actual bind) but is negligible in tests.
fn find_free_port() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Start a SearchEngine JRG TCP server on a random port and return the port.
/// The server runs in a background thread and accepts connections forever.
fn start_search_engine(dir: &TempDir) -> u16 {
    let port = find_free_port();
    let engine = Arc::new(jaringan_search::SearchEngine::new(
        dir.path().join("search-data"),
        "search.localhost".to_string(),
        port,
    ));

    let listener = StdTcpListener::bind(("127.0.0.1", port)).unwrap();
    std::thread::spawn(move || {
        let _ = jaringan_protocol::serve(listener, engine);
    });

    port
}

/// Start the HTTP→JRG gateway (axum HTTP server) on a random port, backed by
/// the given JRG host, and return the HTTP port.
fn start_http_gateway(jrg_host_port: u16) -> u16 {
    let port = find_free_port();

    let config = jaringan_gateway::HttpToJrgGatewayConfig {
        listen_addr: format!("127.0.0.1:{port}"),
        jrg_host: format!("127.0.0.1:{jrg_host_port}"),
        ..Default::default()
    };
    let gateway = jaringan_gateway::HttpToJrgGateway::new(config);

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(gateway.serve()).unwrap();
    });

    port
}

/// Start a bare-bones HTTP server that serves a fixed body on every request.
/// Returns the port it is listening on.
fn start_tiny_http_server(body: &str) -> u16 {
    let port = find_free_port();
    let raw_body = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let body_bytes = raw_body.as_bytes().to_vec();

    let listener = StdTcpListener::bind(("127.0.0.1", port)).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&body_bytes);
            let _ = stream.flush();
        }
    });

    port
}

/// Start a JrgToHttpResolver JRG TCP server on a random port and return the
/// port.
fn start_jrg_http_resolver() -> u16 {
    let port = find_free_port();

    let config = jaringan_gateway::JrgToHttpResolverConfig {
        timeout_secs: 5,
        ..Default::default()
    };
    let resolver = jaringan_gateway::JrgToHttpResolver::new(config);

    let listener = StdTcpListener::bind(("127.0.0.1", port)).unwrap();
    std::thread::spawn(move || {
        let _ = jaringan_protocol::serve(listener, resolver);
    });

    port
}

/// Send a raw JRG wire-format request over TCP and return the full response
/// text. This is needed when the JRG URL's host part doesn't match the TCP
/// target (e.g. `jrg://http/…` where "http" isn't a real hostname).
fn raw_jrg_fetch(jrg_host: &str, jrg_port: u16, request_url: &str) -> String {
    let mut stream = TcpStream::connect((jrg_host, jrg_port))
        .unwrap_or_else(|_| panic!("could not connect to 127.0.0.1:{jrg_port}"));
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();

    // Write a minimal JRG request (GET with the given URL)
    let wire = format!(
        "GET {request_url} JRG/0.1\r\nHost: {jrg_host}\r\nContent-Length: 0\r\n\r\n"
    );
    stream.write_all(wire.as_bytes()).unwrap();
    stream.flush().unwrap();

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

/// Give background servers time to start listening.
fn wait_for_services() {
    std::thread::sleep(Duration::from_millis(400));
}

// ===========================================================================
// Test 1: HTTP→JRG gateway starts and serves a page
// ===========================================================================

#[test]
fn test_http_gateway_serves_search_page() {
    let dir = tempfile::tempdir().unwrap();

    // Start services
    let search_port = start_search_engine(&dir);
    let gateway_port = start_http_gateway(search_port);
    wait_for_services();

    // Fetch /search.jrg via the HTTP gateway using blocking reqwest
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let resp = client
        .get(format!("http://127.0.0.1:{gateway_port}/search.jrg"))
        .send()
        .expect("HTTP GET /search.jrg should succeed");
    assert!(
        resp.status().is_success(),
        "expected 2xx, got {}",
        resp.status()
    );
    let body = resp.text().unwrap();

    assert!(
        body.contains("JRG Search") || body.contains("Search"),
        "response should contain 'JRG Search' or 'Search', got: {body:.200}"
    );
    assert!(
        body.starts_with("<!DOCTYPE html>"),
        "response should start with <!DOCTYPE html>, got: {body:.200}"
    );
}

// ===========================================================================
// Test 2: HTTP→JRG gateway returns 404 for unknown paths
// ===========================================================================

#[test]
fn test_http_gateway_returns_404_for_unknown_path() {
    let dir = tempfile::tempdir().unwrap();

    let search_port = start_search_engine(&dir);
    let gateway_port = start_http_gateway(search_port);
    wait_for_services();

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let resp = client
        .get(format!(
            "http://127.0.0.1:{gateway_port}/nonexistent.jrg"
        ))
        .send()
        .expect("HTTP GET /nonexistent.jrg should not crash the gateway");
    let status = resp.status();
    let body = resp.text().unwrap();

    // The search engine returns a NotFound status JRG page for unknown paths.
    // The gateway renders that as HTML, so we check for 404-related content.
    assert!(
        status.is_server_error()
            || status.is_client_error()
            || body.contains("Not Found")
            || body.contains("404")
            || body.contains("not found"),
        "expected a 4xx/5xx status or 404/Not Found text in body, got status={status} body={body:.200}"
    );
}

// ===========================================================================
// Test 3: JRG→HTTP resolver fetches a URL
// ===========================================================================

#[test]
fn test_jrg_to_http_resolver_fetches_url() {
    // Start a tiny HTTP server that serves "Hello World"
    let http_port = start_tiny_http_server("Hello World");

    // Start the JrgToHttpResolver
    let jrg_port = start_jrg_http_resolver();
    wait_for_services();

    // Connect to the JRG server and send a request targeting
    // jrg://http/127.0.0.1:HTTP_PORT/some-path
    let response_text = raw_jrg_fetch(
        "127.0.0.1",
        jrg_port,
        &format!("jrg://http/127.0.0.1:{http_port}/some-path"),
    );

    assert!(
        response_text.contains("Hello World"),
        "JRG→HTTP response should contain 'Hello World', got: {response_text:.300}"
    );
}

// ===========================================================================
// Test 4: JRG→HTTP resolver handles timeouts gracefully
// ===========================================================================

#[test]
fn test_jrg_to_http_resolver_handles_timeout_gracefully() {
    let jrg_port = start_jrg_http_resolver();
    wait_for_services();

    // Try to fetch a non-existent host — this should not panic
    let response_result = std::panic::catch_unwind(|| {
        raw_jrg_fetch("127.0.0.1", jrg_port, "jrg://http/192.0.2.1:9999/test")
    });

    // The test passes as long as we don't panic — the server should
    // gracefully close the connection (empty response) or return an
    // error status, but never crash.
    match response_result {
        Ok(text) => {
            // An empty response (connection closed without data) is acceptable
            // because the server drops the connection gracefully on error.
            // A non-empty response should indicate an error.
            if !text.is_empty() {
                let has_error = text.contains("JRG/0.1 4")
                    || text.contains("JRG/0.1 5")
                    || text.contains("error")
                    || text.contains("Error")
                    || text.contains("timeout")
                    || text.contains("Timeout")
                    || text.contains("timed out")
                    || text.contains("connection refused")
                    || text.contains("Connection refused")
                    || text.contains("failed to connect")
                    || text.contains("resolve");
                assert!(
                    has_error,
                    "expected error indication in response, got: {text:.300}"
                );
            }
        }
        Err(e) => {
            // A panic during a graceful fetch would be a real failure
            panic!("server panicked on unreachable host: {e:?}");
        }
    }
}

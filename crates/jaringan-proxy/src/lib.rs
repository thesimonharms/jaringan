//! Jaringan vhost reverse proxy
//!
//! Forwards raw bytes between a single listening port and a backend chosen by
//! the request's `Host:` header. The proxy is protocol-agnostic — it only
//! inspects the `Host:` header; upstream services do their own protocol
//! detection.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

pub type RouteMap = HashMap<String, String>;

/// Parse `host=addr` pairs into a map. Hosts are lowercased.
pub fn parse_routes(routes: &[String]) -> RouteMap {
    let mut map = HashMap::new();
    for raw in routes {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        if let Some((host, addr)) = raw.split_once('=') {
            map.insert(host.trim().to_lowercase(), addr.trim().to_string());
        }
    }
    map
}

/// Run the proxy on the given bind address.
pub fn serve(
    bind: &str,
    routes: RouteMap,
    allow_unknown: bool,
    default: &str,
    timeout: Duration,
    max_head_size: usize,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(bind)?;
    let default = default.to_string();
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let routes = routes.clone();
                let default = default.clone();
                thread::spawn(move || {
                    let peer = stream.peer_addr().ok();
                    if let Err(e) = handle_connection(
                        stream,
                        &routes,
                        allow_unknown,
                        &default,
                        timeout,
                        max_head_size,
                    ) {
                        eprintln!("⚠️  [{peer:?}] connection error: {e}");
                    }
                });
            }
            Err(e) => {
                eprintln!("⚠️  accept error: {e}");
            }
        }
    }
    Ok(())
}

/// Handle a single client connection. Public for integration tests.
pub fn handle_connection(
    client: TcpStream,
    routes: &RouteMap,
    allow_unknown: bool,
    default: &str,
    timeout: Duration,
    max_head_size: usize,
) -> std::io::Result<()> {
    let peer = client.peer_addr().ok();
    client.set_read_timeout(Some(timeout))?;
    client.set_write_timeout(Some(timeout))?;

    // 1. Buffer the ENTIRE request (head + body) before connecting upstream.
    //    This way a bad client that disconnects mid-body never reaches a
    //    backend — we handle the failure ourselves.
    let (request_head, request_body, max_hit) =
        read_full_request(&client, max_head_size)?;

    if max_hit {
        // Head exceeded the cap — tell the client and bail before
        // we touch any backend.
        let _ = write_error_response(
            &client,
            413,
            "Payload Too Large",
            "request head exceeds size limit",
        );
        return Ok(());
    }

    // 2. Resolve backend
    let host = extract_host(&request_head)
        .map(|h| h.to_lowercase())
        .unwrap_or_default();
    let backend = routes
        .get(&host)
        .cloned()
        .or_else(|| allow_unknown.then(|| default.to_string()));

    let Some(backend) = backend else {
        eprintln!("⚠️  [{peer:?}] rejected unknown host \"{host}\"");
        let _ = write_error_response(
            &client,
            503,
            "Service Unavailable",
            "service not available",
        );
        return Ok(());
    };

    // If Content-Length is larger than the body we actually received,
    // the client disconnected early. Don't forward to upstream.
    if let Some(claimed) = parse_content_length(&request_head) {
        if request_body.len() < claimed {
            eprintln!(
                "⚠️  [{peer:?}] client sent incomplete body (Content-Length={claimed}, got={})",
                request_body.len()
            );
            let _ = write_error_response(
                &client,
                400,
                "Bad Request",
                "incomplete request body",
            );
            return Ok(());
        }
    }

    // 3. Connect to backend
    let mut upstream = match TcpStream::connect(&backend) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("⚠️  [{peer:?}] upstream connect failed for \"{host}\": {e}");
            let _ = write_error_response(&client, 502, "Bad Gateway", "service unavailable");
            return Ok(());
        }
    };
    upstream.set_read_timeout(Some(timeout))?;
    upstream.set_write_timeout(Some(timeout))?;

    // 4. Forward the complete, validated request
    upstream.write_all(&request_head)?;
    if !request_body.is_empty() {
        upstream.write_all(&request_body)?;
    }
    upstream.shutdown(std::net::Shutdown::Write).ok();

    // 5. Read response and forward to client
    let mut upstream_reader = BufReader::new(upstream);
    let response = read_full_response(&mut upstream_reader)?;
    let mut client = client;
    client.write_all(&response)?;
    Ok(())
}

/// Read the full request: head (up to blank line) + body (Content-Length bytes
/// or whatever arrives). Returns (head_bytes, body_bytes, head_limit_hit).
/// On head overflow, returns with max_hit=true and no further reads.
fn read_full_request(
    client: &TcpStream,
    max_head: usize,
) -> std::io::Result<(Vec<u8>, Vec<u8>, bool)> {
    let mut reader = BufReader::new(client);
    let mut head = Vec::new();

    // Read header section
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        if head.len() + line.len() > max_head {
            return Ok((head, Vec::new(), true));
        }
        head.extend_from_slice(line.as_bytes());
        if line == "\r\n" || line == "\n" {
            break;
        }
    }

    // Read body based on Content-Length
    let body_len = parse_content_length(&head).unwrap_or(0);
    // Cap body at a reasonable size to prevent OOM on malicious Content-Length
    let body_len = body_len.min(max_head * 4);

    let mut body = vec![0u8; body_len];
    if body_len > 0 {
        match reader.read_exact(&mut body) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Client closed before sending the full body — truncate
                // but return what we have; the caller validates the
                // Content-Length match.
                return Ok((head, Vec::new(), false));
            }
            Err(e) => return Err(e),
        }
    }

    Ok((head, body, false))
}

/// Extract the host portion of the `Host:` header (stripping any `:port`).
pub fn extract_host(head: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(head).ok()?;
    for line in text.lines() {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case("host") {
                let v = value.trim();
                let host = v.rsplit_once(':').map(|(h, _)| h).unwrap_or(v);
                return Some(host.to_string());
            }
        }
    }
    None
}

fn parse_content_length(head: &[u8]) -> Option<usize> {
    parse_content_length_from_text(std::str::from_utf8(head).ok()?)
}

fn parse_content_length_from_text(text: &str) -> Option<usize> {
    for line in text.lines() {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case("content-length") {
                return value.trim().parse().ok();
            }
        }
    }
    None
}

fn read_full_response(reader: &mut BufReader<TcpStream>) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(line.as_bytes());
        if line == "\r\n" || line == "\n" {
            break;
        }
    }

    let header_text = std::str::from_utf8(&out).unwrap_or("");
    if let Some(len) = parse_content_length_from_text(header_text) {
        let mut remaining = len;
        let mut buf = [0u8; 4096];
        while remaining > 0 {
            let to_read = buf.len().min(remaining);
            match reader.read(&mut buf[..to_read]) {
                Ok(0) => break,
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    remaining -= n;
                }
                Err(e) => return Err(e),
            }
        }
    } else {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) => return Err(e),
            }
        }
    }

    Ok(out)
}

fn write_error_response(
    client: &TcpStream,
    status: u16,
    reason: &str,
    body: &str,
) -> std::io::Result<()> {
    let mut client = client;
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    client.write_all(response.as_bytes())
}
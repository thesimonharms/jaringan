use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

use jaringan_proxy::{extract_host, handle_connection, parse_routes, RouteMap};

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HEAD: usize = 65536;

// ---------------------------------------------------------------------------
// Pure-function tests
// ---------------------------------------------------------------------------

#[test]
fn routes_by_host_header() {
    let routes = parse_routes(&[
        "search.simonharms.xyz=127.0.0.1:9001".into(),
        "jrg.simonharms.xyz=127.0.0.1:9002".into(),
    ]);
    assert_eq!(routes.get("search.simonharms.xyz"), Some(&"127.0.0.1:9001".into()));
    assert_eq!(routes.get("jrg.simonharms.xyz"), Some(&"127.0.0.1:9002".into()));
    assert!(routes.get("other.example.com").is_none());
}

#[test]
fn routes_are_case_insensitive() {
    let routes = parse_routes(&["Search.SimonHarms.XYZ=127.0.0.1:9001".into()]);
    assert!(routes.contains_key("search.simonharms.xyz"));
    assert!(!routes.contains_key("SEARCH.SIMONHARMS.XYZ"));
}

#[test]
fn malformed_routes_are_skipped() {
    let routes = parse_routes(&["valid.example=127.0.0.1:9001".into(), "no-equals-sign".into()]);
    assert_eq!(routes.len(), 1);
    assert!(routes.contains_key("valid.example"));
}

#[test]
fn host_extraction_strips_port() {
    assert_eq!(extract_host(b"GET / JRG/0.1\r\nHost: example.com:7070\r\n\r\n"), Some("example.com".into()));
}

#[test]
fn host_extraction_works_without_port() {
    assert_eq!(extract_host(b"GET / JRG/0.1\r\nHost: example.com\r\n\r\n"), Some("example.com".into()));
}

#[test]
fn host_extraction_is_case_insensitive() {
    assert_eq!(extract_host(b"GET / JRG/0.1\r\nhost: example.com\r\n\r\n"), Some("example.com".into()));
}

// ---------------------------------------------------------------------------
// End-to-end tests
// ---------------------------------------------------------------------------

fn spawn_echo_backend() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(TIMEOUT));
            let mut buf = [0u8; 4096];
            let n = match stream.read(&mut buf) { Ok(n) => n, Err(_) => continue };
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let host_line = request.lines().find(|l| l.to_lowercase().starts_with("host:")).unwrap_or("");
            let response_body = format!("# Backend Reached\n\nhost_was=`{host_line}`\n\n~~~\ntitle: Echo\n~~~");
            let response = format!(
                "JRG/0.1 200 OK\r\nContent-Type: text/jrg; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                response_body.len(), response_body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    port
}

fn spawn_proxy(routes: RouteMap, allow_unknown: bool, default: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for incoming in listener.incoming().flatten() {
            let r = routes.clone();
            let d = default.clone();
            thread::spawn(move || { let _ = handle_connection(incoming, &r, allow_unknown, &d, TIMEOUT, MAX_HEAD); });
        }
    });
    port
}

fn send_request(proxy_port: u16, host: &str) -> String {
    let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).unwrap();
    let req = format!("GET / JRG/0.1\r\nHost: {host}\r\n\r\n");
    client.write_all(req.as_bytes()).unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut buf = String::new();
    client.read_to_string(&mut buf).unwrap();
    buf
}

#[test]
fn proxy_routes_two_hosts_to_different_backends() {
    let a = spawn_echo_backend();
    let b = spawn_echo_backend();
    let routes = parse_routes(&[format!("a.test=127.0.0.1:{a}"), format!("b.test=127.0.0.1:{b}")]);
    let proxy = spawn_proxy(routes, true, format!("127.0.0.1:{a}"));

    let ra = send_request(proxy, "a.test:7070");
    assert!(ra.contains("host_was=`Host: a.test:7070`"), "Got: {ra}");

    let rb = send_request(proxy, "b.test:7070");
    assert!(rb.contains("host_was=`Host: b.test:7070`"), "Got: {rb}");
}

#[test]
fn unmatched_host_uses_default_when_allowed() {
    let b = spawn_echo_backend();
    let proxy = spawn_proxy(parse_routes(&[]), true, format!("127.0.0.1:{b}"));
    let resp = send_request(proxy, "random.example.com");
    assert!(resp.contains("# Backend Reached"), "Got: {resp}");
}

#[test]
fn unmatched_host_rejected_when_disallowed() {
    let proxy = spawn_proxy(parse_routes(&[]), false, "127.0.0.1:1".into());
    let resp = send_request(proxy, "random.example.com");
    assert!(resp.contains("Service Unavailable") || resp.contains("503"), "Got: {resp}");
}

#[test]
fn proxy_forwards_post_body() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let bport = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let _ = s.set_read_timeout(Some(TIMEOUT));
            let mut head = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                if s.read(&mut byte).unwrap_or(0) == 0 { break; }
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") { break; }
            }
            let hs = String::from_utf8_lossy(&head);
            let cl: usize = hs.lines()
                .find_map(|l| l.split_once(':').and_then(|(k,v)| k.trim().eq_ignore_ascii_case("content-length").then(|| v.trim().parse().ok())).flatten())
                .unwrap_or(0);
            let mut body = vec![0u8; cl];
            let _ = s.read_exact(&mut body);
            let bt = String::from_utf8_lossy(&body);
            let rb = format!("body=`{bt}`");
            let r = format!("JRG/0.1 200 OK\r\nContent-Type: text/jrg; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}", rb.len(), rb);
            let _ = s.write_all(r.as_bytes());
        }
    });
    let routes = parse_routes(&[format!("x.test=127.0.0.1:{bport}")]);
    let proxy = spawn_proxy(routes, true, format!("127.0.0.1:{bport}"));

    let body = "domain=example.com";
    let mut client = TcpStream::connect(("127.0.0.1", proxy)).unwrap();
    let req = format!("POST /actions/submit JRG/0.1\r\nHost: x.test:7070\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    client.write_all(req.as_bytes()).unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut buf = String::new();
    client.read_to_string(&mut buf).unwrap();
    assert!(buf.contains("body=`domain=example.com`"), "Got: {buf}");
}
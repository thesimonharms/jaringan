//! HTTP→JRG Gateway
//!
//! An HTTP server (axum) that accepts HTTP requests and forwards them to
//! a JRG resolver (local file or remote TCP server), returning the JRG
//! response rendered as HTML.
//!
//! # Usage
//!
//! ```rust,no_run
//! use jaringan_gateway::{HttpToJrgGateway, HttpToJrgGatewayConfig};
//!
//! # async fn example() {
//! let gateway = HttpToJrgGateway::new(HttpToJrgGatewayConfig {
//!     listen_addr: "127.0.0.1:8080".to_string(),
//!     jrg_host: "127.0.0.1:7070".to_string(),
//!     enable_http_bridge: false,
//!     ..Default::default()
//! });
//! gateway.serve().await.unwrap();
//! # }
//! ```

use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderValue, Method, StatusCode as HttpStatus, header},
    middleware::{self, Next},
    response::{
        Html,
        IntoResponse,
        Response,
        sse::{Event, Sse},
    },
    routing::get,
};
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{GatewayError, jrg_to_html};
use jaringan_protocol::{
    JaringanUrl,
    ResponseTag,
    fetch_tcp_stream_with_timeout,
    fetch_tcp_with_timeout, post_tcp,
};

/// Configuration for the HTTP→JRG gateway.
#[derive(Debug, Clone)]
pub struct HttpToJrgGatewayConfig {
    /// Address to listen on (e.g., `127.0.0.1:8080`).
    pub listen_addr: String,
    /// Default JRG host to proxy requests to (e.g., `127.0.0.1:7070`).
    /// Used when the path doesn't contain an explicit `jrg://` target.
    pub jrg_host: String,
    /// If true, also serve an HTTP bridge at `/http/*` that lets JRG clients
    /// fetch arbitrary HTTP URLs via this gateway (complementary to JrgToHttpResolver).
    pub enable_http_bridge: bool,
    /// If true, streaming JRG responses (`Tag-Stream: true`) are forwarded as
    /// Server-Sent Events (SSE) rather than being buffered into a single HTML page.
    pub enable_streaming: bool,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Time-to-live for cached responses, in seconds. Default: 60.
    pub cache_ttl_secs: u64,
    /// Maximum number of entries in the response cache. Default: 128.
    pub cache_max_entries: usize,
}

impl Default for HttpToJrgGatewayConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".to_string(),
            jrg_host: "127.0.0.1:7070".to_string(),
            enable_http_bridge: false,
            enable_streaming: true,
            timeout_secs: 10,
            cache_ttl_secs: 60,
            cache_max_entries: 128,
        }
    }
}

/// A cached JRG response entry.
#[derive(Clone)]
struct CachedEntry {
    html: String,
    cached_at: Instant,
}

#[derive(Clone)]
struct AppState {
    config: Arc<HttpToJrgGatewayConfig>,
    cache: Arc<Mutex<HashMap<String, CachedEntry>>>,
}

/// The HTTP→JRG gateway server.
pub struct HttpToJrgGateway {
    config: HttpToJrgGatewayConfig,
}

impl HttpToJrgGateway {
    /// Create a new gateway with the given configuration.
    pub fn new(config: HttpToJrgGatewayConfig) -> Self {
        Self { config }
    }

    /// Start the HTTP server and block forever.
    pub async fn serve(self) -> Result<(), GatewayError> {
        let cache: Arc<Mutex<HashMap<String, CachedEntry>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let state = AppState {
            config: Arc::new(self.config),
            cache,
        };

        let router = Router::new()
            .route("/", get(root_handler))
            .route("/health", get(health_handler))
            .fallback(get(catch_all_handler).post(catch_all_handler))
            .layer(middleware::from_fn(cors_middleware));

        let listen_addr = state.config.listen_addr.clone();
        let jrg_host = state.config.jrg_host.clone();

        let router = router.with_state(state);

        let listener = tokio::net::TcpListener::bind(&listen_addr)
            .await
            .map_err(|e| GatewayError::Config(format!("bind failed: {e}")))?;

        eprintln!(
            "HTTP→JRG gateway listening on http://{listen_addr}"
        );
        eprintln!(
            "Proxying to JRG target: {jrg_host}"
        );

        axum::serve(listener, router)
            .await
            .map_err(|e| GatewayError::Config(format!("serve failed: {e}")))?;

        Ok(())
    }

    fn build_jrg_url(path: &str, config: &HttpToJrgGatewayConfig) -> Result<String, GatewayError> {
        let path = path.trim_start_matches('/');
        if path.starts_with("jrg://") {
            // Explicit jrg:// URL in path — pass through directly
            Ok(path.to_string())
        } else {
            // Map to the configured JRG host
            Ok(format!("jrg://{}/{}", config.jrg_host, path))
        }
    }
}

/// Root handler: show gateway status.
async fn root_handler(State(state): State<AppState>) -> Html<String> {
    let config = &state.config;
    Html(format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>JRG Gateway</title>
    <style>
        body {{ font-family: monospace; max-width: 800px; margin: 2rem auto; padding: 0 1rem; }}
        h1 {{ color: #333; }}
        .info {{ background: #f4f4f4; padding: 1rem; border-radius: 4px; }}
        .info dt {{ font-weight: bold; margin-top: 0.5rem; }}
        .info dd {{ margin-left: 1rem; }}
    </style>
</head>
<body>
    <h1>✦ JRG Gateway</h1>
    <div class="info">
        <dl>
            <dt>Target JRG Host</dt>
            <dd><code>jrg://{jrg_host}/</code></dd>
            <dt>HTTP Bridge</dt>
            <dd>{http_bridge}</dd>
        </dl>
    </div>
    <h2>Usage</h2>
    <ul>
        <li><a href="/proxy/jrg://{jrg_host}/">/proxy/jrg://{jrg_host}/</a> — explicit proxy to any JRG URL</li>
        <li><a href="/">/path/to/page.jrg</a> — implicit proxy to configured host</li>
    </ul>
</body>
</html>"#,
        jrg_host = config.jrg_host,
        http_bridge = if config.enable_http_bridge { "enabled" } else { "disabled" },
    ))
}

/// CORS middleware: add permissive CORS headers for public API access.
async fn cors_middleware(
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Authorization"),
    );
    response
}

/// Health check endpoint.
async fn health_handler() -> axum::response::Json<serde_json::Value> {
    axum::response::Json(serde_json::json!({
        "status": "ok",
        "service": "jaringan-gateway",
        "version": "0.1.0",
    }))
}

/// Catch-all handler: routes to proxy, bridge, or implicit path proxy.
async fn catch_all_handler(
    State(state): State<AppState>,
    uri: axum::http::Uri,
    method: Method,
    query: Query<HashMap<String, String>>,
    body: String,
) -> Response {
    // Get the path from the URI
    let path = uri.path().trim_start_matches('/');

    // Detect web URLs (http:// or https://) and auto-route through HTTP bridge
    if path.starts_with("http://") || path.starts_with("https://") {
        if !state.config.enable_http_bridge {
            return (HttpStatus::BAD_REQUEST, "HTTP bridge is disabled; enable with --enable-http-bridge").into_response();
        }
        return http_bridge_inner(path, &state).await;
    }

    // Detect inline jrg:// URLs as implicit proxy
    if path.starts_with("jrg://") {
        return fetch_via_jrg_and_respond(path, &method, &query, &body, &state.config, &state.cache).await;
    }

    // HTTP bridge: /http/*url
    if path.starts_with("http/") {
        if !state.config.enable_http_bridge {
            return (HttpStatus::NOT_FOUND, "HTTP bridge is disabled").into_response();
        }
        return http_bridge_inner(path.strip_prefix("http/").unwrap_or(""), &state).await;
    }

    // Explicit proxy: /proxy/jrg://... 
    if let Some(jrg_url) = path.strip_prefix("proxy/jrg://") {
        let jrg_url = format!("jrg://{jrg_url}");
        return fetch_via_jrg_and_respond(&jrg_url, &method, &query, &body, &state.config, &state.cache).await;
    }
    if path.starts_with("proxy/") {
        let jrg_url = format!("jrg://{}", path.strip_prefix("proxy/").unwrap_or(""));
        return fetch_via_jrg_and_respond(&jrg_url, &method, &query, &body, &state.config, &state.cache).await;
    }

    // Implicit: map path to configured JRG host
    let jrg_url = match HttpToJrgGateway::build_jrg_url(path, &state.config) {
        Ok(url) => url,
        Err(e) => return (HttpStatus::BAD_REQUEST, format!("Invalid path: {e}")).into_response(),
    };
    fetch_via_jrg_and_respond(&jrg_url, &method, &query, &body, &state.config, &state.cache).await
}

/// Fetch a JRG URL and return either HTML or SSE streaming response.
async fn fetch_via_jrg_and_respond(
    jrg_url: &str,
    http_method: &Method,
    query: &HashMap<String, String>,
    body: &str,
    config: &HttpToJrgGatewayConfig,
    cache: &Arc<Mutex<HashMap<String, CachedEntry>>>,
) -> Response {
    // Check cache first for GET requests
    if *http_method == Method::GET {
        let ttl = Duration::from_secs(config.cache_ttl_secs);
        let guard = cache.lock().unwrap();
        if let Some(entry) = guard.get(jrg_url) {
            if entry.cached_at.elapsed() < ttl {
                return Html(entry.html.clone()).into_response();
            }
        }
    }

    let url = match JaringanUrl::parse(jrg_url) {
        Ok(u) => u,
        Err(e) => {
            return (HttpStatus::BAD_REQUEST, format!("Invalid JRG URL '{jrg_url}': {e}")).into_response();
        }
    };

    let timeout = Duration::from_secs(config.timeout_secs);

    // Determine the effective body (append query params for GET)
    let request_body = if query.is_empty() {
        body.to_string()
    } else {
        let qs: Vec<String> = query
            .iter()
            .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
            .collect();
        if body.is_empty() {
            qs.join("&")
        } else {
            format!("{}&{}", body, qs.join("&"))
        }
    };

    // Try streaming path: GET-like methods only, when streaming is enabled
    if config.enable_streaming && matches!(*http_method, Method::GET | Method::HEAD | Method::OPTIONS) {
        match fetch_tcp_stream_with_timeout(&url, timeout) {
            Ok(mut stream_conn) => {
                if stream_conn.response.tags.contains(&ResponseTag::Stream) {
                    return sse_from_stream(stream_conn, request_body);
                }
                // Streaming tag not present — drain the stream connection and render as HTML.
                let mut full_body = stream_conn.response.body.clone();
                while let Ok(Some(block)) = stream_conn.read_block() {
                    full_body.push_str(&block);
                }
                let response = jaringan_protocol::Response {
                    status: stream_conn.response.status,
                    content_type: stream_conn.response.content_type,
                    tags: stream_conn.response.tags,
                    body: full_body,
                };
                let html = jrg_to_html(&response);
                return Html(html).into_response();
            }
            Err(e) => {
                // Fall through to non-streaming fetch as a fallback
                eprintln!("Stream fetch failed, falling back to regular fetch: {e}");
            }
        }
    }

    // Non-streaming path (or streaming disabled/fell back) — with caching
    match fetch_via_jrg_inner(&url, http_method, &request_body, timeout, jrg_url, config, cache) {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            let status = match &e {
                GatewayError::Config(_) => HttpStatus::BAD_REQUEST,
                _ => HttpStatus::BAD_GATEWAY,
            };
            (status, format!("Gateway error: {e}")).into_response()
        }
    }
}

/// Build an SSE response body that streams blocks from a streaming JRG connection.
fn sse_from_stream(
    mut stream_conn: jaringan_protocol::StreamConnection,
    _request_body: String,
) -> Response {
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);

    // Spawn a blocking task to read blocks from the TCP stream and forward them as SSE events
    tokio::task::spawn_blocking(move || {
        loop {
            match stream_conn.read_block() {
                Ok(Some(block)) => {
                    let event = Event::default()
                        .event("block")
                        .data(block);
                    if tx.blocking_send(Ok(event)).is_err() {
                        // Client disconnected
                        break;
                    }
                }
                Ok(None) => {
                    // Stream ended — send [DONE] marker
                    let done = Event::default().data("[DONE]");
                    let _ = tx.blocking_send(Ok(done));
                    break;
                }
                Err(e) => {
                    eprintln!("SSE stream read error: {e}");
                    break;
                }
            }
        }
    });

    let stream: ReceiverStream<Result<Event, Infallible>> = ReceiverStream::new(rx);
    Sse::new(stream)
        .into_response()
}

/// Internal: perform a JRG TCP fetch and return rendered HTML.
/// Caches successful GET responses in an in-memory cache keyed by JRG URL.
fn fetch_via_jrg_inner(
    url: &JaringanUrl,
    http_method: &Method,
    request_body: &str,
    timeout: Duration,
    jrg_url: &str,
    config: &HttpToJrgGatewayConfig,
    cache: &Arc<Mutex<HashMap<String, CachedEntry>>>,
) -> Result<String, GatewayError> {
    // For GET requests, check the cache first
    if *http_method == Method::GET {
        let ttl = Duration::from_secs(config.cache_ttl_secs);
        let guard = cache.lock().unwrap();
        if let Some(entry) = guard.get(jrg_url) {
            if entry.cached_at.elapsed() < ttl {
                return Ok(entry.html.clone());
            }
        }
    }

    let response = match *http_method {
        Method::GET | Method::HEAD | Method::OPTIONS => {
            fetch_tcp_with_timeout(url, timeout).map_err(GatewayError::JrgProtocol)?
        }
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE => {
            post_tcp(url, request_body.to_string()).map_err(GatewayError::JrgProtocol)?
        }
        _ => {
            return Err(GatewayError::Config(format!(
                "unsupported HTTP method: {http_method}"
            )));
        }
    };

    let html = jrg_to_html(&response);

    // Only cache successful (200 OK) GET responses
    if *http_method == Method::GET
        && response.status == jaringan_protocol::StatusCode::Ok
    {
        let mut guard = cache.lock().unwrap();

        // Clean stale entries if at capacity
        if guard.len() >= config.cache_max_entries {
            let now = Instant::now();
            guard.retain(|_, v| now.duration_since(v.cached_at) < Duration::from_secs(config.cache_ttl_secs));
        }

        guard.insert(
            jrg_url.to_string(),
            CachedEntry {
                html: html.clone(),
                cached_at: Instant::now(),
            },
        );
    }

    Ok(html)
}

/// HTTP bridge: fetch an HTTP(S) URL and return as JRG HTML.
async fn http_bridge_inner(
    http_path: &str,
    state: &AppState,
) -> Response {
    let resolved = if !http_path.starts_with("http://") && !http_path.starts_with("https://") {
        format!("https://{http_path}")
    } else {
        http_path.to_string()
    };

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(state.config.timeout_secs))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (HttpStatus::INTERNAL_SERVER_ERROR, format!("Failed to build HTTP client: {e}")).into_response();
        }
    };

    match client.get(&resolved).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let jrg_response = jaringan_protocol::Response::page(
                jaringan_protocol::StatusCode::from_u16(status).unwrap_or(jaringan_protocol::StatusCode::Ok),
                &body,
            );
            Html(jrg_to_html(&jrg_response)).into_response()
        }
        Err(e) => {
            (HttpStatus::BAD_GATEWAY, format!("HTTP fetch error: {e}")).into_response()
        }
    }
}

fn urlencode(s: &str) -> String {
    let mut result = String::new();
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => result.push_str("+"),
            _ => result.push_str(&format!("%{:02X}", byte)),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_jrg_url() {
        let config = HttpToJrgGatewayConfig {
            jrg_host: "127.0.0.1:7070".to_string(),
            ..Default::default()
        };

        // Explicit jrg:// URL
        let url = HttpToJrgGateway::build_jrg_url("jrg://example.org/page.jrg", &config).unwrap();
        assert_eq!(url, "jrg://example.org/page.jrg");

        // Relative path maps to configured host
        let url = HttpToJrgGateway::build_jrg_url("docs/page.jrg", &config).unwrap();
        assert_eq!(url, "jrg://127.0.0.1:7070/docs/page.jrg");
    }

    #[test]
    fn test_urlencode() {
        assert_eq!(urlencode("hello world"), "hello+world");
        assert_eq!(urlencode("a/b?c"), "a%2Fb%3Fc");
        assert_eq!(urlencode("simple"), "simple");
    }

    #[test]
    fn test_config_default_streaming_enabled() {
        let config = HttpToJrgGatewayConfig::default();
        assert!(config.enable_streaming);
    }

    #[test]
    fn test_config_streaming_can_be_disabled() {
        let config = HttpToJrgGatewayConfig {
            enable_streaming: false,
            ..Default::default()
        };
        assert!(!config.enable_streaming);
    }

    #[test]
    fn test_response_tag_stream_detection() {
        let response = jaringan_protocol::Response {
            status: jaringan_protocol::StatusCode::Ok,
            content_type: jaringan_protocol::ContentType::PlainText,
            tags: vec![ResponseTag::Stream],
            body: String::new(),
        };
        assert!(response.tags.contains(&ResponseTag::Stream));
    }

    #[test]
    fn test_response_tag_no_stream() {
        let response = jaringan_protocol::Response {
            status: jaringan_protocol::StatusCode::Ok,
            content_type: jaringan_protocol::ContentType::PlainText,
            tags: vec![],
            body: String::new(),
        };
        assert!(!response.tags.contains(&ResponseTag::Stream));
    }

    #[test]
    fn test_cache_defaults() {
        let config = HttpToJrgGatewayConfig::default();
        assert_eq!(config.cache_ttl_secs, 60);
        assert_eq!(config.cache_max_entries, 128);
    }

    #[test]
    fn test_cache_hit_returns_cached_content() {
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let config = HttpToJrgGatewayConfig::default();

        let key = "jrg://test/page.jrg";
        let expected_html = "<html>cached</html>".to_string();

        // Insert a cached entry directly
        {
            let mut guard = cache.lock().unwrap();
            guard.insert(
                key.to_string(),
                CachedEntry {
                    html: expected_html.clone(),
                    cached_at: Instant::now(),
                },
            );
        }

        // Verify fetch_via_jrg_inner returns the cached entry without fetching
        let url = JaringanUrl::parse(key).unwrap();
        let method = Method::GET;
        let result = fetch_via_jrg_inner(
            &url,
            &method,
            "",
            Duration::from_secs(10),
            key,
            &config,
            &cache,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected_html);
    }

    #[test]
    fn test_cache_miss_returns_error_from_tcp_fetch() {
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let config = HttpToJrgGatewayConfig::default();

        // Use a URL that will fail TCP fetch (no server listening)
        let key = "jrg://127.0.0.1:19999/nonexistent.jrg";
        let url = JaringanUrl::parse(key).unwrap();
        let method = Method::GET;

        let result = fetch_via_jrg_inner(
            &url,
            &method,
            "",
            Duration::from_secs(1),
            key,
            &config,
            &cache,
        );

        // Should fail with a connection error since no server is listening
        assert!(result.is_err());
        match &result {
            Err(GatewayError::JrgProtocol(_)) => {} // expected
            _ => panic!("Expected JrgProtocol error, got {:?}", result),
        }
    }

    #[test]
    fn test_cache_ttl_expiry_causes_miss() {
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let config = HttpToJrgGatewayConfig {
            cache_ttl_secs: 0, // 0 second TTL — always expired
            ..Default::default()
        };

        let key = "jrg://test/page.jrg";

        // Insert a cached entry
        {
            let mut guard = cache.lock().unwrap();
            guard.insert(
                key.to_string(),
                CachedEntry {
                    html: "<html>stale</html>".to_string(),
                    cached_at: Instant::now() - Duration::from_secs(1),
                },
            );
        }

        // Even though the entry exists, TTL=0 means it should be treated as expired.
        // After that, fetch_via_jrg_inner will try a real TCP fetch which will fail.
        let url = JaringanUrl::parse(key).unwrap();
        let method = Method::GET;

        let result = fetch_via_jrg_inner(
            &url,
            &method,
            "",
            Duration::from_secs(1),
            key,
            &config,
            &cache,
        );

        // Should fail with JrgProtocol (connection refused) because TTL expired
        assert!(result.is_err());
        match &result {
            Err(GatewayError::JrgProtocol(_)) => {} // expected
            _ => panic!("Expected JrgProtocol error (TTL expired -> TCP fetch), got {:?}", result),
        }
    }

    #[test]
    fn test_cache_non_get_not_cached() {
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let config = HttpToJrgGatewayConfig::default();

        // POST requests should not check cache
        let key = "jrg://test/page.jrg";

        // Insert a cached entry
        {
            let mut guard = cache.lock().unwrap();
            guard.insert(
                key.to_string(),
                CachedEntry {
                    html: "<html>stale</html>".to_string(),
                    cached_at: Instant::now(),
                },
            );
        }

        // POST should not serve from cache — it should try TCP fetch
        let url = JaringanUrl::parse(key).unwrap();
        let method = Method::POST;

        let result = fetch_via_jrg_inner(
            &url,
            &method,
            "",
            Duration::from_secs(1),
            key,
            &config,
            &cache,
        );

        // POST does not check cache, so it falls through to TCP fetch
        assert!(result.is_err());
    }

    #[test]
    fn test_cache_configurable_ttl() {
        let config = HttpToJrgGatewayConfig {
            cache_ttl_secs: 300,
            cache_max_entries: 64,
            ..Default::default()
        };
        assert_eq!(config.cache_ttl_secs, 300);
        assert_eq!(config.cache_max_entries, 64);
    }
}

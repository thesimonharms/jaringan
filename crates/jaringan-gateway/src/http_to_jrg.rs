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
//!     listen_addr: "127.0.0.1:8080".parse().unwrap(),
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
    response::{Html, IntoResponse, Response},
    routing::get,
};
use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{GatewayError, jrg_to_html};
use jaringan_protocol::{
    JaringanUrl,
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
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for HttpToJrgGatewayConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".to_string(),
            jrg_host: "127.0.0.1:7070".to_string(),
            enable_http_bridge: false,
            timeout_secs: 10,
        }
    }
}

#[derive(Clone)]
struct AppState {
    config: Arc<HttpToJrgGatewayConfig>,
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
        let state = AppState {
            config: Arc::new(self.config),
        };

        let router = Router::new()
            .route("/", get(root_handler))
            .route("/health", get(health_handler))
            .route_layer(middleware::from_fn(cors_middleware))
            .fallback(get(catch_all_handler).post(catch_all_handler));

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
        return fetch_via_jrg_and_respond(path, &method, &query, &body, &state.config).await;
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
        return fetch_via_jrg_and_respond(&jrg_url, &method, &query, &body, &state.config).await;
    }
    if path.starts_with("proxy/") {
        let jrg_url = format!("jrg://{}", path.strip_prefix("proxy/").unwrap_or(""));
        return fetch_via_jrg_and_respond(&jrg_url, &method, &query, &body, &state.config).await;
    }

    // Implicit: map path to configured JRG host
    let jrg_url = match HttpToJrgGateway::build_jrg_url(path, &state.config) {
        Ok(url) => url,
        Err(e) => return (HttpStatus::BAD_REQUEST, format!("Invalid path: {e}")).into_response(),
    };
    fetch_via_jrg_and_respond(&jrg_url, &method, &query, &body, &state.config).await
}

/// Fetch a JRG URL and return HTML response.
async fn fetch_via_jrg_and_respond(
    jrg_url: &str,
    http_method: &Method,
    query: &HashMap<String, String>,
    body: &str,
    config: &HttpToJrgGatewayConfig,
) -> Response {
    match fetch_via_jrg(jrg_url, http_method, query, body, config).await {
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

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(state.config.timeout_secs))
        .build()
        .unwrap();

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

/// Internal: perform a JRG TCP fetch and return HTML.
async fn fetch_via_jrg(
    jrg_url: &str,
    http_method: &Method,
    query: &HashMap<String, String>,
    body: &str,
    config: &HttpToJrgGatewayConfig,
) -> Result<String, GatewayError> {
    let url = JaringanUrl::parse(jrg_url).map_err(|e| {
        GatewayError::Config(format!("invalid JRG URL '{jrg_url}': {e}"))
    })?;

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

    let response = match *http_method {
        Method::GET | Method::HEAD | Method::OPTIONS => {
            fetch_tcp_with_timeout(&url, timeout).map_err(GatewayError::JrgProtocol)?
        }
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE => {
            post_tcp(&url, request_body).map_err(GatewayError::JrgProtocol)?
        }
        _ => {
            return Err(GatewayError::Config(format!(
                "unsupported HTTP method: {http_method}"
            )));
        }
    };

    Ok(jrg_to_html(&response))
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
}

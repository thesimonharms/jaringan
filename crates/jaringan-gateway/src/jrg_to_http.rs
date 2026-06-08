//! JRG→HTTP Gateway
//!
//! A JRG `PageResolver` that accepts JRG requests and proxies them to
//! real HTTP(S) resources. This allows JRG clients to browse the web
//! through the Jaringan protocol.
//!
//! # Usage
//!
//! ```rust,no_run
//! use jaringan_gateway::{JrgToHttpResolver, JrgToHttpResolverConfig};
//!
//! let resolver = JrgToHttpResolver::new(JrgToHttpResolverConfig {
//!     ..Default::default()
//! });
//!
//! // Use with any JRG TCP server:
//! // let listener = std::net::TcpListener::bind("127.0.0.1:7071")?;
//! // jaringan_protocol::serve(listener, resolver)?;
//! ```

use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;

use crate::GatewayError;
use jaringan_protocol::{
    PageResolver, Request, RequestMethod, ResolveError, Response, StatusCode,
};

/// Configuration for the JRG→HTTP gateway resolver.
#[derive(Debug, Clone)]
pub struct JrgToHttpResolverConfig {
    /// User-Agent header to use for HTTP requests.
    pub user_agent: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum response body size in bytes (default: 1 MB).
    pub max_response_size: usize,
    /// Whether to follow redirects.
    pub follow_redirects: bool,
}

impl Default for JrgToHttpResolverConfig {
    fn default() -> Self {
        Self {
            user_agent: "Jaringan/0.1 (+https://github.com/thesimonharms/jaringan)".to_string(),
            timeout_secs: 15,
            max_response_size: 1024 * 1024, // 1 MB
            follow_redirects: true,
        }
    }
}

/// A JRG `PageResolver` that proxies JRG requests to HTTP(S) resources.
///
/// When the JRG request targets a web URL (via the `jrg://http/` scheme),
/// this resolver fetches the web page and returns it as a JRG page.
#[derive(Clone)]
pub struct JrgToHttpResolver {
    config: JrgToHttpResolverConfig,
    client: Client,
}

impl JrgToHttpResolver {
    /// Create a new JRG→HTTP resolver.
    pub fn new(config: JrgToHttpResolverConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent(&config.user_agent)
            .danger_accept_invalid_certs(false)
            .build()
            .expect("reqwest blocking client build");

        Self { config, client }
    }

    /// Convert an HTTP response body to JRG format.
    fn http_to_jrg_body(content_type: &str, body: &str) -> String {
        let ct_lower = content_type.to_lowercase();

        if ct_lower.contains("text/html") {
            let cleaned = Self::strip_html_tags(body);
            format!(
                "# Web Page\n\n{}\n\n---\n\n> Fetched via JRG→HTTP gateway",
                cleaned
            )
        } else if ct_lower.contains("application/json") {
            format!("# JSON Response\n\n```json\n{}\n```", body)
        } else if ct_lower.contains("text/") {
            format!("# Text Response\n\n```\n{}\n```", body)
        } else {
            format!("{}\n\n---\n\nContent-Type: {}", body, content_type)
        }
    }

    /// Simple HTML tag stripping for display in JRG.
    fn strip_html_tags(html: &str) -> String {
        let mut result = String::new();
        let mut in_tag = false;
        let mut in_script = false;
        let mut in_style = false;
        let lower = html.to_lowercase();
        let mut iter = html.chars().enumerate().peekable();

        while let Some((i, ch)) = iter.next() {
            if ch == '<' {
                if lower[i..].starts_with("<script") {
                    in_script = true;
                    in_tag = true;
                } else if lower[i..].starts_with("<style") {
                    in_style = true;
                    in_tag = true;
                } else {
                    in_tag = true;
                }
                // Add a space when closing a tag to prevent words running together
                if !result.is_empty() && !result.ends_with(' ') {
                    result.push(' ');
                }
                continue;
            }

            if in_tag {
                if ch == '>' {
                    if in_script && lower[i.saturating_sub(8)..].starts_with("</script") {
                        in_script = false;
                    }
                    if in_style && lower[i.saturating_sub(6)..].starts_with("</style") {
                        in_style = false;
                    }
                    in_tag = false;
                }
                continue;
            }

            if in_script || in_style {
                continue;
            }

            result.push(ch);
        }

        // Collapse whitespace
        let mut collapsed = String::with_capacity(result.len());
        let mut prev_space = false;
        for ch in result.chars() {
            if ch.is_whitespace() {
                if !prev_space {
                    collapsed.push(' ');
                    prev_space = true;
                }
            } else {
                collapsed.push(ch);
                prev_space = false;
            }
        }

        collapsed.trim().to_string()
    }

    /// Check if a URL targets this HTTP gateway.
    fn is_gateway_url(url: &jaringan_protocol::JaringanUrl) -> bool {
        let host = url.host();
        host == "http" || host.starts_with("http.") || host.starts_with("https.")
    }

    /// Extract the actual HTTP URL from a gateway-formatted JRG URL.
    fn extract_http_url(url: &jaringan_protocol::JaringanUrl) -> Result<String, GatewayError> {
        let host = url.host();
        let path = url.path();

        if host == "http" {
            Ok(format!("http://{}", path.trim_start_matches('/')))
        } else if let Some(rest) = host.strip_prefix("https.") {
            Ok(format!("https://{}{}", rest, path))
        } else if let Some(rest) = host.strip_prefix("http.") {
            Ok(format!("http://{}{}", rest, path))
        } else {
            Err(GatewayError::Config(format!(
                "cannot extract HTTP URL from JRG URL: {url}"
            )))
        }
    }
}

impl PageResolver for JrgToHttpResolver {
    fn fetch(&self, request: &Request) -> Result<Response, ResolveError> {
        let http_url = if Self::is_gateway_url(&request.url) {
            Self::extract_http_url(&request.url).map_err(|e| ResolveError::Read {
                path: std::path::PathBuf::from(request.url.path()),
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()),
            })?
        } else {
            // Treat the JRG URL host:port as target host
            let host = request.url.host();
            let port = request.url.port_or_default();
            let path = request.url.path();
            format!("http://{}:{}{}", host, port, path)
        };

        // Build and send the HTTP request
        let mut http_req = match request.method {
            RequestMethod::Get => self.client.get(&http_url),
            RequestMethod::Post => {
                self.client
                    .post(&http_url)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(request.body.clone())
            }
        };

        if let Some(token) = &request.action_token {
            http_req = http_req.header("X-Action-Token", token);
        }

        let response = http_req.send().map_err(|e| ResolveError::Read {
            path: std::path::PathBuf::from(&http_url),
            source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
        })?;

        let http_status = response.status().as_u16();
        let jrg_status = StatusCode::from_u16(http_status).unwrap_or(StatusCode::Ok);

        let ct = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/plain")
            .to_string();

        let body = response
            .text()
            .unwrap_or_else(|_| "Error reading response body".to_string());

        // Truncate if too large
        let body = if body.len() > self.config.max_response_size {
            format!(
                "{}\n\n[Response truncated at {} bytes]",
                &body[..self.config.max_response_size],
                self.config.max_response_size,
            )
        } else {
            body
        };

        let jrg_body = Self::http_to_jrg_body(&ct, &body);
        let response = Response::page(jrg_status, jrg_body);

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_tags() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        assert_eq!(JrgToHttpResolver::strip_html_tags(html), "Hello World");

        let html_script = "<html><script>alert('xss');</script><p>Content</p></html>";
        assert_eq!(JrgToHttpResolver::strip_html_tags(html_script), "Content");
    }

    #[test]
    fn test_http_to_jrg_body_html() {
        let result = JrgToHttpResolver::http_to_jrg_body("text/html", "<h1>Hello</h1>");
        assert!(result.starts_with("# Web Page"));
        assert!(result.contains("Hello"));
    }

    #[test]
    fn test_http_to_jrg_body_json() {
        let result =
            JrgToHttpResolver::http_to_jrg_body("application/json", "{\"key\": \"value\"}");
        assert!(result.starts_with("# JSON Response"));
        assert!(result.contains("key"));
    }

    #[test]
    fn test_is_gateway_url() {
        let http_url = jaringan_protocol::JaringanUrl::parse("jrg://http/example.com").unwrap();
        assert!(JrgToHttpResolver::is_gateway_url(&http_url));

        let https_url =
            jaringan_protocol::JaringanUrl::parse("jrg://https.example.com/page").unwrap();
        assert!(JrgToHttpResolver::is_gateway_url(&https_url));
    }

    #[test]
    fn test_extract_http_url() {
        let url = jaringan_protocol::JaringanUrl::parse("jrg://http/example.com").unwrap();
        assert_eq!(
            JrgToHttpResolver::extract_http_url(&url).unwrap(),
            "http://example.com"
        );

        let url = jaringan_protocol::JaringanUrl::parse("jrg://https.example.com/page").unwrap();
        assert_eq!(
            JrgToHttpResolver::extract_http_url(&url).unwrap(),
            "https://example.com/page"
        );
    }
}

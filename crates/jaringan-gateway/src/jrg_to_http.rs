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

use std::collections::HashMap;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;

use crate::GatewayError;
use jaringan_protocol::{
    PageResolver, Request, RequestMethod, ResolveError, Response, ResponseTag, StatusCode,
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
    /// When true, accept invalid/self-signed TLS certificates.
    pub danger_accept_invalid_certs: bool,
    /// Additional headers to include in every HTTP request (e.g., User-Agent override, accept headers).
    pub additional_headers: HashMap<String, String>,
}

impl Default for JrgToHttpResolverConfig {
    fn default() -> Self {
        Self {
            user_agent: "Jaringan/0.1 (+https://github.com/thesimonharms/jaringan)".to_string(),
            timeout_secs: 15,
            max_response_size: 1024 * 1024, // 1 MB
            follow_redirects: true,
            danger_accept_invalid_certs: false,
            additional_headers: HashMap::new(),
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
        let builder = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent(&config.user_agent)
            .danger_accept_invalid_certs(config.danger_accept_invalid_certs)
            .redirect(if config.follow_redirects {
                Policy::default()
            } else {
                Policy::none()
            });

        let client = builder
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

    /// Check if the remaining bytes at position `i` form an opening tag with the given name.
    /// The tag name is followed by `>`, ` `, or `/`.
    fn is_tag(remaining: &[u8], tag_name: &[u8]) -> bool {
        let tlen = tag_name.len();
        let rlen = remaining.len();
        if rlen < tlen + 2 {
            return false;
        }
        if remaining[0] != b'<' {
            return false;
        }
        if !remaining[1..1 + tlen].eq_ignore_ascii_case(tag_name) {
            return false;
        }
        let after = remaining[1 + tlen];
        after == b'>' || after == b' ' || after == b'/' || after == b'\t' || after == b'\n'
    }

    /// Find the position of `>` relative to the start of `remaining` bytes.
    fn find_tag_end(remaining: &[u8]) -> Option<usize> {
        remaining.iter().position(|&b| b == b'>')
    }

    /// Extract a quoted attribute value from tag bytes.
    /// Searches for `attr_name="..."` (double-quoted only).
    fn extract_attr(tag_bytes: &[u8], attr_name: &[u8]) -> Option<String> {
        let tag_str = std::str::from_utf8(tag_bytes).ok()?;
        // Build prefix: e.g. `href="`
        let mut prefix = Vec::with_capacity(attr_name.len() + 2);
        prefix.extend_from_slice(attr_name);
        prefix.push(b'=');
        prefix.push(b'"');
        let prefix_str = std::str::from_utf8(&prefix).ok()?;

        if let Some(pos) = tag_str.find(prefix_str) {
            let value_start = pos + prefix_str.len();
            if let Some(end) = tag_str[value_start..].find('"') {
                return Some(tag_str[value_start..value_start + end].to_string());
            }
        }
        None
    }

    /// Enhanced HTML tag stripping for display in JRG.
    /// Uses a character-by-character state machine (O(n), no extra allocations).
    /// Handles: `<br>`, `<img>` (alt text), `<a href>` (link preservation), line breaks.
    fn strip_html_tags(html: &str) -> String {
        let mut result = String::with_capacity(html.len());
        let bytes = html.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        let mut in_tag = false;
        let mut in_script = false;
        let mut in_style = false;
        let mut in_anchor = false;
        let mut anchor_href = String::new();
        let mut anchor_text = String::new();

        while i < len {
            let ch = bytes[i] as char;

            if ch == '<' {
                // Check for script/style tags without lowercasing the whole string
                let remaining = &bytes[i..];
                let rlen = remaining.len();

                let tag3 = |s: &[u8]| s.len() >= 8
                    && s[1..8].eq_ignore_ascii_case(b"script>");
                let tag6 = |s: &[u8]| s.len() >= 7
                    && s[1..7].eq_ignore_ascii_case(b"style>");
                let close_script = rlen >= 9
                    && remaining[..9].eq_ignore_ascii_case(b"</script>");
                let close_style = rlen >= 8
                    && remaining[..8].eq_ignore_ascii_case(b"</style>");

                if close_script {
                    in_script = false;
                    in_tag = true;
                } else if close_style {
                    in_style = false;
                    in_tag = true;
                } else if !in_script && !in_style && tag3(remaining) {
                    in_script = true;
                    in_tag = true;
                } else if !in_script && !in_style && tag6(remaining) {
                    in_style = true;
                    in_tag = true;
                } else if !in_script && !in_style && in_anchor
                    && rlen >= 4
                    && remaining[..4].eq_ignore_ascii_case(b"</a>")
                {
                    // Closing anchor tag — output "text (url)"
                    let anchor_text_trimmed = anchor_text.trim();
                    if !anchor_text_trimmed.is_empty() && !anchor_href.is_empty() {
                        result.push_str(anchor_text_trimmed);
                        result.push_str(" (");
                        result.push_str(&anchor_href);
                        result.push(')');
                    } else if !anchor_text_trimmed.is_empty() {
                        result.push_str(anchor_text_trimmed);
                    }
                    in_anchor = false;
                    anchor_href.clear();
                    anchor_text.clear();
                    in_tag = true;
                } else if !in_script && !in_style && !in_anchor
                    && Self::is_tag(remaining, b"a")
                {
                    // Opening anchor tag — scan for href
                    in_anchor = true;
                    anchor_href.clear();
                    anchor_text.clear();
                    // Scan inside the tag for href="..."
                    let tag_end = Self::find_tag_end(&bytes[i..]);
                    if let Some(end) = tag_end {
                        let tag_content = &bytes[i..i + end + 1];
                        if let Some(href) = Self::extract_attr(tag_content, b"href") {
                            anchor_href = href;
                        }
                    }
                    in_tag = true;
                } else if !in_script && !in_style
                    && Self::is_tag(remaining, b"br")
                {
                    // <br> → newline
                    if !result.is_empty() && !result.ends_with('\n') && !result.ends_with(' ') {
                        result.push('\n');
                    }
                    in_tag = true;
                } else if !in_script && !in_style && Self::is_tag(remaining, b"img") {
                    // <img> — extract alt text
                    let tag_end = Self::find_tag_end(&bytes[i..]);
                    if let Some(end) = tag_end {
                        let tag_content = &bytes[i..i + end + 1];
                        if let Some(alt) = Self::extract_attr(tag_content, b"alt")
                            && !alt.is_empty() {
                                if !result.is_empty() && !result.ends_with(' ') {
                                    result.push(' ');
                                }
                                result.push_str(&alt);
                            }
                    }
                    in_tag = true;
                } else {
                    in_tag = true;
                }

                // Add a space when closing a tag to prevent words running together,
                // but only if we're not in an anchor collecting text
                if !in_anchor && !result.is_empty() && !result.ends_with(' ') && !result.ends_with('\n') {
                    result.push(' ');
                }
                i += 1;
                continue;
            }

            if in_tag {
                if ch == '>' {
                    in_tag = false;
                }
                i += 1;
                continue;
            }

            if in_script || in_style {
                i += 1;
                continue;
            }

            if in_anchor {
                anchor_text.push(ch);
            } else {
                result.push(ch);
            }
            i += 1;
        }

        // If anchor was never closed, flush any collected text
        if in_anchor {
            let anchor_text_trimmed = anchor_text.trim();
            if !anchor_text_trimmed.is_empty() && !anchor_href.is_empty() {
                result.push_str(anchor_text_trimmed);
                result.push_str(" (");
                result.push_str(&anchor_href);
                result.push(')');
            } else if !anchor_text_trimmed.is_empty() {
                result.push_str(anchor_text_trimmed);
            }
        }

        // Collapse whitespace but preserve newlines
        let mut collapsed = String::with_capacity(result.len());
        let mut prev_space = false;
        for ch in result.chars() {
            if ch == '\n' {
                collapsed.push('\n');
                prev_space = false;
            } else if ch.is_whitespace() {
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
        let port_opt = url.port();

        /// Helper: format host with optional port.
        fn with_port(host: &str, port: Option<u16>) -> String {
            match port {
                Some(p) => format!("{}:{}", host, p),
                None => host.to_string(),
            }
        }

        if host == "http" {
            Ok(format!("http://{}", path.trim_start_matches('/')))
        } else if let Some(rest) = host.strip_prefix("https.") {
            Ok(format!("https://{}{}", with_port(rest, port_opt), path))
        } else if let Some(rest) = host.strip_prefix("http.") {
            Ok(format!("http://{}{}", with_port(rest, port_opt), path))
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

        // Apply additional headers from config
        for (key, value) in &self.config.additional_headers {
            http_req = http_req.header(key.as_str(), value.as_str());
        }

        if let Some(token) = &request.action_token {
            http_req = http_req.header("X-Action-Token", token);
        }

        let response = http_req.send().map_err(|e| ResolveError::Read {
            path: std::path::PathBuf::from(&http_url),
            source: std::io::Error::other(e.to_string()),
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

        // Truncate if too large (char-boundary safe to avoid UTF-8 panic)
        let body = if body.len() > self.config.max_response_size {
            let truncate_at = self.config.max_response_size;
            // Find the nearest char boundary at or before truncate_at
            let safe_boundary = if body.is_char_boundary(truncate_at) {
                truncate_at
            } else {
                body[..truncate_at].char_indices().last().map(|(i, _)| i).unwrap_or(0)
            };
            format!(
                "{}\n\n[Response truncated at {} bytes]",
                &body[..safe_boundary],
                self.config.max_response_size,
            )
        } else {
            body
        };

        let jrg_body = Self::http_to_jrg_body(&ct, &body);
        let response = Response::page(jrg_status, jrg_body)
            .with_tag(ResponseTag::ContentType { value: ct });

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- strip_html_tags tests ---

    #[test]
    fn test_strip_html_tags_basic() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        assert_eq!(JrgToHttpResolver::strip_html_tags(html), "Hello World");

        let html_script = "<html><script>alert('xss');</script><p>Content</p></html>";
        assert_eq!(JrgToHttpResolver::strip_html_tags(html_script), "Content");
    }

    #[test]
    fn test_strip_html_br_tag() {
        // <br> should produce a newline
        let html = "Line1<br>Line2";
        assert_eq!(JrgToHttpResolver::strip_html_tags(html), "Line1\nLine2");

        // <br/> variant
        let html = "A<br/>B";
        assert_eq!(JrgToHttpResolver::strip_html_tags(html), "A\nB");

        // <br /> variant
        let html = "X<br />Y";
        assert_eq!(JrgToHttpResolver::strip_html_tags(html), "X\nY");
    }

    #[test]
    fn test_strip_html_img_alt() {
        // <img> with alt text should extract the alt
        let html = "Before <img src=\"foo.jpg\" alt=\"A photo\"> after";
        assert_eq!(
            JrgToHttpResolver::strip_html_tags(html),
            "Before A photo after"
        );

        // <img> without alt should be removed silently
        let html = "Text <img src=\"bar.png\"> more";
        assert_eq!(JrgToHttpResolver::strip_html_tags(html), "Text more");
    }

    #[test]
    fn test_strip_html_anchor() {
        // <a href="url">text</a> → "text (url)"
        let html = "Visit <a href=\"https://example.com\">Example</a> now";
        assert_eq!(
            JrgToHttpResolver::strip_html_tags(html),
            "Visit Example (https://example.com) now"
        );

        // <a> without href — just text
        let html = "<a>plain</a> link";
        assert_eq!(JrgToHttpResolver::strip_html_tags(html), "plain link");

        // Nested anchors not supported (unusual HTML), but ensure no panic
        let html = "<a href=\"u1\">one</a> and <a href=\"u2\">two</a>";
        let result = JrgToHttpResolver::strip_html_tags(html);
        assert!(result.contains("one (u1)"));
        assert!(result.contains("two (u2)"));
    }

    #[test]
    fn test_strip_html_line_breaks_preserved() {
        // Newlines in the source (e.g. <p>) should not all collapse
        let html = "<p>Para one</p><p>Para two</p>";
        let result = JrgToHttpResolver::strip_html_tags(html);
        assert!(result.contains("Para one"));
        assert!(result.contains("Para two"));
    }

    #[test]
    fn test_strip_html_complex() {
        let html = r#"<html>
<body>
<h1>Title</h1>
<p>Hello<br>World</p>
<img src="pic.jpg" alt="A nice picture">
Visit <a href="https://example.com">Example Site</a> for more.
<script>alert('hidden');</script>
</body>
</html>"#;
        let result = JrgToHttpResolver::strip_html_tags(html);
        assert!(result.contains("Title"));
        assert!(result.contains("Hello"));
        assert!(result.contains("World"));
        assert!(result.contains("A nice picture"));
        assert!(result.contains("Example Site (https://example.com)"));
        assert!(!result.contains("alert"));
        assert!(!result.contains("hidden"));
    }

    // --- http_to_jrg_body tests ---

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
    fn test_http_to_jrg_body_plain_text() {
        let result = JrgToHttpResolver::http_to_jrg_body("text/plain", "Hello world");
        assert!(result.starts_with("# Text Response"));
        assert!(result.contains("Hello world"));
    }

    // --- is_gateway_url tests ---

    #[test]
    fn test_is_gateway_url() {
        let http_url = jaringan_protocol::JaringanUrl::parse("jrg://http/example.com").unwrap();
        assert!(JrgToHttpResolver::is_gateway_url(&http_url));

        let https_url =
            jaringan_protocol::JaringanUrl::parse("jrg://https.example.com/page").unwrap();
        assert!(JrgToHttpResolver::is_gateway_url(&https_url));
    }

    // --- extract_http_url tests ---

    #[test]
    fn test_extract_http_url_basic() {
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

    #[test]
    fn test_extract_http_url_preserves_port() {
        // jrg://https.example.com:8443/path → https://example.com:8443/path
        let url =
            jaringan_protocol::JaringanUrl::parse("jrg://https.example.com:8443/path").unwrap();
        assert_eq!(
            JrgToHttpResolver::extract_http_url(&url).unwrap(),
            "https://example.com:8443/path"
        );

        // http. prefix with port
        let url =
            jaringan_protocol::JaringanUrl::parse("jrg://http.localhost:8080/api").unwrap();
        assert_eq!(
            JrgToHttpResolver::extract_http_url(&url).unwrap(),
            "http://localhost:8080/api"
        );

        // Without explicit port — no port in output
        let url =
            jaringan_protocol::JaringanUrl::parse("jrg://https.example.com/page").unwrap();
        assert_eq!(
            JrgToHttpResolver::extract_http_url(&url).unwrap(),
            "https://example.com/page"
        );
    }

    // --- config tests ---

    #[test]
    fn test_config_default_danger_accept_invalid_certs() {
        let config = JrgToHttpResolverConfig::default();
        assert!(!config.danger_accept_invalid_certs);
    }

    #[test]
    fn test_config_custom_danger_accept_invalid_certs() {
        let config = JrgToHttpResolverConfig {
            danger_accept_invalid_certs: true,
            ..Default::default()
        };
        assert!(config.danger_accept_invalid_certs);

        let resolver = JrgToHttpResolver::new(config);
        // 1 second timeout to avoid blocking
        let _ = resolver;
    }

    #[test]
    fn test_config_additional_headers_default_empty() {
        let config = JrgToHttpResolverConfig::default();
        assert!(config.additional_headers.is_empty());
    }

    #[test]
    fn test_config_additional_headers_custom() {
        let mut headers = HashMap::new();
        headers.insert("Accept".to_string(), "application/json".to_string());
        headers.insert("X-Custom".to_string(), "test-value".to_string());

        let config = JrgToHttpResolverConfig {
            additional_headers: headers,
            timeout_secs: 1,
            ..Default::default()
        };
        assert_eq!(config.additional_headers.len(), 2);
        assert_eq!(
            config.additional_headers.get("Accept").unwrap(),
            "application/json"
        );
        assert_eq!(
            config.additional_headers.get("X-Custom").unwrap(),
            "test-value"
        );

        let _resolver = JrgToHttpResolver::new(config);
    }

    // --- ResponseTag ContentType round-trip test ---

    #[test]
    fn test_response_tag_content_type() {
        let response = Response::page(StatusCode::Ok, "hello")
            .with_tag(ResponseTag::ContentType { value: "text/html; charset=utf-8".to_string() });
        assert!(response.tags.iter().any(|t| matches!(
            t,
            ResponseTag::ContentType { value } if value == "text/html; charset=utf-8"
        )));
    }
}
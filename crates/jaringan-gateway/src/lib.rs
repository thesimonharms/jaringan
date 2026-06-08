//! Jaringan HTTP↔JRG Gateway
//!
//! This crate provides two gateway implementations:
//! 1. **HTTP→JRG Gateway**: An HTTP server that accepts HTTP requests and forwards
//!    them to a JRG resolver (local file or network JRG server).
//! 2. **JRG→HTTP Gateway**: A JRG resolver that fetches HTTP resources and presents
//!    them as JRG pages, allowing JRG clients to browse the web.

use reqwest::Method as HttpMethod;
use thiserror::Error;

use jaringan_protocol::{ContentType, Response, ResponseTag};

pub mod http_to_jrg;
pub mod jrg_to_http;

pub use http_to_jrg::{HttpToJrgGateway, HttpToJrgGatewayConfig};
pub use jrg_to_http::{JrgToHttpResolver, JrgToHttpResolverConfig};

/// Common gateway errors
#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("JRG protocol error: {0}")]
    JrgProtocol(#[from] jaringan_protocol::WireError),
    #[error("HTTP request error: {0}")]
    HttpRequest(#[from] reqwest::Error),
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid gateway configuration: {0}")]
    Config(String),
    #[error("JRG resolver error: {0}")]
    Resolver(String),
}

/// Convert an HTTP method to JRG request method
pub fn http_method_to_jrg(method: &HttpMethod) -> jaringan_protocol::RequestMethod {
    match *method {
        HttpMethod::GET => jaringan_protocol::RequestMethod::Get,
        HttpMethod::POST => jaringan_protocol::RequestMethod::Post,
        HttpMethod::PUT => jaringan_protocol::RequestMethod::Post,
        HttpMethod::DELETE => jaringan_protocol::RequestMethod::Post,
        HttpMethod::PATCH => jaringan_protocol::RequestMethod::Post,
        HttpMethod::HEAD => jaringan_protocol::RequestMethod::Get,
        HttpMethod::OPTIONS => jaringan_protocol::RequestMethod::Get,
        _ => jaringan_protocol::RequestMethod::Get,
    }
}

/// Convert a JRG response to HTTP response parts
pub fn jrg_response_to_http_response(
    response: Response,
) -> (u16, String, Vec<(String, String)>, String) {
    let status = response.status.as_u16();
    let reason = response.status.reason_phrase().to_string();
    let mut headers = vec![(
        "Content-Type".to_string(),
        response.content_type.as_str().to_string(),
    )];

    for tag in response.tags {
        match tag {
            ResponseTag::Redirect { target } => {
                headers.push(("Location".to_string(), target));
            }
            ResponseTag::Stream => {
                headers.push(("X-JRG-Stream".to_string(), "true".to_string()));
            }
        }
    }

    (status, reason, headers, response.body)
}

/// Convert JRG response body to HTML for browser display
pub fn jrg_to_html(response: &Response) -> String {
    let rendered = if response.content_type == ContentType::JaringanPage {
        jaringan_render::render_plain(&jaringan_core::parse_document(&response.body).unwrap())
    } else {
        response.body.clone()
    };

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Jaringan Page</title>
    <style>
        body {{ font-family: monospace; max-width: 800px; margin: 2rem auto; padding: 0 1rem; }}
        pre {{ background: #f4f4f4; padding: 1rem; overflow-x: auto; }}
        .status {{ color: #666; margin-bottom: 1rem; }}
        .header {{ color: #888; font-size: 0.9rem; }}
    </style>
</head>
<body>
    <div class="status">JRG/0.1 {} {}</div>
    <pre>{}</pre>
</body>
</html>"#,
        response.status.as_u16(),
        response.status.reason_phrase(),
        html_escape(&rendered)
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape(""), "");
    }

    #[test]
    fn test_http_method_to_jrg() {
        assert_eq!(
            http_method_to_jrg(&HttpMethod::GET),
            jaringan_protocol::RequestMethod::Get
        );
        assert_eq!(
            http_method_to_jrg(&HttpMethod::POST),
            jaringan_protocol::RequestMethod::Post
        );
        assert_eq!(
            http_method_to_jrg(&HttpMethod::PUT),
            jaringan_protocol::RequestMethod::Post
        );
        assert_eq!(
            http_method_to_jrg(&HttpMethod::DELETE),
            jaringan_protocol::RequestMethod::Post
        );
    }
}

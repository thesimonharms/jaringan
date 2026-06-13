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
            ResponseTag::Key { key_id, key_base64 } => {
                headers.push(("X-JRG-Key".to_string(), format!("{key_id} ed25519:{key_base64}")));
            }
            ResponseTag::ContentType { value } => {
                headers.push(("X-JRG-Content-Type".to_string(), value));
            }
            ResponseTag::Token {
                service,
                value,
                expires_at,
            } => {
                let mut header_val = format!("service={service} value={value}");
                if let Some(expires_at) = expires_at {
                    header_val.push_str(&format!(" expires_at={expires_at}"));
                }
                headers.push(("X-JRG-Token".to_string(), header_val));
            }
        }
    }

    (status, reason, headers, response.body)
}

/// Terminal-aesthetic CSS for JRG HTML pages.
const TERMINAL_CSS: &str = r#"
body {
    background: #0d0d0d;
    color: #c9d1d9;
    font-family: 'Fira Code', 'Cascadia Code', 'JetBrains Mono', 'Menlo', 'Consolas', monospace;
    font-size: 14px;
    line-height: 1.6;
    margin: 0;
    padding: 2rem 1rem;
}
.jrg-wrapper {
    max-width: 800px;
    margin: 0 auto;
}
.jrg-status {
    color: #8b949e;
    font-size: 12px;
    margin-bottom: 1.5rem;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid #21262d;
}
.jrg-content {}
h1 { color: #58a6ff; font-size: 1.8em; font-weight: 600; margin: 1.5em 0 0.5em; }
h2 { color: #58a6ff; font-size: 1.4em; font-weight: 600; margin: 1.3em 0 0.4em; }
h3 { color: #79c0ff; font-size: 1.15em; font-weight: 600; margin: 1.2em 0 0.3em; }
h4 { color: #79c0ff; font-size: 1em; font-weight: 600; margin: 1em 0 0.2em; }
p { margin: 0.5em 0; }
a { color: #58a6ff; text-decoration: none; }
a:hover { text-decoration: underline; color: #79c0ff; }
.jrg-link { color: #58a6ff; }
.jrg-link:hover { text-decoration: underline; }
ul { list-style: none; padding-left: 1.5em; }
ul li::before { content: "• "; color: #8b949e; }
li { margin: 0.2em 0; }
blockquote {
    margin: 0.5em 0;
    padding: 0.3em 1em;
    border-left: 3px solid #30363d;
    color: #8b949e;
}
blockquote p { margin: 0.2em 0; }
pre {
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 1em;
    overflow-x: auto;
    margin: 0.5em 0;
}
code {
    font-family: 'Fira Code', 'Cascadia Code', 'JetBrains Mono', 'Menlo', 'Consolas', monospace;
    font-size: 13px;
}
p code, li code, h1 code, h2 code, h3 code, h4 code {
    background: #161b22;
    padding: 0.2em 0.4em;
    border-radius: 4px;
    font-size: 0.9em;
}
strong { color: #f0f6fc; font-weight: 700; }
em { color: #c9d1d9; font-style: italic; }
hr { border: none; border-top: 1px solid #21262d; margin: 1.5em 0; }
table.jrg-table {
    border-collapse: collapse;
    margin: 0.5em 0;
    width: 100%;
    font-size: 13px;
}
table.jrg-table th {
    background: #161b22;
    border: 1px solid #30363d;
    padding: 0.4em 0.6em;
    text-align: left;
    font-weight: 600;
    color: #f0f6fc;
}
table.jrg-table td {
    border: 1px solid #30363d;
    padding: 0.4em 0.6em;
}
table.jrg-table tr:nth-child(even) td { background: #0d1117; }
.jrg-input { margin: 0.5em 0; }
.jrg-input label { display: block; color: #8b949e; font-size: 12px; margin-bottom: 0.2em; }
.jrg-input input {
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 4px;
    color: #c9d1d9;
    font-family: inherit;
    font-size: 13px;
    padding: 0.4em 0.6em;
    width: 100%;
    max-width: 400px;
    box-sizing: border-box;
}
.jrg-btn {
    background: #21262d;
    border: 1px solid #30363d;
    border-radius: 4px;
    color: #c9d1d9;
    font-family: inherit;
    font-size: 13px;
    padding: 0.4em 1em;
    cursor: pointer;
    margin: 0.25em 0;
}
.jrg-btn:hover { background: #30363d; border-color: #8b949e; }
img { max-width: 100%; border-radius: 6px; margin: 0.5em 0; }
"#;

/// HTML-escape a string (replace &, <, >, ", ').
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Convert JRG response body to HTML for browser display.
///
/// For JRG pages, this parses the document and renders it as semantic HTML
/// with terminal-themed styling. For other content types, it returns the body
/// as-is wrapped in a minimal document.
pub fn jrg_to_html(response: &Response) -> String {
    let (body_content, title) = if response.content_type == ContentType::JaringanPage {
        match jaringan_core::parse_document(&response.body) {
            Ok(doc) => {
                let title = doc.title().unwrap_or("Jaringan Page").to_owned();
                (jaringan_render::render_html(&doc), title)
            }
            Err(_) => (
                format!("<pre>{}</pre>", html_escape(&response.body)),
                "Jaringan Page (parse error)".to_owned(),
            ),
        }
    } else {
        (response.body.clone(), "Jaringan Page".to_owned())
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>{title}</title>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>{css}</style>
</head>
<body>
    <div class="jrg-wrapper">
        <div class="jrg-status">JRG/0.1 {status} {reason}</div>
        <main class="jrg-content">
            {body}
        </main>
        <footer style="margin-top: 2rem; padding-top: 1rem; border-top: 1px solid #21262d; font-size: 12px; color: #8b949e;">
            Built with <a href="https://github.com/thesimonharms/jaringan" style="color: #58a6ff;">Jaringan</a>
            — the terminal-native, AI-friendly web protocol.
        </footer>
    </div>
</body>
</html>"#,
        title = title,
        status = response.status.as_u16(),
        reason = response.status.reason_phrase(),
        body = body_content,
        css = TERMINAL_CSS,
    )
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

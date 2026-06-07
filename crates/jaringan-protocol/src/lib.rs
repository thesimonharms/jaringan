use std::{
    fmt, fs, io,
    path::{Component, PathBuf},
};

use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JaringanUrl(Url);

impl JaringanUrl {
    pub fn parse(input: &str) -> Result<Self, UrlError> {
        let url = Url::parse(input).map_err(UrlError::Parse)?;
        Self::from_url(url)
    }

    fn from_url(url: Url) -> Result<Self, UrlError> {
        if url.scheme() != "jrg" {
            return Err(UrlError::UnsupportedScheme(url.scheme().to_owned()));
        }
        if url.host_str().is_none() {
            return Err(UrlError::MissingHost);
        }
        Ok(Self(url))
    }

    pub fn host(&self) -> &str {
        self.0.host_str().expect("validated in parse")
    }

    pub fn path(&self) -> &str {
        self.0.path()
    }

    pub fn query(&self) -> Option<&str> {
        self.0.query()
    }

    pub fn fragment(&self) -> Option<&str> {
        self.0.fragment()
    }

    pub fn resolve(&self, target: &str) -> Result<Self, UrlError> {
        let url = self.0.join(target).map_err(UrlError::Parse)?;
        Self::from_url(url)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for JaringanUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum UrlError {
    #[error("failed to parse URL: {0}")]
    Parse(url::ParseError),
    #[error("unsupported scheme `{0}`; expected `jrg`")]
    UnsupportedScheme(String),
    #[error("jrg URL must include a host")]
    MissingHost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub url: JaringanUrl,
}

impl Request {
    pub fn new(url: JaringanUrl) -> Self {
        Self { url }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: StatusCode,
    pub content_type: ContentType,
    pub tags: Vec<ResponseTag>,
    pub body: String,
}

impl Response {
    pub fn page(status: StatusCode, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: ContentType::JaringanPage,
            tags: Vec::new(),
            body: body.into(),
        }
    }

    pub fn text(status: StatusCode, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: ContentType::PlainText,
            tags: Vec::new(),
            body: body.into(),
        }
    }

    pub fn with_tag(mut self, tag: ResponseTag) -> Self {
        self.tags.push(tag);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseTag {
    Redirect { target: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusCode {
    Ok,
    MovedPermanently,
    Found,
    SeeOther,
    NotModified,
    BadRequest,
    Forbidden,
    NotFound,
    Conflict,
    Gone,
    UnprocessableContent,
    TooManyRequests,
    ServerError,
    NotImplemented,
    BadGateway,
    ServiceUnavailable,
}

impl StatusCode {
    pub fn as_u16(self) -> u16 {
        match self {
            Self::Ok => 200,
            Self::MovedPermanently => 301,
            Self::Found => 302,
            Self::SeeOther => 303,
            Self::NotModified => 304,
            Self::BadRequest => 400,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::Conflict => 409,
            Self::Gone => 410,
            Self::UnprocessableContent => 422,
            Self::TooManyRequests => 429,
            Self::ServerError => 500,
            Self::NotImplemented => 501,
            Self::BadGateway => 502,
            Self::ServiceUnavailable => 503,
        }
    }

    pub fn reason_phrase(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::MovedPermanently => "Moved Permanently",
            Self::Found => "Found",
            Self::SeeOther => "See Other",
            Self::NotModified => "Not Modified",
            Self::BadRequest => "Bad Request",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "Not Found",
            Self::Conflict => "Conflict",
            Self::Gone => "Gone",
            Self::UnprocessableContent => "Unprocessable Content",
            Self::TooManyRequests => "Too Many Requests",
            Self::ServerError => "Internal Server Error",
            Self::NotImplemented => "Not Implemented",
            Self::BadGateway => "Bad Gateway",
            Self::ServiceUnavailable => "Service Unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    JaringanPage,
    PlainText,
}

impl ContentType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JaringanPage => "text/jrg; charset=utf-8",
            Self::PlainText => "text/plain; charset=utf-8",
        }
    }
}

pub trait PageResolver {
    fn fetch(&self, request: &Request) -> Result<Response, ResolveError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFileResolver {
    root: PathBuf,
}

impl LocalFileResolver {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve_path(&self, url: &JaringanUrl) -> Option<PathBuf> {
        let path = url.path();
        let relative = if path == "/" {
            PathBuf::from("index.jrg")
        } else if path.ends_with('/') {
            PathBuf::from(path.trim_start_matches('/')).join("index.jrg")
        } else if path.ends_with(".jrg") {
            PathBuf::from(path.trim_start_matches('/'))
        } else {
            return None;
        };

        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return None;
        }

        Some(self.root.join(relative))
    }
}

impl PageResolver for LocalFileResolver {
    fn fetch(&self, request: &Request) -> Result<Response, ResolveError> {
        let Some(path) = self.resolve_path(&request.url) else {
            return Ok(Response::text(
                StatusCode::NotFound,
                format!("{} is not a .jrg document path", request.url.path()),
            ));
        };

        match fs::read_to_string(&path) {
            Ok(body) => Ok(Response::page(StatusCode::Ok, body)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Response::text(
                StatusCode::NotFound,
                format!("Jaringan page not found: {}", request.url.path()),
            )),
            Err(error) => Err(ResolveError::Read {
                path,
                source: error,
            }),
        }
    }
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("failed to read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jrg_urls() {
        let url = JaringanUrl::parse("jrg://example.org/docs/start").unwrap();

        assert_eq!(url.host(), "example.org");
        assert_eq!(url.path(), "/docs/start");
    }

    #[test]
    fn rejects_other_schemes() {
        let error = JaringanUrl::parse("https://example.org").unwrap_err();

        assert!(matches!(error, UrlError::UnsupportedScheme(scheme) if scheme == "https"));
    }

    #[test]
    fn rejects_legacy_jar_scheme() {
        let error = JaringanUrl::parse("jar://example.org/docs/start").unwrap_err();

        assert!(matches!(error, UrlError::UnsupportedScheme(scheme) if scheme == "jar"));
    }

    #[test]
    fn exposes_status_numbers() {
        assert_eq!(StatusCode::Ok.as_u16(), 200);
        assert_eq!(StatusCode::NotFound.as_u16(), 404);
    }

    #[test]
    fn supports_query_strings_and_fragments() {
        let url = JaringanUrl::parse("jrg://example.org/search.jrg?q=terminal#results").unwrap();

        assert_eq!(url.host(), "example.org");
        assert_eq!(url.path(), "/search.jrg");
        assert_eq!(url.query(), Some("q=terminal"));
        assert_eq!(url.fragment(), Some("results"));
    }

    #[test]
    fn resolves_relative_links_like_html_anchors() {
        let base = JaringanUrl::parse("jrg://example.org/docs/start.jrg?old=1#top").unwrap();

        assert_eq!(
            base.resolve("guide/intro.jrg?mode=ai#install")
                .unwrap()
                .as_str(),
            "jrg://example.org/docs/guide/intro.jrg?mode=ai#install"
        );
        assert_eq!(
            base.resolve("../about.jrg").unwrap().as_str(),
            "jrg://example.org/about.jrg"
        );
        assert_eq!(
            base.resolve("#section-two").unwrap().as_str(),
            "jrg://example.org/docs/start.jrg?old=1#section-two"
        );
    }

    #[test]
    fn local_file_resolver_requires_jrg_documents_and_serves_folder_indexes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("foo")).unwrap();
        std::fs::write(root.path().join("index.jrg"), "# Home\n").unwrap();
        std::fs::write(root.path().join("foo/index.jrg"), "# Folder\n").unwrap();
        std::fs::write(root.path().join("foo.jrg"), "# Document\n").unwrap();

        let resolver = LocalFileResolver::new(root.path());

        assert_eq!(
            resolver
                .fetch(&Request::new(
                    JaringanUrl::parse("jrg://example.org/foo").unwrap(),
                ))
                .unwrap()
                .status,
            StatusCode::NotFound
        );
        assert_eq!(
            resolver
                .fetch(&Request::new(
                    JaringanUrl::parse("jrg://example.org/foo/").unwrap(),
                ))
                .unwrap()
                .body,
            "# Folder\n"
        );
        assert_eq!(
            resolver
                .fetch(&Request::new(
                    JaringanUrl::parse("jrg://example.org/foo.jrg?q=1#frag").unwrap(),
                ))
                .unwrap()
                .body,
            "# Document\n"
        );
    }
}

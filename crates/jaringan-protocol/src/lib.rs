use std::fmt;

use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JaringanUrl(Url);

impl JaringanUrl {
    pub fn parse(input: &str) -> Result<Self, UrlError> {
        let url = Url::parse(input).map_err(UrlError::Parse)?;
        if url.scheme() != "jar" {
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
    #[error("unsupported scheme `{0}`; expected `jar`")]
    UnsupportedScheme(String),
    #[error("jar URL must include a host")]
    MissingHost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub url: JaringanUrl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: StatusCode,
    pub content_type: ContentType,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusCode {
    Ok,
    NotFound,
    BadRequest,
    ServerError,
}

impl StatusCode {
    pub fn as_u16(self) -> u16 {
        match self {
            Self::Ok => 200,
            Self::BadRequest => 400,
            Self::NotFound => 404,
            Self::ServerError => 500,
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
            Self::JaringanPage => "text/jaringan; charset=utf-8",
            Self::PlainText => "text/plain; charset=utf-8",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jar_urls() {
        let url = JaringanUrl::parse("jar://example.org/docs/start").unwrap();

        assert_eq!(url.host(), "example.org");
        assert_eq!(url.path(), "/docs/start");
    }

    #[test]
    fn rejects_other_schemes() {
        let error = JaringanUrl::parse("https://example.org").unwrap_err();

        assert!(matches!(error, UrlError::UnsupportedScheme(scheme) if scheme == "https"));
    }

    #[test]
    fn exposes_status_numbers() {
        assert_eq!(StatusCode::Ok.as_u16(), 200);
        assert_eq!(StatusCode::NotFound.as_u16(), 404);
    }
}

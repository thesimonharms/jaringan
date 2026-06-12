use std::{
    fmt, fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::{Component, PathBuf},
    time::Duration,
};

use base64::Engine;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use thiserror::Error;
use url::Url;

/// Maximum accepted request body size (10 MB). Prevents OOM from malicious
/// or malformed Content-Length headers.
pub const MAX_REQUEST_BODY: usize = 10 * 1024 * 1024;

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

    pub fn port(&self) -> Option<u16> {
        self.0.port()
    }

    pub fn port_or_default(&self) -> u16 {
        self.0.port().unwrap_or(7070)
    }

    pub fn authority(&self) -> String {
        match self.0.port() {
            Some(port) => format!("{}:{port}", self.host()),
            None => self.host().to_owned(),
        }
    }

    pub fn request_target(&self) -> String {
        let mut target = self.path().to_owned();
        if let Some(query) = self.query() {
            target.push('?');
            target.push_str(query);
        }
        if let Some(fragment) = self.fragment() {
            target.push('#');
            target.push_str(fragment);
        }
        target
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestMethod {
    Get,
    Post,
}

impl RequestMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }

    fn parse(input: &str) -> Option<Self> {
        match input {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: RequestMethod,
    pub url: JaringanUrl,
    pub body: String,
    pub action_token: Option<String>,
}

impl Request {
    pub fn new(url: JaringanUrl) -> Self {
        Self {
            method: RequestMethod::Get,
            url,
            body: String::new(),
            action_token: None,
        }
    }

    pub fn post(url: JaringanUrl, body: impl Into<String>) -> Self {
        Self {
            method: RequestMethod::Post,
            url,
            body: body.into(),
            action_token: None,
        }
    }

    pub fn with_action_token(mut self, token: impl Into<String>) -> Self {
        self.action_token = Some(token.into());
        self
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
    /// Response is a stream — the connection stays open for incremental blocks.
    Stream,
    /// Server advertises its ed25519 public key for signature verification.
    /// Format: `Tag-Key: <key-id> ed25519:<base64-public-key>`
    Key { key_id: String, key_base64: String },
    /// Content-Type of the original HTTP response.
    /// Format: `Tag-ContentType: <mime-type>`
    ContentType { value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionSuite {
    XChaCha20Poly1305,
}

impl EncryptionSuite {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::XChaCha20Poly1305 => "xchacha20poly1305",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "xchacha20poly1305" => Some(Self::XChaCha20Poly1305),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionCapability {
    pub suite: EncryptionSuite,
    pub key_id: String,
}

impl EncryptionCapability {
    pub fn xchacha20_poly1305(key_id: impl Into<String>) -> Self {
        Self {
            suite: EncryptionSuite::XChaCha20Poly1305,
            key_id: key_id.into(),
        }
    }

    pub fn to_header_value(&self) -> String {
        format!("{}; key-id={}", self.suite.as_str(), self.key_id)
    }

    pub fn from_header_value(input: &str) -> Result<Self, EncryptionError> {
        let mut parts = input.split(';').map(str::trim);
        let suite = parts
            .next()
            .and_then(EncryptionSuite::parse)
            .ok_or(EncryptionError::BadCapability)?;
        let mut key_id = None;
        for part in parts {
            if let Some(value) = part.strip_prefix("key-id=") {
                key_id = Some(value.trim().to_owned());
            }
        }
        let key_id = key_id
            .filter(|value| !value.is_empty())
            .ok_or(EncryptionError::BadCapability)?;
        Ok(Self { suite, key_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionKey([u8; 32]);

impl EncryptionKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncryptionNonce([u8; 24]);

impl EncryptionNonce {
    pub fn from_bytes(bytes: [u8; 24]) -> Self {
        Self(bytes)
    }

    fn as_bytes(&self) -> &[u8; 24] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedPayload {
    pub suite: EncryptionSuite,
    pub nonce_base64: String,
    pub ciphertext_base64: String,
}

pub fn encrypt_payload(
    key: &EncryptionKey,
    nonce: EncryptionNonce,
    plaintext: &[u8],
    associated_data: &[u8],
) -> Result<EncryptedPayload, EncryptionError> {
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(nonce.as_bytes()),
            chacha20poly1305::aead::Payload {
                msg: plaintext,
                aad: associated_data,
            },
        )
        .map_err(|_| EncryptionError::Encrypt)?;
    Ok(EncryptedPayload {
        suite: EncryptionSuite::XChaCha20Poly1305,
        nonce_base64: base64::engine::general_purpose::STANDARD.encode(nonce.as_bytes()),
        ciphertext_base64: base64::engine::general_purpose::STANDARD.encode(ciphertext),
    })
}

pub fn decrypt_payload(
    key: &EncryptionKey,
    payload: &EncryptedPayload,
    associated_data: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    if payload.suite != EncryptionSuite::XChaCha20Poly1305 {
        return Err(EncryptionError::UnsupportedSuite);
    }
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(&payload.nonce_base64)
        .map_err(|_| EncryptionError::BadNonce)?;
    let nonce: [u8; 24] = nonce.try_into().map_err(|_| EncryptionError::BadNonce)?;
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&payload.ciphertext_base64)
        .map_err(|_| EncryptionError::BadCiphertext)?;
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            chacha20poly1305::aead::Payload {
                msg: &ciphertext,
                aad: associated_data,
            },
        )
        .map_err(|_| EncryptionError::Decrypt)
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EncryptionError {
    #[error("bad encryption capability")]
    BadCapability,
    #[error("unsupported encryption suite")]
    UnsupportedSuite,
    #[error("bad encryption nonce")]
    BadNonce,
    #[error("bad ciphertext")]
    BadCiphertext,
    #[error("encryption failed")]
    Encrypt,
    #[error("decryption failed")]
    Decrypt,
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
    pub fn from_u16(code: u16) -> Option<Self> {
        match code {
            200 => Some(Self::Ok),
            301 => Some(Self::MovedPermanently),
            302 => Some(Self::Found),
            303 => Some(Self::SeeOther),
            304 => Some(Self::NotModified),
            400 => Some(Self::BadRequest),
            403 => Some(Self::Forbidden),
            404 => Some(Self::NotFound),
            409 => Some(Self::Conflict),
            410 => Some(Self::Gone),
            422 => Some(Self::UnprocessableContent),
            429 => Some(Self::TooManyRequests),
            500 => Some(Self::ServerError),
            501 => Some(Self::NotImplemented),
            502 => Some(Self::BadGateway),
            503 => Some(Self::ServiceUnavailable),
            _ => None,
        }
    }

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
    JrgStream,
}

impl ContentType {
    pub fn from_header(input: &str) -> Option<Self> {
        let media_type = input.split(';').next()?.trim();
        match media_type {
            "text/jrg" | "text/jaringan" => Some(Self::JaringanPage),
            "text/plain" => Some(Self::PlainText),
            "text/jrg-stream" => Some(Self::JrgStream),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::JaringanPage => "text/jrg; charset=utf-8",
            Self::PlainText => "text/plain; charset=utf-8",
            Self::JrgStream => "text/jrg-stream; charset=utf-8",
        }
    }
}

pub trait PageResolver {
    fn fetch(&self, request: &Request) -> Result<Response, ResolveError>;
}

/// A resolver that serves files from a local directory root.
pub struct LocalFileResolver {
    root: PathBuf,
    /// Optional key identity to include as `Tag-Key` in every response.
    pub advertise_key_id: Option<(String, String)>,
}

impl LocalFileResolver {
    /// Create a new resolver rooted at the given directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            advertise_key_id: None,
        }
    }

    /// Create a new resolver that advertises a key in every response.
    pub fn new_with_key(root: PathBuf, key_id: String, key_base64: String) -> Self {
        Self {
            root,
            advertise_key_id: Some((key_id, key_base64)),
        }
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

    fn handle_demo_search_action(&self, request: &Request) -> Result<Response, ResolveError> {
        if request.action_token.as_deref() != Some("demo-search") {
            return Ok(Response::text(
                StatusCode::Forbidden,
                "missing or invalid action capability token",
            ));
        }

        fs::create_dir_all(&self.root).map_err(|source| ResolveError::Write {
            path: self.root.clone(),
            source,
        })?;
        let log_path = self.root.join(".jrg-actions.log");
        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|source| ResolveError::Write {
                path: log_path.clone(),
                source,
            })?;
        writeln!(log, "POST {} {}", request.url.path(), request.body).map_err(|source| {
            ResolveError::Write {
                path: log_path.clone(),
                source,
            }
        })?;

        let query = form_value(&request.body, "q").unwrap_or_default();
        Ok(Response::page(
            StatusCode::Ok,
            format!(
                "# Search Results\n\nDemo action received query: {query}\n\n=> /action-form.jrg Back to form\n"
            ),
        ))
    }
}

fn form_value(body: &str, key: &str) -> Option<String> {
    body.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| percent_decode(value))
    })
}

fn percent_decode(input: &str) -> String {
    let mut output = Vec::new();
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let decoded = std::str::from_utf8(&bytes[index + 1..index + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(value) = decoded {
                output.push(value);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

impl PageResolver for LocalFileResolver {
    fn fetch(&self, request: &Request) -> Result<Response, ResolveError> {
        if request.method == RequestMethod::Post && request.url.path() == "/actions/search" {
            return self.handle_demo_search_action(request);
        }

        // Handle .well-known/key — return the advertised key as a page
        if request.url.path() == "/.well-known/key" {
            if let Some((key_id, key_base64)) = &self.advertise_key_id {
                let body = format!(
                    "# Server Key\n\n\
                     This server is signed by key:\n\n\
                     ```\n{} ed25519:{}\n```\n\n\
                     To trust this key, add it to your keyring:\n\n\
                     ```\n{} ed25519:{}\n```\n\
                     \n~~~\ntitle: Server Key\n",
                    key_id, key_base64, key_id, key_base64,
                );
                return Ok(Response::page(StatusCode::Ok, body).with_tag(ResponseTag::Key {
                    key_id: key_id.clone(),
                    key_base64: key_base64.clone(),
                }));
            } else {
                return Ok(Response::text(
                    StatusCode::NotFound,
                    "This server does not advertise a public key. Start with --advertise-key KEY_ID".to_string(),
                ));
            }
        }

        let Some(path) = self.resolve_path(&request.url) else {
            return Ok(Response::text(
                StatusCode::NotFound,
                format!("{} is not a .jrg document path", request.url.path()),
            ));
        };

        let mut response = match fs::read_to_string(&path) {
            Ok(body) => Ok(Response::page(StatusCode::Ok, body)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Response::text(
                StatusCode::NotFound,
                format!("Jaringan page not found: {}", request.url.path()),
            )),
            Err(error) => Err(ResolveError::Read {
                path,
                source: error,
            }),
        }?;

        // Inject Tag-Key on every response if the server advertises one
        if let Some((key_id, key_base64)) = &self.advertise_key_id {
            response.tags.push(ResponseTag::Key {
                key_id: key_id.clone(),
                key_base64: key_base64.clone(),
            });
        }

        Ok(response)
    }
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("failed to read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to write {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
}

pub fn serve(listener: TcpListener, resolver: impl PageResolver) -> Result<(), WireError> {
    for stream in listener.incoming() {
        serve_stream(stream?, &resolver)?;
    }
    Ok(())
}

pub fn serve_one(listener: TcpListener, resolver: impl PageResolver) -> Result<(), WireError> {
    let (stream, _) = listener.accept()?;
    serve_stream(stream, &resolver)
}

pub fn serve_encrypted(
    listener: TcpListener,
    resolver: impl PageResolver,
    config: &EncryptedTcpConfig,
) -> Result<(), WireError> {
    serve_encrypted_connections(listener, resolver, config, None)
}

fn serve_encrypted_connections(
    listener: TcpListener,
    resolver: impl PageResolver,
    config: &EncryptedTcpConfig,
    max_connections: Option<usize>,
) -> Result<(), WireError> {
    let mut handled = 0usize;
    loop {
        if max_connections.is_some_and(|max| handled >= max) {
            break;
        }
        let (stream, _) = listener.accept()?;
        handled += 1;
        match serve_stream_encrypted(stream, &resolver, config) {
            Ok(()) => {}
            Err(
                WireError::Encryption(_)
                | WireError::BadEncryptedFrame
                | WireError::BadEncryptedCapability,
            ) => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn serve_one_encrypted(
    listener: TcpListener,
    resolver: impl PageResolver,
    config: &EncryptedTcpConfig,
) -> Result<(), WireError> {
    let (stream, _) = listener.accept()?;
    serve_stream_encrypted(stream, &resolver, config)
}

pub fn serve_stream(mut stream: TcpStream, resolver: &impl PageResolver) -> Result<(), WireError> {
    let request = read_wire_request(&mut stream)?;
    let response = resolver.fetch(&request)?;
    write_response(&mut stream, &response)?;
    Ok(())
}

pub fn serve_stream_encrypted(
    mut stream: TcpStream,
    resolver: &impl PageResolver,
    config: &EncryptedTcpConfig,
) -> Result<(), WireError> {
    let plaintext = read_encrypted_frame(&mut stream, config, EncryptedFrameDirection::Request)?;
    let request = read_wire_request_bytes(&plaintext)?;
    let response = resolver.fetch(&request)?;
    let mut response_bytes = Vec::new();
    write_response(&mut response_bytes, &response)?;
    write_encrypted_frame(
        &mut stream,
        config,
        EncryptedFrameDirection::Response,
        &response_bytes,
    )?;
    Ok(())
}

pub fn fetch_tcp(url: &JaringanUrl) -> Result<Response, WireError> {
    fetch_tcp_with_timeout(url, Duration::from_secs(30))
}

pub fn post_tcp(url: &JaringanUrl, body: String) -> Result<Response, WireError> {
    post_tcp_with_timeout(url, body, Duration::from_secs(30))
}

/// Like `post_tcp` but with a configurable timeout.
pub fn post_tcp_with_timeout(
    url: &JaringanUrl,
    body: String,
    timeout: Duration,
) -> Result<Response, WireError> {
    send_tcp_with_timeout(Request::post(url.clone(), body), timeout)
}

pub fn post_tcp_with_action_token(
    url: &JaringanUrl,
    body: String,
    token: impl Into<String>,
) -> Result<Response, WireError> {
    send_tcp_with_timeout(
        Request::post(url.clone(), body).with_action_token(token),
        Duration::from_secs(30),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedTcpConfig {
    pub capability: EncryptionCapability,
    pub key: EncryptionKey,
}

impl EncryptedTcpConfig {
    pub fn new(key_id: impl Into<String>, key: EncryptionKey) -> Self {
        Self {
            capability: EncryptionCapability::xchacha20_poly1305(key_id),
            key,
        }
    }
}

pub fn fetch_tcp_encrypted(
    url: &JaringanUrl,
    config: &EncryptedTcpConfig,
) -> Result<Response, WireError> {
    fetch_tcp_encrypted_with_timeout(url, config, Duration::from_secs(30))
}

pub fn fetch_tcp_encrypted_with_timeout(
    url: &JaringanUrl,
    config: &EncryptedTcpConfig,
    timeout: Duration,
) -> Result<Response, WireError> {
    send_tcp_encrypted_with_timeout(Request::new(url.clone()), config, timeout)
}

pub fn post_tcp_encrypted(
    url: &JaringanUrl,
    body: String,
    config: &EncryptedTcpConfig,
) -> Result<Response, WireError> {
    send_tcp_encrypted_with_timeout(
        Request::post(url.clone(), body),
        config,
        Duration::from_secs(30),
    )
}

pub fn fetch_tcp_with_timeout(url: &JaringanUrl, timeout: Duration) -> Result<Response, WireError> {
    send_tcp_with_timeout(Request::new(url.clone()), timeout)
}

/// Resolve a host:port pair to a socket address with a timeout.
/// Prevents indefinite blocking on slow/malicious DNS servers.
fn resolve_addr(host: &str, port: u16, timeout: Duration) -> Result<std::net::SocketAddr, WireError> {
    let host = host.to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send((host.as_str(), port).to_socket_addrs());
    });
    let mut addrs = rx
        .recv_timeout(timeout)
        .map_err(|_| WireError::BadAddress)?
        .map_err(|_| WireError::BadAddress)?;
    addrs.next().ok_or(WireError::BadAddress)
}

// ── Base TCP fetch ────────────────────────────────────────────────────

fn send_tcp_with_timeout(request: Request, timeout: Duration) -> Result<Response, WireError> {
    let addr = resolve_addr(request.url.host(), request.url.port_or_default(), timeout)?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write_wire_request(&mut stream, &request)?;
    read_response(&mut stream)
}

// ── Streaming ─────────────────────────────────────────────────────────

/// Read just the JRG response header (status + tag headers), leaving the
/// body in the stream for incremental reading.
fn read_response_header(reader: &mut BufReader<TcpStream>) -> Result<Response, WireError> {
    let mut header_lines: Vec<String> = Vec::new();
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Err(WireError::BadResponse);
        }
        if line == "\n" || line == "\r\n" {
            break;
        }
        header_lines.push(line.trim_end().to_owned());
    }

    let status_line = header_lines.first().ok_or(WireError::BadResponse)?;
    let mut status_parts = status_line.split_whitespace();
    if status_parts.next() != Some("JRG/0.1") {
        return Err(WireError::BadResponse);
    }
    let code = status_parts
        .next()
        .ok_or(WireError::BadResponse)?
        .parse::<u16>()
        .map_err(|_| WireError::BadResponse)?;
    let status = StatusCode::from_u16(code).ok_or(WireError::BadResponse)?;

    let mut content_type = None;
    let mut tags = Vec::new();
    for line in &header_lines[1..] {
        if let Some(value) = line.strip_prefix("Content-Type:") {
            content_type = ContentType::from_header(value.trim());
        } else if let Some(value) = line.strip_prefix("Tag-Redirect:") {
            tags.push(ResponseTag::Redirect {
                target: value.trim().to_owned(),
            });
        } else if line.trim() == "Tag-Stream: true" {
            tags.push(ResponseTag::Stream);
        } else if let Some(value) = line.strip_prefix("Tag-Key:") {
            let value = value.trim();
            if let Some((key_id, key_data)) = value.split_once(' ') {
                if let Some(key_base64) = key_data.strip_prefix("ed25519:") {
                    tags.push(ResponseTag::Key {
                        key_id: key_id.to_owned(),
                        key_base64: key_base64.to_owned(),
                    });
                }
            }
        } else if let Some(value) = line.strip_prefix("Tag-ContentType:") {
            tags.push(ResponseTag::ContentType {
                value: value.trim().to_owned(),
            });
        }
    }

    Ok(Response {
        status,
        content_type: content_type.ok_or(WireError::BadResponse)?,
        tags,
        body: String::new(),
    })
}

/// A streaming JRG connection that keeps the TCP socket open for
/// incremental blocks. Blocks are separated by `.\n` on a line by itself.
pub struct StreamConnection {
    /// The initial response (status, content-type, tags).  The body is
    /// initially empty; call `read_block()` to read the first one.
    pub response: Response,
    reader: BufReader<TcpStream>,
}

impl StreamConnection {
    /// Read the next block of JRG content.  Returns `Ok(None)` when the
    /// server closes the connection.  Blocks are separated by `.\n` on a
    /// line by itself; everything up to `.\n` or EOF is one block.
    pub fn read_block(&mut self) -> Result<Option<String>, WireError> {
        let mut block = String::new();
        loop {
            let mut line = String::new();
            let bytes = self.reader.read_line(&mut line)?;
            if bytes == 0 {
                return if block.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(block))
                };
            }
            if line.trim_end() == "." {
                return Ok(Some(block));
            }
            block.push_str(&line);
        }
    }
}

/// Fetch a JRG URL and keep the TCP connection alive for streaming.
/// The server must respond with `Tag-Stream: true` (or `Content-Type:
/// text/jrg-stream`).  Returns the header-only response and a
/// `StreamConnection` for reading incremental blocks.
pub fn fetch_tcp_stream(url: &JaringanUrl) -> Result<StreamConnection, WireError> {
    fetch_tcp_stream_with_timeout(url, Duration::from_secs(30))
}

/// Like `fetch_tcp_stream` but with a configurable timeout.
pub fn fetch_tcp_stream_with_timeout(
    url: &JaringanUrl,
    timeout: Duration,
) -> Result<StreamConnection, WireError> {
    let request = Request::new(url.clone());
    let addr = resolve_addr(request.url.host(), request.url.port_or_default(), timeout)?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    // Long read timeout for streaming; will be signalled by connection close
    stream.set_read_timeout(Some(Duration::from_secs(300)))?;
    stream.set_write_timeout(Some(timeout))?;
    write_wire_request(&mut stream, &request)?;
    let mut reader = BufReader::new(stream);
    let response = read_response_header(&mut reader)?;
    Ok(StreamConnection { response, reader })
}

fn send_tcp_encrypted_with_timeout(
    request: Request,
    config: &EncryptedTcpConfig,
    timeout: Duration,
) -> Result<Response, WireError> {
    let addr = resolve_addr(request.url.host(), request.url.port_or_default(), timeout)?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let request_bytes = wire_request_bytes(&request)?;
    write_encrypted_frame(
        &mut stream,
        config,
        EncryptedFrameDirection::Request,
        &request_bytes,
    )?;
    let response_bytes =
        read_encrypted_frame(&mut stream, config, EncryptedFrameDirection::Response)?;
    read_response(&mut io::Cursor::new(response_bytes))
}

fn write_wire_request(writer: &mut impl Write, request: &Request) -> io::Result<()> {
    writeln!(
        writer,
        "{} {} JRG/0.1",
        request.method.as_str(),
        request.url.as_str()
    )?;
    writeln!(writer, "Host: {}", request.url.authority())?;
    if let Some(token) = &request.action_token {
        writeln!(writer, "Action-Token: {token}")?;
    }
    if !request.body.is_empty() {
        writeln!(writer, "Content-Length: {}", request.body.len())?;
    }
    writeln!(writer)?;
    write!(writer, "{}", request.body)?;
    writer.flush()
}

fn wire_request_bytes(request: &Request) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    write_wire_request(&mut bytes, request)?;
    Ok(bytes)
}

fn read_wire_request(stream: &mut TcpStream) -> Result<Request, WireError> {
    let mut reader = BufReader::new(stream);
    read_wire_request_from_reader(&mut reader)
}

fn read_wire_request_bytes(bytes: &[u8]) -> Result<Request, WireError> {
    let mut reader = BufReader::new(io::Cursor::new(bytes));
    read_wire_request_from_reader(&mut reader)
}

fn read_wire_request_from_reader(reader: &mut impl BufRead) -> Result<Request, WireError> {
    let mut first_line = String::new();
    reader.read_line(&mut first_line)?;
    let mut parts = first_line.split_whitespace();
    let method = RequestMethod::parse(parts.next().ok_or(WireError::BadRequest)?)
        .ok_or(WireError::BadRequest)?;
    let target = parts.next().ok_or(WireError::BadRequest)?;

    let mut host = String::new();
    let mut action_token = None;
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Host:") {
            host = value.trim().to_owned();
        } else if let Some(value) = trimmed.strip_prefix("Action-Token:") {
            action_token = Some(value.trim().to_owned());
        } else if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse().map_err(|_| WireError::BadRequest)?;
        }
    }

    let url = if target.starts_with("jrg://") {
        JaringanUrl::parse(target)?
    } else if target.starts_with('/') && !host.is_empty() {
        JaringanUrl::parse(&format!("jrg://{host}{target}"))?
    } else {
        return Err(WireError::BadRequest);
    };

    if content_length > MAX_REQUEST_BODY {
        return Err(WireError::PayloadTooLarge);
    }

    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    let body = String::from_utf8(body).map_err(|_| WireError::BadRequest)?;

    Ok(Request {
        method,
        url,
        body,
        action_token,
    })
}

pub fn write_response(writer: &mut impl Write, response: &Response) -> io::Result<()> {
    writeln!(
        writer,
        "JRG/0.1 {} {}",
        response.status.as_u16(),
        response.status.reason_phrase()
    )?;
    writeln!(writer, "Content-Type: {}", response.content_type.as_str())?;
    for tag in &response.tags {
        match tag {
            ResponseTag::Redirect { target } => writeln!(writer, "Tag-Redirect: {target}")?,
            ResponseTag::Stream => writeln!(writer, "Tag-Stream: true")?,
            ResponseTag::Key {
                key_id,
                key_base64,
            } => writeln!(writer, "Tag-Key: {key_id} ed25519:{key_base64}")?,
            ResponseTag::ContentType { value } => {
                writeln!(writer, "Tag-ContentType: {value}")?
            }
        }
    }
    writeln!(writer)?;
    write!(writer, "{}", response.body)?;
    writer.flush()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EncryptedFrameDirection {
    Request,
    Response,
}

impl EncryptedFrameDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Response => "response",
        }
    }
}

fn encrypted_frame_aad(config: &EncryptedTcpConfig, direction: EncryptedFrameDirection) -> Vec<u8> {
    format!(
        "JRG-ENC/0.1 {}; {}",
        direction.as_str(),
        config.capability.to_header_value()
    )
    .into_bytes()
}

fn random_encryption_nonce() -> EncryptionNonce {
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut bytes = [0; 24];
    bytes.copy_from_slice(&nonce);
    EncryptionNonce::from_bytes(bytes)
}

fn write_encrypted_frame(
    writer: &mut impl Write,
    config: &EncryptedTcpConfig,
    direction: EncryptedFrameDirection,
    plaintext: &[u8],
) -> Result<(), WireError> {
    let payload = encrypt_payload(
        &config.key,
        random_encryption_nonce(),
        plaintext,
        &encrypted_frame_aad(config, direction),
    )?;
    writeln!(writer, "JRG-ENC/0.1")?;
    writeln!(
        writer,
        "Content-Encryption: {}",
        config.capability.to_header_value()
    )?;
    writeln!(writer, "Nonce: {}", payload.nonce_base64)?;
    writeln!(
        writer,
        "Content-Length: {}",
        payload.ciphertext_base64.len()
    )?;
    writeln!(writer)?;
    write!(writer, "{}", payload.ciphertext_base64)?;
    writer.flush()?;
    Ok(())
}

fn read_encrypted_frame(
    reader: &mut impl Read,
    config: &EncryptedTcpConfig,
    direction: EncryptedFrameDirection,
) -> Result<Vec<u8>, WireError> {
    let mut reader = BufReader::new(reader);
    let mut first_line = String::new();
    reader.read_line(&mut first_line)?;
    if first_line.trim_end() != "JRG-ENC/0.1" {
        return Err(WireError::BadEncryptedFrame);
    }

    let mut capability = None;
    let mut nonce_base64 = None;
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Encryption:") {
            capability = Some(EncryptionCapability::from_header_value(value.trim())?);
        } else if let Some(value) = trimmed.strip_prefix("Nonce:") {
            nonce_base64 = Some(value.trim().to_owned());
        } else if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| WireError::BadEncryptedFrame)?,
            );
        }
    }

    let capability = capability.ok_or(WireError::BadEncryptedFrame)?;
    if capability != config.capability {
        return Err(WireError::BadEncryptedCapability);
    }
    let nonce_base64 = nonce_base64.ok_or(WireError::BadEncryptedFrame)?;
    let content_length = content_length.ok_or(WireError::BadEncryptedFrame)?;
    if content_length > MAX_REQUEST_BODY {
        return Err(WireError::PayloadTooLarge);
    }
    let mut ciphertext = vec![0; content_length];
    reader.read_exact(&mut ciphertext)?;
    let ciphertext_base64 =
        String::from_utf8(ciphertext).map_err(|_| WireError::BadEncryptedFrame)?;
    let payload = EncryptedPayload {
        suite: capability.suite,
        nonce_base64,
        ciphertext_base64,
    };
    Ok(decrypt_payload(
        &config.key,
        &payload,
        &encrypted_frame_aad(config, direction),
    )?)
}

pub fn read_response(reader: &mut impl Read) -> Result<Response, WireError> {
    let mut input = String::new();
    reader.read_to_string(&mut input)?;
    let (headers, body) = input.split_once("\n\n").ok_or(WireError::BadResponse)?;
    let mut lines = headers.lines();
    let status_line = lines.next().ok_or(WireError::BadResponse)?;
    let mut status_parts = status_line.split_whitespace();
    if status_parts.next() != Some("JRG/0.1") {
        return Err(WireError::BadResponse);
    }
    let code = status_parts
        .next()
        .ok_or(WireError::BadResponse)?
        .parse::<u16>()
        .map_err(|_| WireError::BadResponse)?;
    let status = StatusCode::from_u16(code).ok_or(WireError::BadResponse)?;

    let mut content_type = None;
    let mut tags = Vec::new();
    for line in lines {
        if let Some(value) = line.strip_prefix("Content-Type:") {
            content_type = ContentType::from_header(value.trim());
        } else if let Some(value) = line.strip_prefix("Tag-Redirect:") {
            tags.push(ResponseTag::Redirect {
                target: value.trim().to_owned(),
            });
        } else if line.trim() == "Tag-Stream: true" {
            tags.push(ResponseTag::Stream);
        } else if let Some(value) = line.strip_prefix("Tag-Key:") {
            let value = value.trim();
            if let Some((key_id, key_data)) = value.split_once(' ') {
                if let Some(key_base64) = key_data.strip_prefix("ed25519:") {
                    tags.push(ResponseTag::Key {
                        key_id: key_id.to_owned(),
                        key_base64: key_base64.to_owned(),
                    });
                }
            }
        } else if let Some(value) = line.strip_prefix("Tag-ContentType:") {
            tags.push(ResponseTag::ContentType {
                value: value.trim().to_owned(),
            });
        }
    }

    Ok(Response {
        status,
        content_type: content_type.ok_or(WireError::BadResponse)?,
        tags,
        body: body.to_owned(),
    })
}

#[derive(Debug, Error)]
pub enum WireError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("URL error: {0}")]
    Url(#[from] UrlError),
    #[error("resolver error: {0}")]
    Resolve(#[from] ResolveError),
    #[error("bad Jaringan wire request")]
    BadRequest,
    #[error("bad Jaringan wire response")]
    BadResponse,
    #[error("bad encrypted Jaringan frame")]
    BadEncryptedFrame,
    #[error("encrypted Jaringan frame does not match configured capability")]
    BadEncryptedCapability,
    #[error("encryption error: {0}")]
    Encryption(#[from] EncryptionError),
    #[error("could not resolve Jaringan host")]
    BadAddress,
    #[error("request body exceeds maximum allowed size ({MAX_REQUEST_BODY} bytes)")]
    PayloadTooLarge,
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

    #[test]
    fn tcp_client_fetches_page_from_single_request_server() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("index.jrg"), "# TCP Home\n").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let resolver = LocalFileResolver::new(root.path());
        let server = std::thread::spawn(move || serve_one(listener, resolver).unwrap());

        let response = fetch_tcp(&JaringanUrl::parse(&format!("jrg://{addr}/")).unwrap()).unwrap();
        server.join().unwrap();

        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.content_type, ContentType::JaringanPage);
        assert_eq!(response.body, "# TCP Home\n");
    }

    #[test]
    fn tcp_client_times_out_when_server_does_not_respond() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(250));
        });

        let started = std::time::Instant::now();
        let error = fetch_tcp_with_timeout(
            &JaringanUrl::parse(&format!("jrg://{addr}/")).unwrap(),
            std::time::Duration::from_millis(25),
        )
        .unwrap_err();
        server.join().unwrap();

        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert!(
            matches!(error, WireError::Io(error) if error.kind() == io::ErrorKind::WouldBlock || error.kind() == io::ErrorKind::TimedOut)
        );
    }

    #[test]
    fn encrypted_payload_round_trips_with_xchacha20_poly1305() {
        let key = EncryptionKey::from_bytes([3; 32]);
        let nonce = EncryptionNonce::from_bytes([4; 24]);
        let payload =
            encrypt_payload(&key, nonce, b"# Secret page\n", b"jrg://example.org/").unwrap();

        assert_eq!(payload.suite, EncryptionSuite::XChaCha20Poly1305);
        assert_ne!(payload.ciphertext_base64, "# Secret page\n");
        assert_eq!(
            decrypt_payload(&key, &payload, b"jrg://example.org/").unwrap(),
            b"# Secret page\n"
        );
    }

    #[test]
    fn encrypted_tcp_fetch_round_trips_without_plaintext_on_wire() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("index.jrg"), "# Encrypted TCP Home\n").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let config = EncryptedTcpConfig::new("local-dev", EncryptionKey::from_bytes([42; 32]));
        let server_config = config.clone();
        let resolver = LocalFileResolver::new(root.path());
        let server = std::thread::spawn(move || {
            serve_one_encrypted(listener, resolver, &server_config).unwrap()
        });

        let response = fetch_tcp_encrypted(
            &JaringanUrl::parse(&format!("jrg://{addr}/")).unwrap(),
            &config,
        )
        .unwrap();
        server.join().unwrap();

        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.body, "# Encrypted TCP Home\n");
    }

    #[test]
    fn encrypted_tcp_rejects_wrong_key() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("index.jrg"), "# Secret\n").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server_config =
            EncryptedTcpConfig::new("local-dev", EncryptionKey::from_bytes([1; 32]));
        let client_config =
            EncryptedTcpConfig::new("local-dev", EncryptionKey::from_bytes([2; 32]));
        let resolver = LocalFileResolver::new(root.path());
        let server = std::thread::spawn(move || {
            serve_one_encrypted(listener, resolver, &server_config).unwrap_err()
        });

        let error = fetch_tcp_encrypted(
            &JaringanUrl::parse(&format!("jrg://{addr}/")).unwrap(),
            &client_config,
        )
        .unwrap_err();
        let server_error = server.join().unwrap();

        assert!(matches!(
            server_error,
            WireError::Encryption(EncryptionError::Decrypt)
        ));
        assert!(matches!(
            error,
            WireError::Io(_) | WireError::BadResponse | WireError::BadEncryptedFrame
        ));
    }

    #[test]
    fn encrypted_server_keeps_serving_after_bad_client_key() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("index.jrg"), "# Still Serving\n").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server_config =
            EncryptedTcpConfig::new("local-dev", EncryptionKey::from_bytes([7; 32]));
        let bad_client_config =
            EncryptedTcpConfig::new("local-dev", EncryptionKey::from_bytes([8; 32]));
        let good_client_config = server_config.clone();
        let resolver = LocalFileResolver::new(root.path());
        let server = std::thread::spawn(move || {
            serve_encrypted_connections(listener, resolver, &server_config, Some(2)).unwrap()
        });

        let bad_result = fetch_tcp_encrypted(
            &JaringanUrl::parse(&format!("jrg://{addr}/")).unwrap(),
            &bad_client_config,
        );
        assert!(bad_result.is_err());

        let response = fetch_tcp_encrypted(
            &JaringanUrl::parse(&format!("jrg://{addr}/")).unwrap(),
            &good_client_config,
        )
        .unwrap();
        server.join().unwrap();

        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.body, "# Still Serving\n");
    }

    #[test]
    fn encryption_capabilities_serialize_to_headers() {
        let capability = EncryptionCapability::xchacha20_poly1305("key-2026");

        assert_eq!(
            capability.to_header_value(),
            "xchacha20poly1305; key-id=key-2026"
        );
        assert_eq!(
            EncryptionCapability::from_header_value("xchacha20poly1305; key-id=key-2026").unwrap(),
            capability
        );
    }

    #[derive(Clone)]
    struct EchoPostResolver;

    impl PageResolver for EchoPostResolver {
        fn fetch(&self, request: &Request) -> Result<Response, ResolveError> {
            assert_eq!(request.method, RequestMethod::Post);
            assert_eq!(request.url.path(), "/actions/search");
            assert_eq!(request.body, "q=laksa");
            Ok(Response::page(StatusCode::Ok, "# Action received\n"))
        }
    }

    #[test]
    fn tcp_client_posts_action_payload_to_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || serve_one(listener, EchoPostResolver).unwrap());

        let response = post_tcp(
            &JaringanUrl::parse(&format!("jrg://{addr}/actions/search")).unwrap(),
            "q=laksa".to_owned(),
        )
        .unwrap();
        server.join().unwrap();

        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.body, "# Action received\n");
    }

    #[test]
    fn local_resolver_handles_demo_search_action_and_records_side_effect() {
        let root =
            std::env::temp_dir().join(format!("jaringan-action-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let resolver = LocalFileResolver::new(&root);

        let response = resolver
            .fetch(
                &Request::post(
                    JaringanUrl::parse("jrg://localhost/actions/search").unwrap(),
                    "q=laksa".to_owned(),
                )
                .with_action_token("demo-search"),
            )
            .unwrap();

        assert_eq!(response.status, StatusCode::Ok);
        assert!(response.body.contains("# Search Results"));
        let log = fs::read_to_string(root.join(".jrg-actions.log")).unwrap();
        assert!(log.contains("POST /actions/search q=laksa"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn local_resolver_rejects_post_actions_without_capability_token() {
        let root =
            std::env::temp_dir().join(format!("jaringan-action-auth-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let resolver = LocalFileResolver::new(&root);

        let response = resolver
            .fetch(&Request::post(
                JaringanUrl::parse("jrg://localhost/actions/search").unwrap(),
                "q=laksa".to_owned(),
            ))
            .unwrap();

        assert_eq!(response.status, StatusCode::Forbidden);
        assert!(
            response
                .body
                .contains("missing or invalid action capability token")
        );
        assert!(!root.join(".jrg-actions.log").exists());
        let _ = fs::remove_dir_all(&root);
    }
}

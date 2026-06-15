use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use jaringan_core::{IndexEntryV1, SearchIndexV1, PublicKeyring, SignatureStatus,
    verify_source_signature, source_metadata, metadata_value};

use base64::Engine;

use crate::pages;

// ---------------------------------------------------------------------------
// Submission state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Submission {
    pub domain: String,
    pub token: String,
    pub created_at: String,
    pub verified: bool,
}

// ---------------------------------------------------------------------------
// Search engine state
// ---------------------------------------------------------------------------

pub struct SearchEngine {
    pub data_dir: PathBuf,
    pub domain: String,
    pub port: u16,
    pub index: Mutex<SearchIndexV1>,
    pub submissions: Mutex<HashMap<String, Submission>>,
    pub snippets: Mutex<HashMap<String, String>>,
    /// If true, execute WASM scripts on pages during verification/indexing
    /// so the index reflects rendered content, not raw page source.
    pub execute_scripts: bool,
}

impl SearchEngine {
    pub fn new(data_dir: impl Into<PathBuf>, domain: String, port: u16) -> Self {
        let data_dir = data_dir.into();
        let index = Self::load_index(&data_dir);
        let submissions = Self::load_submissions(&data_dir);
        Self {
            data_dir,
            domain,
            port,
            index: Mutex::new(index),
            submissions: Mutex::new(submissions),
            snippets: Mutex::new(HashMap::new()),
            execute_scripts: false,
        }
    }

    fn load_index(data_dir: &Path) -> SearchIndexV1 {
        let path = data_dir.join("index.jrgidx");
        if path.exists() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(index) = SearchIndexV1::from_index_text_v1(&text) {
                    return index;
                }
            }
        }
        SearchIndexV1::default()
    }

    fn load_submissions(data_dir: &Path) -> HashMap<String, Submission> {
        let path = data_dir.join("submissions.json");
        if path.exists() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(subs) = serde_json::from_str(&text) {
                    return subs;
                }
            }
        }
        HashMap::new()
    }

    pub fn save_state(&self) {
        let _ = std::fs::create_dir_all(&self.data_dir);

        let index_text = self.index.lock().unwrap().to_index_text_v1();
        let _ = std::fs::write(self.data_dir.join("index.jrgidx"), index_text);

        let subs = &*self.submissions.lock().unwrap();
        if let Ok(json) = serde_json::to_string_pretty(subs) {
            let _ = std::fs::write(self.data_dir.join("submissions.json"), json);
        }
    }

    /// Generate a random hex verification token
    pub fn generate_token() -> String {
        let bytes: [u8; 16] = rand::random();
        hex::encode(bytes)
    }

    /// Submit a domain for verification
    pub fn submit_domain(&self, domain: &str) -> String {
        let domain = domain.trim().to_lowercase();
        let token = Self::generate_token();
        let submission = Submission {
            domain: domain.clone(),
            token: token.clone(),
            created_at: iso_now(),
            verified: false,
        };
        self.submissions.lock().unwrap().insert(domain.clone(), submission);
        self.save_state();
        token
    }

    /// Check if a submission exists and isn't yet verified
    pub fn get_pending(&self, domain: &str) -> Option<Submission> {
        let subs = self.submissions.lock().unwrap();
        subs.get(domain).filter(|s| !s.verified).cloned()
    }

    /// Mark a submission as verified and add validated entries to the index
    pub fn verify_and_index(
        &self,
        domain: &str,
        entries: Vec<IndexEntryV1>,
    ) -> usize {
        // Mark verified
        {
            let mut subs = self.submissions.lock().unwrap();
            if let Some(s) = subs.get_mut(domain) {
                s.verified = true;
            }
        }

        let count = entries.len();
        {
            let mut index = self.index.lock().unwrap();
            for entry in entries {
                index.add(entry);
            }
        }

        self.save_state();
        count
    }

    /// Get index stats
    pub fn stats(&self) -> (usize, usize) {
        let total_pages = self.index.lock().unwrap().entries().len();
        let subs = self.submissions.lock().unwrap();
        let verified_domains = subs.values().filter(|s| s.verified).count();
        (total_pages, verified_domains)
    }

    /// Get verified domains for periodic re-indexing
    pub fn verified_domains(&self) -> Vec<String> {
        let subs = self.submissions.lock().unwrap();
        subs.values()
            .filter(|s| s.verified)
            .map(|s| s.domain.clone())
            .collect()
    }

    /// Remove a pending (unverified) submission
    pub fn purge_domain(&self, domain: &str) -> bool {
        let removed = {
            let mut subs = self.submissions.lock().unwrap();
            subs.remove(domain).is_some()
        };
        if removed {
            self.save_state();
            true
        } else {
            false
        }
    }

    /// Store a text snippet for a verified page URL
    pub fn store_snippet(&self, url: &str, snippet: String) {
        self.snippets.lock().unwrap().insert(url.to_string(), snippet);
    }
}

// ---------------------------------------------------------------------------
// PageResolver implementation
// ---------------------------------------------------------------------------

use jaringan_protocol::{PageResolver, Request, RequestMethod, ResolveError, Response, StatusCode};

fn parse_form_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    for pair in body.split('&') {
        let mut parts = pair.splitn(2, '=');
        let k = parts.next()?;
        let v = parts.next()?;
        if k == key && !v.is_empty() {
            return Some(v);
        }
    }
    None
}

fn iso_now() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    // Approximate ISO 8601 from Unix epoch (works from 2020-2030)
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Approximate year/month/day from days since epoch
    // Good enough for timestamps; exact date math is complex
    let year = 1970 + (days as f64 / 365.25) as u64;
    let ydays = days as i64 - ((year - 1970) as i64 * 365 + ((year - 1970) / 4) as i64);
    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u64;
    let mut remaining = ydays;
    for &md in &month_days {
        if remaining < md as i64 {
            break;
        }
        remaining -= md as i64;
        month += 1;
    }
    let day = remaining.max(1);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

impl PageResolver for SearchEngine {
    fn fetch(&self, request: &Request) -> Result<Response, ResolveError> {
        let path = request.url.path();

        match (request.method, path) {
            // GET / — Search form
            (RequestMethod::Get, p) if p == "/" || p == "/search.jrg" => {
                let query = request.url.query().unwrap_or("");
                let q = if let Some(qs) = query.strip_prefix("q=") {
                    urlencoding_decode(qs)
                } else {
                    String::new()
                };

                if q.is_empty() {
                    Ok(Response::page(StatusCode::Ok, pages::SEARCH_FORM.to_string()))
                } else {
                    self.render_search_results(&q)
                }
            }

            // GET /submit.jrg — Submission form
            (RequestMethod::Get, p) if p == "/submit.jrg" => {
                Ok(Response::page(StatusCode::Ok, pages::SUBMIT_FORM.to_string()))
            }

            // GET /status.jrg — Engine status
            (RequestMethod::Get, p) if p == "/status.jrg" => {
                let (total_pages, verified_domains) = self.stats();
                let page = pages::status_page(total_pages, verified_domains, &self.domain, self.port);
                Ok(Response::page(StatusCode::Ok, page))
            }

            // GET /actions/verify — Shows verify page with optional status
            (RequestMethod::Get, p) if p.starts_with("/actions/verify") => {
                let domain = parse_query_param(request.url.query().unwrap_or(""), "domain");
                match domain {
                    Some(d) => match self.get_pending(&d) {
                        Some(sub) => {
                            let page = pages::verify_instruction_page(
                                &d, &sub.token, &format!("_jrg-verify.{d}"),
                            );
                            Ok(Response::page(StatusCode::Ok, page))
                        }
                        None => {
                            let page = format!(
                                "# Domain Not Found\n\nNo pending submission for **{d}**.\n\n=> /submit.jrg Back to submission\n\n~~~\ntitle: Domain Not Found\n~~~"
                            );
                            Ok(Response::page(StatusCode::Ok, page))
                        }
                    },
                    None => Ok(Response::page(StatusCode::Ok, pages::VERIFY_FORM.to_string())),
                }
            }

            // POST /actions/submit — Submit a domain
            (RequestMethod::Post, "/actions/submit") => {
                let domain = parse_form_value(&request.body, "domain")
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();

                if domain.is_empty() || !domain.contains('.') {
                    return Ok(Response::page(
                        StatusCode::Ok,
                        pages::error_page("Invalid domain. Please enter a valid domain name like `example.com`."),
                    ));
                }

                let token = self.submit_domain(&domain);
                let verify_record = format!("_jrg-verify.{domain}");
                let page = pages::verification_pending_page(&domain, &token, &verify_record);
                Ok(Response::page(StatusCode::Ok, page))
            }

            // POST /actions/check-verify — Check DNS + fetch jrgidx + index
            (RequestMethod::Post, "/actions/check-verify") => {
                let domain = parse_form_value(&request.body, "domain")
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();

                match self.get_pending(&domain) {
                    Some(sub) => {
                        let verified = check_dns_txt(&domain, &sub.token);
                        if verified {
                            // DNS verified — now fetch and index the jrgidx
                            match fetch_remote_jrgidx(&domain) {
                                Ok(entries) => {
                                    // Validate entries (format + crypto)
                                    let total = entries.len();
                                    let mut valid = Vec::new();
                                    for entry in entries {
                                        if validate_entry(&entry) {
                                            match verify_page_signature(&entry, self.execute_scripts) {
                                                Ok(snippet) => {
                                                    self.store_snippet(&entry.url, snippet);
                                                    valid.push(entry);
                                                }
                                                Err(_) => {}
                                            }
                                        }
                                    }
                                    let page_count = valid.len();
                                    let skip_count = total - page_count;

                                    let count = self.verify_and_index(&domain, valid);
                                    let page = pages::verified_and_indexed_page(
                                        &domain,
                                        count,
                                        skip_count,
                                        &iso_now(),
                                    );
                                    Ok(Response::page(StatusCode::Ok, page))
                                }
                                Err(msg) => {
                                    // DNS verified but jrgidx fetch failed
                                    // Still mark as verified domain, just no pages yet
                                    let _ = self.verify_and_index(&domain, Vec::new());
                                    let page = format!(
                                        "# ✅ Domain Verified — No jrgidx Found\n\n**{domain}** is DNS-verified, but we couldn't fetch a jrgidx from it.\n\nMake sure your server serves `/.well-known/jrgidx` as:\n- `jrg://{domain}:7070/.well-known/jrgidx` (JRG TCP)\n- `https://{domain}/.well-known/jrgidx` (HTTPS)\n- `http://{domain}/.well-known/jrgidx` (HTTP, fallback)\n\nError: {msg}\n\n=> /status.jrg View Index Status\n\n~~~\ntitle: Domain Verified — No jrgidx\n~~~"
                                    );
                                    Ok(Response::page(StatusCode::Ok, page))
                                }
                            }
                        } else {
                            let verify_record = format!("_jrg-verify.{domain}");
                            let page = format!(
                                "# ⏳ Verification Pending\n\nDNS TXT record not yet found for `{verify_record}`.\n\nMake sure you've created the TXT record with value:\n\n> `{token}`\n\nThen try again.\n\n=> /submit.jrg Back to Submission\n\n~~~\ntitle: Verification Pending\n~~~",
                                token = sub.token
                            );
                            Ok(Response::page(StatusCode::Ok, page))
                        }
                    }
                    None => {
                        let page = format!(
                            "# Domain Not Found\n\nNo pending submission for **{domain}**. Please submit first.\n\n=> /submit.jrg Submit Your Site\n\n~~~\ntitle: Domain Not Found\n~~~"
                        );
                        Ok(Response::page(StatusCode::Ok, page))
                    }
                }
            }

            // POST /actions/search — Search
            (RequestMethod::Post, "/actions/search") => {
                let q = parse_form_value(&request.body, "q").unwrap_or("");
                self.render_search_results(q)
            }

            // POST /actions/purge — Remove a pending submission
            (RequestMethod::Post, "/actions/purge") => {
                let domain = parse_form_value(&request.body, "domain")
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();

                if domain.is_empty() {
                    return Ok(Response::page(
                        StatusCode::Ok,
                        pages::error_page("No domain specified. Use `domain=example.com` to purge a submission."),
                    ));
                }

                if self.purge_domain(&domain) {
                    let page = format!(
                        "# ✅ Submission Purged\n\n**{domain}** has been removed from the submission queue.\n\n=> /submit.jrg Submit a new domain\n=> /status.jrg View Index Status\n\n~~~\ntitle: Submission Purged\n~~~"
                    );
                    Ok(Response::page(StatusCode::Ok, page))
                } else {
                    Ok(Response::page(
                        StatusCode::Ok,
                        pages::error_page(&format!("No pending submission found for **{domain}**.")),
                    ))
                }
            }

            _ => Ok(Response::page(
                StatusCode::NotFound,
                pages::not_found_page(),
            )),
        }
    }
}

impl SearchEngine {
    fn render_search_results(&self, query: &str) -> Result<Response, ResolveError> {
        let q = query.trim().to_string();
        if q.is_empty() {
            return Ok(Response::page(StatusCode::Ok, pages::SEARCH_FORM.to_string()));
        }

        let index = self.index.lock().unwrap();
        let results = index.search(&q);

        let page = if results.is_empty() {
            format!(
                "# No Results for \"{q}\"\n\nNo pages found. Try different search terms.\n\n=> /search.jrg?q={q} Back to Search\n\n~~~\ntitle: No Results\n~~~"
            )
        } else {
            let mut body = String::new();
            body.push_str(&format!("# Search Results for \"{q}\"\n\n**{} result(s) found.**\n\n---\n\n", results.len()));

            for (i, result) in results.iter().enumerate() {
                let entry = result.entry;
                let snippet = match self.snippets.lock().unwrap().get(&entry.url) {
                    Some(s) if !s.is_empty() => s.clone(),
                    _ => entry.title.clone(),
                };
                body.push_str(&format!(
                    "### {}. **{}**\n\n=> {} Click to visit\n\n_{}_\n\n---\n\n",
                    i + 1,
                    entry.title,
                    entry.url,
                    snippet,
                ));
            }

            body.push_str("=> /search.jrg New Search\n");
            body.push_str("=> /submit.jrg Submit Your Site\n");
            body.push_str(&format!("\n~~~\ntitle: Search Results for \"{q}\"\n~~~"));

            body
        };

        Ok(Response::page(StatusCode::Ok, page))
    }
}

// ---------------------------------------------------------------------------
// Remote jrgidx fetch
// ---------------------------------------------------------------------------

/// Try to fetch and parse a remote jrgidx from a domain.
/// Tries HTTPS first, then HTTP.
fn fetch_remote_jrgidx(domain: &str) -> Result<Vec<IndexEntryV1>, String> {
    // Try HTTPS
    let https_url = format!("https://{domain}/.well-known/jrgidx");
    match fetch_url_text(&https_url) {
        Ok(text) => {
            let index = SearchIndexV1::from_index_text_v1(&text)
                .map_err(|e| format!("bad jrgidx format from {https_url}: {e}"))?;
            return Ok(index.entries().to_vec());
        }
        Err(_) => { /* fall through to HTTP */ }
    }

    // Try HTTP
    let http_url = format!("http://{domain}/.well-known/jrgidx");
    match fetch_url_text(&http_url) {
        Ok(text) => {
            let index = SearchIndexV1::from_index_text_v1(&text)
                .map_err(|e| format!("bad jrgidx format from {http_url}: {e}"))?;
            return Ok(index.entries().to_vec());
        }
        Err(e) => {
            return Err(format!(
                "could not fetch jrgidx from {domain}. Tried HTTPS and HTTP. Last error: {e}"
            ));
        }
    }
}

/// Fetch a URL and return the response text.
/// Uses reqwest's blocking client with a 10-second timeout.
fn fetch_url_text(url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("JRG-Search-Engine/0.1")
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    resp.text().map_err(|e| format!("read body: {e}"))
}

// ---------------------------------------------------------------------------
// Entry validation
// ---------------------------------------------------------------------------

/// Validate a jrgidx entry's signature and encryption fields.
/// Checks format validity (doesn't fetch the actual page).
pub fn validate_entry(entry: &IndexEntryV1) -> bool {
    // 1. Public key must be ed25519:<base64> format, exactly 32 bytes decoded
    if !entry.public_key.starts_with("ed25519:") {
        return false;
    }
    let pk_b64 = &entry.public_key["ed25519:".len()..];
    let _pk_bytes = match base64::engine::general_purpose::STANDARD.decode(pk_b64) {
        Ok(b) if b.len() == 32 => b,
        _ => return false,
    };

    // 2. Signature must be ed25519:<base64> format, exactly 64 bytes decoded
    if !entry.signature.starts_with("ed25519:") {
        return false;
    }
    let sig_b64 = &entry.signature["ed25519:".len()..];
    let _sig_bytes = match base64::engine::general_purpose::STANDARD.decode(sig_b64) {
        Ok(b) if b.len() == 64 => b,
        _ => return false,
    };

    // 3. Encryption must declare a known suite
    let known_suites = ["xchacha20poly1305", "chacha20poly1305"];
    let has_known_suite = known_suites.iter().any(|s| entry.encryption.starts_with(s));
    if !has_known_suite {
        return false;
    }

    // 4. URL must be a valid jrg:// URL
    let is_valid_jrg = entry.url.starts_with("jrg://")
        && entry.url.len() > 7
        && entry.url.contains('/');

    // 5. Title must be non-empty
    let has_title = !entry.title.is_empty();

    is_valid_jrg && has_title
}

// ---------------------------------------------------------------------------
// Cryptographic page signature verification
// ---------------------------------------------------------------------------

/// Convert a jrg:// URL to an https:// URL for fetching
fn jrg_to_http_url(jrg_url: &str) -> Result<String, String> {
    let without_scheme = jrg_url.strip_prefix("jrg://")
        .ok_or_else(|| format!("not a jrg:// URL: {jrg_url}"))?;

    // Use HTTP for localhost (for testing), HTTPS otherwise
    if without_scheme.starts_with("127.0.0.1") || without_scheme.starts_with("localhost") {
        Ok(format!("http://{without_scheme}"))
    } else {
        Ok(format!("https://{without_scheme}"))
    }
}

/// Fetch and cryptographically verify a page's signature against the
/// public key declared in its jrgidx entry.
///
/// If `execute_scripts` is true, WASM scripts on the page are executed
/// before snippet extraction, so the index reflects rendered content.
///
/// Returns Ok(snippet) if the page has a valid Ed25519 signature that verifies
/// against the declared public key, along with a text snippet from the page body.
/// Returns Err(reason) otherwise.
pub fn verify_page_signature(entry: &IndexEntryV1, execute_scripts: bool) -> Result<String, String> {
    // 1. Convert jrg:// URL to an HTTP URL we can fetch
    let http_url = jrg_to_http_url(&entry.url)?;

    // 2. Fetch the page text
    let page_text = fetch_url_text(&http_url)?;

    // 3. Optionally execute WASM scripts so the snippet reflects rendered content
    let page_text = if execute_scripts {
        render_page_with_scripts(&page_text)
    } else {
        page_text
    };

    // 4. Extract snippet from page body (before metadata)
    let snippet = extract_snippet(&page_text);

    // 5. Extract the signer name from the page's `signed-by:` metadata
    let metadata = source_metadata(&page_text)
        .ok_or_else(|| "page has no metadata section (no ~~~ delimiters)".to_string())?;
    let signer_name = metadata_value(metadata, "signed-by")
        .ok_or_else(|| "page has no signed-by: metadata".to_string())?;
    let _page_sig = metadata_value(metadata, "signature")
        .ok_or_else(|| "page has signed-by but no signature: metadata".to_string())?;

    // 5. Build a PublicKeyring with the key from the jrgidx entry
    let pk_b64 = entry.public_key.strip_prefix("ed25519:").unwrap_or("");
    let mut keyring = PublicKeyring::default();
    keyring
        .add_ed25519_key(signer_name, pk_b64)
        .map_err(|e| format!("invalid public key for {}: {}", entry.url, e))?;

    // 6. Verify
    match verify_source_signature(&page_text, &keyring) {
        SignatureStatus::Secure { .. } => Ok(snippet),
        SignatureStatus::Unsigned => Err("page is unsigned (no valid signed-by/signature)".into()),
        SignatureStatus::UnknownSigner { .. } => Err("signer key not in declared keyring".into()),
        SignatureStatus::Invalid { reason } => Err(format!("signature invalid: {reason}")),
    }
}

/// Optionally render a page by executing its WASM scripts, returning the
/// rendered page text. If there are no scripts or execution fails, returns
/// the original text.
///
/// This lets the index reflect script-processed content (e.g. enriched
/// data, injected forms) rather than raw page source.
fn render_page_with_scripts(page_text: &str) -> String {
    use jaringan_core::parse_document;
    use jaringan_script::WasmRuntime;

    let doc = match parse_document(page_text) {
        Ok(doc) => doc,
        Err(_) => return page_text.to_string(),
    };

    // Check if there are any Script blocks
    let has_scripts = doc.blocks.iter().any(|b| {
        matches!(b, jaringan_core::Block::Script { .. })
    });
    if !has_scripts {
        return page_text.to_string();
    }

    let runtime = match WasmRuntime::new() {
        Ok(r) => r,
        Err(_) => return page_text.to_string(),
    };

    match jaringan_script::execute_document_scripts(&runtime, &doc, None) {
        Ok(rendered_blocks) => {
            // Convert rendered blocks to plain text for snippet extraction
            let mut text = String::new();
            for block in &rendered_blocks {
                match block {
                    jaringan_core::Block::Heading { text: t, .. }
                    | jaringan_core::Block::Paragraph(t) => {
                        text.push_str(t);
                        text.push(' ');
                    }
                    jaringan_core::Block::List(items) => {
                        for item in items {
                            text.push_str(item);
                            text.push(' ');
                        }
                    }
                    _ => {}
                }
            }
            if text.is_empty() {
                page_text.to_string()
            } else {
                text
            }
        }
        Err(_) => page_text.to_string(),
    }
}

/// Takes the first non-heading, non-link, non-empty paragraph.
fn extract_snippet(page_text: &str) -> String {
    // Strip metadata section (everything after ~~~~~)
    let body = page_text.split("\n~~~~~\n").next().unwrap_or(page_text);

    // Find first substantial paragraph
    for line in body.lines() {
        let trimmed = line.trim();
        // Skip empty lines, headings, links, buttons, inputs, rules, code fences, lists
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("=>")
            || trimmed.starts_with('!')
            || trimmed.starts_with('?')
            || trimmed.starts_with("---")
            || trimmed.starts_with("```")
            || trimmed.starts_with('-')
            || trimmed.starts_with('*')
            || trimmed.starts_with('|')
            || trimmed.starts_with('>')
            || trimmed.starts_with('@')
        {
            continue;
        }
        if trimmed.len() > 20 {
            // Truncate to ~150 chars with ellipsis
            if trimmed.len() > 150 {
                let mut s = trimmed[..147].to_string();
                s.push_str("...");
                return s;
            }
            return trimmed.to_string();
        }
    }

    // Fall back to empty snippet
    String::new()
}

// ---------------------------------------------------------------------------
// Periodic re-indexing
// ---------------------------------------------------------------------------

/// Start a background thread that re-indexes verified domains every N hours.
/// The interval defaults to 6 hours.
pub fn start_periodic_reindex(engine: Arc<SearchEngine>) {
    let interval_hours = 6;
    let interval = Duration::from_secs(interval_hours * 3600);

    let engine_for_thread = engine.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(interval);

        eprintln!("🔄 Periodic re-index starting...");

        // Read verified domains from the engine's in-memory state
        let current_domains = engine_for_thread.verified_domains();

        if current_domains.is_empty() {
            eprintln!("🔄 No verified domains to re-index");
            continue;
        }

        let mut all_entries = Vec::new();
        for domain in &current_domains {
            eprintln!("🔄 Re-indexing {domain}...");
            match fetch_remote_jrgidx(domain) {
                Ok(entries) => {
                    let mut valid = Vec::new();
                    for entry in entries {
                        if validate_entry(&entry) {
                            match verify_page_signature(&entry, engine_for_thread.execute_scripts) {
                                Ok(snippet) => {
                                    engine_for_thread.store_snippet(&entry.url, snippet);
                                    valid.push(entry);
                                }
                                Err(_) => {}
                            }
                        }
                    }
                    eprintln!("🔄   {domain}: {} valid entries", valid.len());
                    for entry in valid {
                        all_entries.push(entry);
                    }
                }
                Err(e) => {
                    eprintln!("🔄   {domain}: fetch failed: {e}");
                }
            }
        }

        // Update in-memory index
        {
            let mut new_index = SearchIndexV1::default();
            for entry in all_entries {
                new_index.add(entry);
            }
            eprintln!("🔄 Periodic re-index complete: {} total pages", new_index.entries().len());
            *engine_for_thread.index.lock().unwrap() = new_index;
        }

        // Persist to disk
        engine_for_thread.save_state();
    });
}

// ---------------------------------------------------------------------------
// DNS verification
// ---------------------------------------------------------------------------

fn check_dns_txt(domain: &str, expected_token: &str) -> bool {
    use hickory_resolver::config::{ResolverConfig, ResolverOpts};
    use hickory_resolver::Resolver;

    let lookup_name = format!("_jrg-verify.{domain}");

    let resolver = match Resolver::new(ResolverConfig::default(), ResolverOpts::default()) {
        Ok(r) => r,
        Err(_) => return false,
    };

    let lookup = match resolver.txt_lookup(&lookup_name) {
        Ok(l) => l,
        Err(_) => return false,
    };

    for record in lookup.iter() {
        let txt = record.to_string();
        let clean = txt.trim_matches('"').trim();
        if clean == expected_token {
            return true;
        }
    }

    false
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

fn parse_query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let k = parts.next()?;
        let v = parts.next()?;
        if k == key && !v.is_empty() {
            return Some(urlencoding_decode(v));
        }
    }
    None
}

fn urlencoding_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

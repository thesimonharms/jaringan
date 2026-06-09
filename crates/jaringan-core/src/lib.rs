use std::{collections::BTreeMap, fmt};

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub metadata: Option<String>,
}

impl Document {
    pub fn new(blocks: Vec<Block>) -> Self {
        Self {
            blocks,
            metadata: None,
        }
    }

    pub fn with_metadata(blocks: Vec<Block>, metadata: Option<String>) -> Self {
        Self { blocks, metadata }
    }

    pub fn title(&self) -> Option<&str> {
        self.metadata_title().or_else(|| {
            self.blocks.iter().find_map(|block| match block {
                Block::Heading { level: 1, text } => Some(text.as_str()),
                _ => None,
            })
        })
    }

    pub fn metadata_title(&self) -> Option<&str> {
        let metadata = self.metadata.as_ref()?;
        metadata.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case("title")
                .then_some(value.trim())
                .filter(|value| !value.is_empty())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading { level: u8, text: String },
    Paragraph(String),
    Link(Link),
    Input(Input),
    Button(Button),
    Image(Image),
    Quote(String),
    List(Vec<String>),
    Rule,
    Table(Table),
    Preformatted(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub target: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub name: String,
    pub label: String,
    pub value: String,
    pub placeholder: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionMethod {
    Get,
    Post,
}

impl ActionMethod {
    fn parse(input: &str) -> Option<Self> {
        match input.to_ascii_uppercase().as_str() {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub id: String,
    pub label: String,
    pub target: String,
    pub method: ActionMethod,
    pub confirm: Option<String>,
    pub auth: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub source: String,
    pub alt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureStatus {
    Secure { signer: String },
    Unsigned,
    UnknownSigner { signer: String },
    Invalid { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PublicKeyring {
    ed25519_keys: BTreeMap<String, VerifyingKey>,
}

impl PublicKeyring {
    pub fn add_ed25519_key(
        &mut self,
        signer: impl Into<String>,
        key_base64: &str,
    ) -> Result<(), String> {
        let key_bytes = base64::engine::general_purpose::STANDARD
            .decode(key_base64)
            .map_err(|error| format!("bad ed25519 public key base64: {error}"))?;
        let key_bytes: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| String::from("ed25519 public keys must be 32 bytes"))?;
        let verifying_key = VerifyingKey::from_bytes(&key_bytes)
            .map_err(|error| format!("bad ed25519 public key: {error}"))?;
        self.ed25519_keys.insert(signer.into(), verifying_key);
        Ok(())
    }

    pub fn from_text(source: &str) -> Result<Self, String> {
        let mut keyring = Self::default();
        for (index, raw_line) in source.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() != 2 {
                return Err(format!(
                    "line {line_number}: expected `<signer> ed25519:<base64-public-key>`"
                ));
            }
            let (signer, key) = (parts[0], parts[1]);
            let Some(key_base64) = key.strip_prefix("ed25519:") else {
                return Err(format!(
                    "line {line_number}: expected `<signer> ed25519:<base64-public-key>`"
                ));
            };
            keyring
                .add_ed25519_key(signer, key_base64)
                .map_err(|error| format!("line {line_number}: {error}"))?;
        }
        Ok(keyring)
    }

    fn ed25519_key(&self, signer: &str) -> Option<&VerifyingKey> {
        self.ed25519_keys.get(signer)
    }
}

pub fn verify_source_signature(source: &str, keyring: &PublicKeyring) -> SignatureStatus {
    let Some(metadata) = source_metadata(source) else {
        return SignatureStatus::Unsigned;
    };
    let Some(signer) = metadata_value(metadata, "signed-by") else {
        return SignatureStatus::Unsigned;
    };
    let Some(signature_value) = metadata_value(metadata, "signature") else {
        return SignatureStatus::Invalid {
            reason: format!("signed-by `{signer}` is present without a signature"),
        };
    };
    let Some(signature_base64) = signature_value.strip_prefix("ed25519:") else {
        return SignatureStatus::Invalid {
            reason: String::from("signature must use ed25519:<base64>"),
        };
    };
    let Some(verifying_key) = keyring.ed25519_key(signer) else {
        return SignatureStatus::UnknownSigner {
            signer: signer.to_owned(),
        };
    };
    let signature_bytes = match base64::engine::general_purpose::STANDARD.decode(signature_base64) {
        Ok(bytes) => bytes,
        Err(error) => {
            return SignatureStatus::Invalid {
                reason: format!("bad signature base64: {error}"),
            };
        }
    };
    let signature_bytes: [u8; 64] = match signature_bytes.try_into() {
        Ok(bytes) => bytes,
        Err(_) => {
            return SignatureStatus::Invalid {
                reason: String::from("ed25519 signatures must be 64 bytes"),
            };
        }
    };
    let signature = Signature::from_bytes(&signature_bytes);
    match verifying_key.verify(canonical_signature_payload(source).as_bytes(), &signature) {
        Ok(()) => SignatureStatus::Secure {
            signer: signer.to_owned(),
        },
        Err(error) => SignatureStatus::Invalid {
            reason: format!("signature verification failed: {error}"),
        },
    }
}

pub fn canonical_signature_payload(source: &str) -> String {
    let Some((body, metadata)) = source.split_once("~~~~~") else {
        return source.to_owned();
    };
    let metadata_without_signature = metadata
        .lines()
        .filter(|line| {
            line.split_once(':')
                .is_none_or(|(key, _)| !key.trim().eq_ignore_ascii_case("signature"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{body}~~~~~\n{}\n", metadata_without_signature.trim())
}

fn source_metadata(source: &str) -> Option<&str> {
    source.split_once("~~~~~").map(|(_, metadata)| metadata)
}

fn metadata_value<'a>(metadata: &'a str, key: &str) -> Option<&'a str> {
    metadata.lines().find_map(|line| {
        let (candidate, value) = line.split_once(':')?;
        candidate
            .trim()
            .eq_ignore_ascii_case(key)
            .then_some(value.trim())
            .filter(|value| !value.is_empty())
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchLink {
    pub target: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchEntry {
    pub url: String,
    pub title: String,
    pub headings: Vec<String>,
    pub links: Vec<SearchLink>,
    pub metadata: Option<String>,
    pub body: String,
}

impl SearchEntry {
    pub fn from_document(url: impl Into<String>, document: &Document) -> Self {
        let mut headings = Vec::new();
        let mut links = Vec::new();
        let mut body_parts = Vec::new();
        for block in &document.blocks {
            match block {
                Block::Heading { text, .. } => headings.push(text.clone()),
                Block::Link(link) => links.push(SearchLink {
                    target: link.target.clone(),
                    label: link.label.clone(),
                }),
                Block::Paragraph(text) | Block::Preformatted(text) | Block::Quote(text) => {
                    body_parts.push(text.clone())
                }
                Block::List(items) => body_parts.push(items.join("\n")),
                Block::Table(table) => body_parts.push(table_text(table)),
                Block::Rule => {}
                Block::Input(input) => body_parts.push(format!("{} {}", input.label, input.value)),
                Block::Button(button) => body_parts.push(button.label.clone()),
                Block::Image(image) => body_parts.push(image.alt.clone()),
            }
        }
        let title = document.title().unwrap_or("Untitled").to_owned();
        Self {
            url: url.into(),
            title,
            headings,
            links,
            metadata: document.metadata.clone(),
            body: body_parts.join("\n"),
        }
    }

    pub fn search_text(&self) -> String {
        let link_text = self
            .links
            .iter()
            .map(|link| format!("{} {}", link.label, link.target))
            .collect::<Vec<_>>()
            .join(" ");
        [
            self.title.as_str(),
            &self.headings.join(" "),
            &link_text,
            self.metadata.as_deref().unwrap_or_default(),
            &self.body,
        ]
        .join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult<'a> {
    pub entry: &'a SearchEntry,
    pub score: usize,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchIndex {
    entries: Vec<SearchEntry>,
}

impl SearchIndex {
    pub fn add(&mut self, entry: SearchEntry) {
        self.entries.push(entry);
    }

    pub fn entries(&self) -> &[SearchEntry] {
        &self.entries
    }

    pub fn search(&self, query: &str) -> Vec<SearchResult<'_>> {
        let tokens = query_tokens(query);
        if tokens.is_empty() {
            return Vec::new();
        }
        let mut results = self
            .entries
            .iter()
            .filter_map(|entry| score_entry(entry, &tokens))
            .collect::<Vec<_>>();
        results.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.entry.title.cmp(&right.entry.title))
                .then_with(|| left.entry.url.cmp(&right.entry.url))
        });
        results
    }

    pub fn to_index_text(&self) -> String {
        let mut output = String::from("JRG-SEARCH/0.1\n");
        for entry in &self.entries {
            let headings = entry
                .headings
                .iter()
                .map(|heading| escape_index_field(heading))
                .collect::<Vec<_>>()
                .join("\u{1f}");
            let links = entry
                .links
                .iter()
                .map(|link| {
                    format!(
                        "{}\u{1e}{}",
                        escape_index_field(&link.target),
                        escape_index_field(&link.label)
                    )
                })
                .collect::<Vec<_>>()
                .join("\u{1f}");
            output.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                escape_index_field(&entry.url),
                escape_index_field(&entry.title),
                headings,
                links,
                entry
                    .metadata
                    .as_ref()
                    .map(|metadata| escape_index_field(metadata))
                    .unwrap_or_default(),
                escape_index_field(&entry.body)
            ));
        }
        output
    }

    pub fn from_index_text(input: &str) -> Result<Self, String> {
        let mut lines = input.lines();
        match lines.next() {
            Some("JRG-SEARCH/0.1") => {}
            Some(other) => return Err(format!("unsupported search index header: {other}")),
            None => return Err(String::from("empty search index")),
        }

        let mut index = SearchIndex::default();
        for (line_number, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() != 6 {
                return Err(format!(
                    "bad search index entry at line {}: expected 6 fields",
                    line_number + 2
                ));
            }
            let headings = split_escaped_list(fields[2], '\u{1f}')?;
            let links = if fields[3].is_empty() {
                Vec::new()
            } else {
                fields[3]
                    .split('\u{1f}')
                    .map(|link| {
                        let (target, label) = link
                            .split_once('\u{1e}')
                            .ok_or_else(|| format!("bad link field at line {}", line_number + 2))?;
                        Ok(SearchLink {
                            target: unescape_index_field(target)?,
                            label: unescape_index_field(label)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?
            };
            index.add(SearchEntry {
                url: unescape_index_field(fields[0])?,
                title: unescape_index_field(fields[1])?,
                headings,
                links,
                metadata: (!fields[4].is_empty())
                    .then(|| unescape_index_field(fields[4]))
                    .transpose()?,
                body: unescape_index_field(fields[5])?,
            });
        }
        Ok(index)
    }
}

fn score_entry<'a>(entry: &'a SearchEntry, tokens: &[String]) -> Option<SearchResult<'a>> {
    let mut score = 0;
    let mut snippet = None;
    score += score_field(&entry.title, tokens, 10, &mut snippet);
    score += score_fields(&entry.headings, tokens, 5, &mut snippet);
    for link in &entry.links {
        score += score_field(&link.label, tokens, 3, &mut snippet);
        score += score_field(&link.target, tokens, 3, &mut snippet);
    }
    if let Some(metadata) = &entry.metadata {
        score += score_multiline_field(metadata, tokens, 1, &mut snippet);
    }
    score += score_multiline_field(&entry.body, tokens, 1, &mut snippet);
    (score > 0).then(|| SearchResult {
        entry,
        score,
        snippet: snippet.unwrap_or_else(|| entry.title.clone()),
    })
}

fn score_fields(
    fields: &[String],
    tokens: &[String],
    weight: usize,
    snippet: &mut Option<String>,
) -> usize {
    fields
        .iter()
        .map(|field| score_field(field, tokens, weight, snippet))
        .sum()
}

fn score_multiline_field(
    field: &str,
    tokens: &[String],
    weight: usize,
    snippet: &mut Option<String>,
) -> usize {
    field
        .lines()
        .flat_map(sentence_snippets)
        .map(|line| score_field(&line, tokens, weight, snippet))
        .sum()
}

fn sentence_snippets(line: &str) -> Vec<String> {
    let mut snippets = Vec::new();
    let mut start = 0usize;
    for (index, _) in line.match_indices(". ") {
        let end = index + 1;
        let snippet = line[start..end].trim();
        if !snippet.is_empty() {
            snippets.push(snippet.to_owned());
        }
        start = index + 2;
    }
    let tail = line[start..].trim();
    if !tail.is_empty() {
        snippets.push(tail.to_owned());
    }
    snippets
}

fn score_field(
    field: &str,
    tokens: &[String],
    weight: usize,
    snippet: &mut Option<String>,
) -> usize {
    let haystack = field.to_ascii_lowercase();
    if tokens.iter().all(|token| haystack.contains(token)) {
        snippet.get_or_insert_with(|| field.to_owned());
        tokens.len() * weight
    } else {
        0
    }
}

fn query_tokens(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn split_escaped_list(input: &str, separator: char) -> Result<Vec<String>, String> {
    if input.is_empty() {
        return Ok(Vec::new());
    }
    input.split(separator).map(unescape_index_field).collect()
}

fn escape_index_field(input: &str) -> String {
    let mut escaped = String::new();
    for ch in input.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            '\u{1f}' => escaped.push_str("\\u001f"),
            '\u{1e}' => escaped.push_str("\\u001e"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn unescape_index_field(input: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => output.push('\n'),
            Some('t') => output.push('\t'),
            Some('\\') => output.push('\\'),
            Some('u') => {
                let code = chars.by_ref().take(4).collect::<String>();
                match code.as_str() {
                    "001f" => output.push('\u{1f}'),
                    "001e" => output.push('\u{1e}'),
                    _ => return Err(format!("unsupported escape: \\u{code}")),
                }
            }
            Some(other) => return Err(format!("unsupported escape: \\{other}")),
            None => return Err(String::from("trailing escape in search index field")),
        }
    }
    Ok(output)
}

fn table_text(table: &Table) -> String {
    table
        .headers
        .iter()
        .chain(table.rows.iter().flatten())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("unterminated preformatted block starting at line {line}")]
    UnterminatedPreformatted { line: usize },
}

pub fn parse_document(input: &str) -> Result<Document, ParseError> {
    let mut parser = Parser::new(input);
    parser.parse()
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    cursor: usize,
    blocks: Vec<Block>,
    metadata: Option<String>,
}

impl<'a> Parser<'a> {
    /// Returns true if `trimmed` starts a block type that terminates paragraphs.
    fn is_block_start(trimmed: &str) -> bool {
        trimmed.starts_with("```")
            || trimmed.starts_with('|')
            || trimmed.starts_with('>')
            || is_list_item(trimmed)
            || is_rule(trimmed)
            || parse_heading(trimmed).is_some()
            || parse_link(trimmed).is_some()
            || parse_input(trimmed).is_some()
            || parse_button(trimmed).is_some()
            || parse_image(trimmed).is_some()
            || trimmed == "~~~~~"
    }

    fn new(input: &'a str) -> Self {
        Self {
            lines: input.lines().collect(),
            cursor: 0,
            blocks: Vec::new(),
            metadata: None,
        }
    }

    fn parse(&mut self) -> Result<Document, ParseError> {
        while let Some(line) = self.peek() {
            let trimmed = line.trim();

            if trimmed == "~~~~~" {
                self.parse_metadata();
                break;
            }

            if trimmed.is_empty() {
                self.cursor += 1;
                continue;
            }

            if trimmed.starts_with("```") {
                self.parse_preformatted()?;
            } else if trimmed.starts_with('|') {
                self.parse_table();
            } else if trimmed.starts_with('>') {
                self.parse_quote();
            } else if is_list_item(trimmed) {
                self.parse_list();
            } else if is_rule(trimmed) {
                self.blocks.push(Block::Rule);
                self.cursor += 1;
            } else if let Some(block) = parse_heading(trimmed) {
                self.blocks.push(block);
                self.cursor += 1;
            } else if let Some(link) = parse_link(trimmed) {
                self.blocks.push(Block::Link(link));
                self.cursor += 1;
            } else if let Some(input) = parse_input(trimmed) {
                self.blocks.push(Block::Input(input));
                self.cursor += 1;
            } else if let Some(button) = parse_button(trimmed) {
                self.blocks.push(Block::Button(button));
                self.cursor += 1;
            } else if let Some(image) = parse_image(trimmed) {
                self.blocks.push(Block::Image(image));
                self.cursor += 1;
            } else {
                self.parse_paragraph();
            }
        }

        Ok(Document::with_metadata(
            std::mem::take(&mut self.blocks),
            self.metadata.take(),
        ))
    }

    fn peek(&self) -> Option<&'a str> {
        self.lines.get(self.cursor).copied()
    }

    fn parse_preformatted(&mut self) -> Result<(), ParseError> {
        let start_line = self.cursor + 1;
        self.cursor += 1;
        let mut body = Vec::new();

        while let Some(line) = self.peek() {
            if line.trim().starts_with("```") {
                self.cursor += 1;
                self.blocks.push(Block::Preformatted(body.join("\n")));
                return Ok(());
            }
            body.push(line);
            self.cursor += 1;
        }

        Err(ParseError::UnterminatedPreformatted { line: start_line })
    }

    fn parse_table(&mut self) {
        let mut rows = Vec::new();
        while let Some(line) = self.peek() {
            let trimmed = line.trim();
            if !trimmed.starts_with('|') {
                break;
            }
            let cells = parse_table_row(trimmed);
            if !is_table_separator(&cells) {
                rows.push(cells);
            }
            self.cursor += 1;
        }

        if rows.is_empty() {
            return;
        }
        let headers = rows.remove(0);
        self.blocks.push(Block::Table(Table { headers, rows }));
    }

    fn parse_quote(&mut self) {
        let mut lines = Vec::new();
        while let Some(line) = self.peek() {
            let trimmed = line.trim();
            let Some(text) = trimmed.strip_prefix('>') else {
                break;
            };
            lines.push(text.trim().to_owned());
            self.cursor += 1;
        }
        self.blocks.push(Block::Quote(lines.join("\n")));
    }

    fn parse_list(&mut self) {
        let mut items = Vec::new();
        while let Some(line) = self.peek() {
            let trimmed = line.trim();
            let Some(item) = list_item_text(trimmed) else {
                break;
            };
            items.push(item.to_owned());
            self.cursor += 1;
        }
        self.blocks.push(Block::List(items));
    }

    fn parse_metadata(&mut self) {
        self.cursor += 1;
        let metadata = self.lines[self.cursor..].join("\n");
        let metadata = metadata.trim().to_owned();
        self.metadata = (!metadata.is_empty()).then_some(metadata);
        self.cursor = self.lines.len();
    }

    fn parse_paragraph(&mut self) {
        let mut lines = Vec::new();

        while let Some(line) = self.peek() {
            let trimmed = line.trim();
            if trimmed.is_empty() || Self::is_block_start(trimmed) {
                break;
            }
            lines.push(trimmed);
            self.cursor += 1;
        }

        self.blocks.push(Block::Paragraph(lines.join(" ")));
    }
}

fn parse_heading(line: &str) -> Option<Block> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }

    let text = line.get(hashes..)?.strip_prefix(' ')?;
    Some(Block::Heading {
        level: hashes as u8,
        text: text.trim().to_owned(),
    })
}

fn is_rule(line: &str) -> bool {
    matches!(line, "---" | "***" | "___")
}

fn is_list_item(line: &str) -> bool {
    list_item_text(line).is_some()
}

fn list_item_text(line: &str) -> Option<&str> {
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

fn parse_table_row(line: &str) -> Vec<String> {
    line.trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

fn is_table_separator(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let trimmed = cell.trim();
            trimmed.len() >= 3
                && trimmed.chars().all(|ch| matches!(ch, '-' | ':' | ' '))
                && trimmed.chars().any(|ch| ch == '-')
        })
}

fn parse_link(line: &str) -> Option<Link> {
    let remainder = line.strip_prefix("=>")?.trim();
    if remainder.is_empty() {
        return None;
    }

    let mut parts = remainder.splitn(2, char::is_whitespace);
    let target = parts.next()?.trim().to_owned();
    let label = parts
        .next()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(&target)
        .to_owned();

    Some(Link { target, label })
}

fn parse_button(line: &str) -> Option<Button> {
    let remainder = line.strip_prefix('!')?.trim();
    let mut parts = remainder.splitn(2, char::is_whitespace);
    let id = parts.next()?.trim();
    if id.is_empty() {
        return None;
    }
    let attrs = parts.next().unwrap_or_default();
    let label = parse_quoted_attr(attrs, "label").unwrap_or_else(|| id.to_owned());
    let target = parse_quoted_attr(attrs, "target").unwrap_or_else(|| id.to_owned());
    let method = parse_quoted_attr(attrs, "method")
        .as_deref()
        .and_then(ActionMethod::parse)
        .unwrap_or(ActionMethod::Get);
    let confirm = parse_quoted_attr(attrs, "confirm");
    let auth = parse_quoted_attr(attrs, "auth");

    Some(Button {
        id: id.to_owned(),
        label,
        target,
        method,
        confirm,
        auth,
    })
}

fn parse_input(line: &str) -> Option<Input> {
    let remainder = line.strip_prefix('?')?.trim();
    let mut parts = remainder.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim();
    if name.is_empty() {
        return None;
    }
    let attrs = parts.next().unwrap_or_default();
    let label = parse_quoted_attr(attrs, "label").unwrap_or_else(|| name.to_owned());
    let value = parse_quoted_attr(attrs, "value").unwrap_or_default();
    let placeholder = parse_quoted_attr(attrs, "placeholder");

    Some(Input {
        name: name.to_owned(),
        label,
        value,
        placeholder,
    })
}

fn parse_image(line: &str) -> Option<Image> {
    let remainder = line.strip_prefix('@')?.trim();
    let mut parts = remainder.splitn(2, char::is_whitespace);
    let source = parts.next()?.trim();
    if source.is_empty() {
        return None;
    }
    let attrs = parts.next().unwrap_or_default();
    let alt = parse_quoted_attr(attrs, "alt").unwrap_or_else(|| source.to_owned());

    Some(Image {
        source: source.to_owned(),
        alt,
    })
}

fn parse_quoted_attr(input: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = input.find(&needle)? + needle.len();
    let rest = &input[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} <{}>", self.label, self.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_heading_paragraph_and_link() {
        let doc = parse_document(
            "# Hello\n\nThis is a page\nfor terminals.\n\n=> jrg://example/about About us\n",
        )
        .unwrap();

        assert_eq!(doc.title(), Some("Hello"));
        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    text: "Hello".into()
                },
                Block::Paragraph("This is a page for terminals.".into()),
                Block::Link(Link {
                    target: "jrg://example/about".into(),
                    label: "About us".into()
                })
            ]
        );
    }

    #[test]
    fn parses_preformatted_blocks() {
        let doc = parse_document("# Code\n\n```plain\n  keep spacing\n```\n").unwrap();

        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    text: "Code".into()
                },
                Block::Preformatted("  keep spacing".into())
            ]
        );
    }

    #[test]
    fn reports_unterminated_preformatted_blocks() {
        let error = parse_document("# Broken\n\n```plain\nno end\n").unwrap_err();

        assert_eq!(error, ParseError::UnterminatedPreformatted { line: 3 });
    }

    #[test]
    fn parses_buttons_and_images() {
        let doc = parse_document(
            "# Rich\n\n! save label=\"Save this page\" target=\"save\"\n@ ./cover.png alt=\"Cover art\"\n",
        )
        .unwrap();

        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    text: "Rich".into()
                },
                Block::Button(Button {
                    id: "save".into(),
                    label: "Save this page".into(),
                    target: "save".into(),
                    method: ActionMethod::Get,
                    confirm: None,
                    auth: None
                }),
                Block::Image(Image {
                    source: "./cover.png".into(),
                    alt: "Cover art".into()
                })
            ]
        );
    }

    #[test]
    fn parses_structured_inputs_and_action_buttons() {
        let doc = parse_document(
            "# Search\n\n? q label=\"Query\" value=\"laksa\" placeholder=\"Restaurant name\"\n! submit label=\"Search\" method=\"POST\" target=\"/actions/search\" confirm=\"Submit search?\" auth=\"demo-search\"\n",
        )
        .unwrap();

        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    text: "Search".into()
                },
                Block::Input(Input {
                    name: "q".into(),
                    label: "Query".into(),
                    value: "laksa".into(),
                    placeholder: Some("Restaurant name".into())
                }),
                Block::Button(Button {
                    id: "submit".into(),
                    label: "Search".into(),
                    target: "/actions/search".into(),
                    method: ActionMethod::Post,
                    confirm: Some("Submit search?".into()),
                    auth: Some("demo-search".into())
                })
            ]
        );
    }

    #[test]
    fn parses_tables_quotes_lists_and_rules() {
        let doc = parse_document(
            "# Rich layout\n\n> Keep pages beautiful.\n> Even in terminals.\n\n- fast\n- calm\n- readable\n\n---\n\n| Name | Role |\n| --- | --- |\n| Simon | Builder |\n| Jaringan | Browser |\n",
        )
        .unwrap();

        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    text: "Rich layout".into()
                },
                Block::Quote("Keep pages beautiful.\nEven in terminals.".into()),
                Block::List(vec!["fast".into(), "calm".into(), "readable".into()]),
                Block::Rule,
                Block::Table(Table {
                    headers: vec!["Name".into(), "Role".into()],
                    rows: vec![
                        vec!["Simon".into(), "Builder".into()],
                        vec!["Jaringan".into(), "Browser".into()]
                    ]
                })
            ]
        );
    }

    #[test]
    fn parses_trailing_metadata_after_delimiter() {
        let doc = parse_document(
            "# Visible heading\n\nThis stays in the document.\n\n~~~~~\ntitle: Simon's page\ndate: 2026-06-07\nredirect: jrg://example.org/new.jrg\n",
        )
        .unwrap();

        assert_eq!(doc.title(), Some("Simon's page"));
        assert_eq!(
            doc.metadata.as_deref(),
            Some("title: Simon's page\ndate: 2026-06-07\nredirect: jrg://example.org/new.jrg")
        );
        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    text: "Visible heading".into()
                },
                Block::Paragraph("This stays in the document.".into())
            ]
        );
    }

    #[test]
    fn search_index_extracts_titles_headings_links_and_metadata() {
        let document = parse_document(
            "# Laksa guide\n\n## Penang stalls\n\n=> food/penang.jrg Penang food map\n\n~~~~~\ntitle: Street Food Index\ntags: laksa, hawker\n",
        )
        .unwrap();

        let entry = SearchEntry::from_document("jrg://local/laksa.jrg", &document);

        assert_eq!(entry.url, "jrg://local/laksa.jrg");
        assert_eq!(entry.title, "Street Food Index");
        assert_eq!(entry.headings, vec!["Laksa guide", "Penang stalls"]);
        assert_eq!(
            entry.links,
            vec![SearchLink {
                target: "food/penang.jrg".into(),
                label: "Penang food map".into(),
            }]
        );
        assert_eq!(
            entry.metadata.as_deref(),
            Some("title: Street Food Index\ntags: laksa, hawker")
        );
        assert!(entry.search_text().contains("hawker"));
    }

    #[test]
    fn search_index_returns_ranked_case_insensitive_matches() {
        let mut index = SearchIndex::default();
        index.add(SearchEntry {
            url: "jrg://local/penang.jrg".into(),
            title: "Penang Laksa".into(),
            headings: vec!["Hawker guide".into()],
            links: Vec::new(),
            metadata: None,
            body: String::new(),
        });
        index.add(SearchEntry {
            url: "jrg://local/coffee.jrg".into(),
            title: "Coffee".into(),
            headings: vec!["Laksa nearby".into()],
            links: Vec::new(),
            metadata: Some("tags: cafe".into()),
            body: String::new(),
        });

        let results = index.search("laksa");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].entry.url, "jrg://local/penang.jrg");
        assert!(results[0].score > results[1].score);
        assert_eq!(results[0].snippet, "Penang Laksa");
    }

    #[test]
    fn search_requires_all_query_tokens_and_snippets_matching_body_text() {
        let document = parse_document(
            "# Food notes\n\nThe best evening laksa stall is beside the blue market.\nCoffee appears elsewhere.\n",
        )
        .unwrap();
        let mut index = SearchIndex::default();
        index.add(SearchEntry::from_document(
            "jrg://local/food.jrg",
            &document,
        ));

        assert!(index.search("laksa satay").is_empty());
        let results = index.search("evening laksa");

        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].snippet,
            "The best evening laksa stall is beside the blue market."
        );
    }

    #[test]
    fn search_index_serializes_and_loads_from_text() {
        let mut index = SearchIndex::default();
        index.add(SearchEntry {
            url: "jrg://local/penang.jrg".into(),
            title: "Penang Laksa".into(),
            headings: vec!["Hawker guide".into()],
            links: vec![SearchLink {
                target: "../index.jrg".into(),
                label: "Back home".into(),
            }],
            metadata: Some("tags: laksa, hawker".into()),
            body: "A body with tabs\tand newlines\ninside.".into(),
        });

        let encoded = index.to_index_text();
        let decoded = SearchIndex::from_index_text(&encoded).unwrap();

        assert_eq!(decoded.entries(), index.entries());
        assert_eq!(decoded.search("hawker")[0].entry.title, "Penang Laksa");
    }

    #[test]
    fn unsigned_sources_are_not_secure_but_allowed() {
        let source = "# Plain page\n\nUnsigned pages are still valid Jaringan pages.\n";
        let keyring = PublicKeyring::default();

        assert_eq!(
            verify_source_signature(source, &keyring),
            SignatureStatus::Unsigned
        );
    }

    #[test]
    fn signed_sources_verify_against_public_keyring() {
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};

        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let verifying_key = signing_key.verifying_key();
        let unsigned = "# Signed page\n\nThis content is covered.\n\n~~~~~\ntitle: Signed page\nsigned-by: alice\n";
        let signature = signing_key.sign(canonical_signature_payload(unsigned).as_bytes());
        let source = format!(
            "{unsigned}signature: ed25519:{}\n",
            base64::engine::general_purpose::STANDARD.encode(signature.to_bytes())
        );
        let mut keyring = PublicKeyring::default();
        keyring
            .add_ed25519_key(
                "alice",
                &base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes()),
            )
            .unwrap();

        assert_eq!(
            verify_source_signature(&source, &keyring),
            SignatureStatus::Secure {
                signer: "alice".into()
            }
        );
    }

    #[test]
    fn keyring_text_parses_human_editable_ed25519_keys() {
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};

        let signing_key = SigningKey::from_bytes(&[9; 32]);
        let public_key = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes());
        let keyring = PublicKeyring::from_text(&format!(
            "# trusted signers\n\nalice ed25519:{public_key}\n"
        ))
        .unwrap();
        let unsigned =
            "# Signed page\n\nLoaded through a keyring file.\n\n~~~~~\nsigned-by: alice\n";
        let signature = signing_key.sign(canonical_signature_payload(unsigned).as_bytes());
        let source = format!(
            "{unsigned}signature: ed25519:{}\n",
            base64::engine::general_purpose::STANDARD.encode(signature.to_bytes())
        );

        assert_eq!(
            verify_source_signature(&source, &keyring),
            SignatureStatus::Secure {
                signer: "alice".into()
            }
        );
    }

    #[test]
    fn keyring_text_rejects_malformed_lines_with_line_numbers() {
        let valid_empty_key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let error = PublicKeyring::from_text(&format!("alice ed25519:{valid_empty_key}\nbroken\n"))
            .unwrap_err();

        assert!(error.contains("line 2"));
        assert!(error.contains("expected `<signer> ed25519:<base64-public-key>`"));
    }
}

use std::fmt;

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
    Button(Button),
    Image(Image),
    Preformatted(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub target: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub id: String,
    pub label: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub source: String,
    pub alt: String,
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
            } else if let Some(block) = parse_heading(trimmed) {
                self.blocks.push(block);
                self.cursor += 1;
            } else if let Some(link) = parse_link(trimmed) {
                self.blocks.push(Block::Link(link));
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
            if trimmed.is_empty()
                || trimmed.starts_with("```")
                || parse_heading(trimmed).is_some()
                || parse_link(trimmed).is_some()
                || parse_button(trimmed).is_some()
                || parse_image(trimmed).is_some()
                || trimmed == "~~~~~"
            {
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

    Some(Button {
        id: id.to_owned(),
        label,
        target,
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
                    target: "save".into()
                }),
                Block::Image(Image {
                    source: "./cover.png".into(),
                    alt: "Cover art".into()
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
}

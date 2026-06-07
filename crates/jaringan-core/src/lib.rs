use std::fmt;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
}

impl Document {
    pub fn new(blocks: Vec<Block>) -> Self {
        Self { blocks }
    }

    pub fn title(&self) -> Option<&str> {
        self.blocks.iter().find_map(|block| match block {
            Block::Heading { level: 1, text } => Some(text.as_str()),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading { level: u8, text: String },
    Paragraph(String),
    Link(Link),
    Preformatted(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub target: String,
    pub label: String,
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
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            lines: input.lines().collect(),
            cursor: 0,
            blocks: Vec::new(),
        }
    }

    fn parse(&mut self) -> Result<Document, ParseError> {
        while let Some(line) = self.peek() {
            let trimmed = line.trim();

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
            } else {
                self.parse_paragraph();
            }
        }

        Ok(Document::new(std::mem::take(&mut self.blocks)))
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

    fn parse_paragraph(&mut self) {
        let mut lines = Vec::new();

        while let Some(line) = self.peek() {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with("```")
                || parse_heading(trimmed).is_some()
                || parse_link(trimmed).is_some()
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
            "# Hello\n\nThis is a page\nfor terminals.\n\n=> jar://example/about About us\n",
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
                    target: "jar://example/about".into(),
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
}

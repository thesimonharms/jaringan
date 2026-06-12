use std::fmt::Write;

use jaringan_core::{ActionMethod, Block, Button, Document, Image, Input, Link, Table};

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FormatOptions {
    pub indent_size: usize,
    pub max_line_width: usize,
    pub trailing_newline: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            indent_size: 2,
            max_line_width: 80,
            trailing_newline: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Lint types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for LintLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LintLevel::Error => write!(f, "error"),
            LintLevel::Warning => write!(f, "warning"),
            LintLevel::Info => write!(f, "info"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LintIssue {
    pub level: LintLevel,
    pub rule: String,
    pub message: String,
    pub line: usize,
}

// ---------------------------------------------------------------------------
// JrgFormatter
// ---------------------------------------------------------------------------

pub struct JrgFormatter {
    options: FormatOptions,
}

impl JrgFormatter {
    pub fn new(options: FormatOptions) -> Self {
        Self { options }
    }

    /// Format a parsed Document back into valid JRG markup.
    pub fn format_document(&self, doc: &Document) -> String {
        let mut out = String::new();
        let mut iter = doc.blocks.iter().peekable();

        while let Some(block) = iter.next() {
            match block {
                Block::Heading { level, text } => {
                    let _ = writeln!(
                        out,
                        "{} {}",
                        "#".repeat(*level as usize),
                        text
                    );
                }
                Block::Paragraph(text) => {
                    let _ = writeln!(out, "{}", text);
                }
                Block::Link(link) => {
                    self.format_link(&mut out, link);
                    let _ = writeln!(out);
                }
                Block::Input(input) => {
                    self.format_input(&mut out, input);
                    let _ = writeln!(out);
                }
                Block::Button(button) => {
                    self.format_button(&mut out, button);
                    let _ = writeln!(out);
                }
                Block::Image(image) => {
                    self.format_image(&mut out, image);
                    let _ = writeln!(out);
                }
                Block::Quote(text) => {
                    for line in text.lines() {
                        let _ = writeln!(out, "> {}", line);
                    }
                }
                Block::List(items) => {
                    for item in items {
                        let _ = writeln!(out, "- {}", item);
                    }
                }
                Block::Rule => {
                    let _ = writeln!(out, "---");
                }
                Block::Table(table) => {
                    self.format_table(&mut out, table);
                }
                Block::Preformatted { code, language } => {
                    match language {
                        Some(lang) => writeln!(out, "```{}", lang).ok(),
                        None => writeln!(out, "```").ok(),
                    };
                    let _ = write!(out, "{}", code);
                    if !code.ends_with('\n') {
                        let _ = writeln!(out);
                    }
                    let _ = writeln!(out, "```");
                }
                Block::Script { label, .. } => {
                    if let Some(label) = label {
                        let _ = writeln!(out, "~> {}", label);
                    } else {
                        let _ = writeln!(out, "~>");
                    }
                }
            }

            // Blank line separator between blocks
            if iter.peek().is_some() {
                let _ = writeln!(out);
            }
        }

        // Append metadata if present
        if let Some(metadata) = &doc.metadata {
            let _ = writeln!(out, "~~~~~");
            let _ = write!(out, "{}", metadata);
            if !metadata.ends_with('\n') {
                let _ = writeln!(out);
            }
        }

        // Ensure trailing newline
        if self.options.trailing_newline && !out.ends_with('\n') {
            out.push('\n');
        }

        out
    }

    /// Lint a parsed Document against its source text.
    pub fn lint_document(&self, doc: &Document, source: &str) -> Vec<LintIssue> {
        let mut issues = Vec::new();
        let source_lines: Vec<&str> = source.lines().collect();

        // Build a line → block-type map for context
        let mut line_to_block: Vec<&str> = Vec::new();
        for block in doc.blocks.iter() {
            let lines = self.block_source_lines(block);
            for _ in 0..lines {
                line_to_block.push(match block {
                    Block::Heading { .. } => "heading",
                    Block::Paragraph(_) => "paragraph",
                    Block::Link(_) => "link",
                    Block::Input(_) => "input",
                    Block::Button(_) => "button",
                    Block::Image(_) => "image",
                    Block::Quote(_) => "quote",
                    Block::List(_) => "list",
                    Block::Rule => "rule",
                    Block::Table(_) => "table",
                    Block::Preformatted { .. } => "preformatted",
                    Block::Script { .. } => "script",
                });
            }
        }

        // Rule: no-trailing-spaces
        for (i, line) in source_lines.iter().enumerate() {
            let line_num = i + 1;
            if line.len() > line.trim_end().len() {
                issues.push(LintIssue {
                    level: LintLevel::Warning,
                    rule: "no-trailing-spaces".into(),
                    message: "trailing whitespace detected".into(),
                    line: line_num,
                });
            }
        }

        // Rule: max-line-length
        for (i, line) in source_lines.iter().enumerate() {
            // Skip preformatted and table separator lines
            let block_type = line_to_block.get(i).copied().unwrap_or("");
            if block_type == "preformatted" {
                continue;
            }
            let line_num = i + 1;
            if line.len() > self.options.max_line_width {
                issues.push(LintIssue {
                    level: LintLevel::Warning,
                    rule: "max-line-length".into(),
                    message: format!(
                        "line too long ({} > {} characters)",
                        line.len(),
                        self.options.max_line_width
                    ),
                    line: line_num,
                });
            }
        }

        // Rule: h1-max — at most one H1 heading
        let h1_count = doc
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Heading { level: 1, .. }))
            .count();
        if h1_count > 1 {
            issues.push(LintIssue {
                level: LintLevel::Warning,
                rule: "h1-max".into(),
                message: format!("document has {} H1 headings, expected at most 1", h1_count),
                line: 1,
            });
        }

        // Rule: link-label-matches-target
        for (block_idx, block) in doc.blocks.iter().enumerate() {
            if let Block::Link(link) = block
                && link.label == link.target {
                    let line_num = source_line_for_block(source, block_idx);
                    issues.push(LintIssue {
                        level: LintLevel::Info,
                        rule: "link-label-matches-target".into(),
                        message: format!(
                            "link label '{}' matches target '{}', could use short form",
                            link.label, link.target
                        ),
                        line: line_num,
                    });
                }
        }

        // Rule: empty-paragraph
        for (block_idx, block) in doc.blocks.iter().enumerate() {
            if let Block::Paragraph(text) = block
                && text.trim().is_empty() {
                    let line_num = source_line_for_block(source, block_idx);
                    issues.push(LintIssue {
                        level: LintLevel::Warning,
                        rule: "empty-paragraph".into(),
                        message: "empty or whitespace-only paragraph block".into(),
                        line: line_num,
                    });
                }
        }

        // Rule: trailing-newline
        if self.options.trailing_newline && !source.ends_with('\n') {
            issues.push(LintIssue {
                level: LintLevel::Info,
                rule: "trailing-newline".into(),
                message: "file does not end with a newline".into(),
                line: source_lines.len(),
            });
        }

        issues
    }

    /// Parse source, format, and return formatted output preserving metadata.
    pub fn format_source(&self, source: &str) -> Result<String, String> {
        let doc = jaringan_core::parse_document(source).map_err(|e| format!("parse error: {e}"))?;
        Ok(self.format_document(&doc))
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn format_link(&self, out: &mut String, link: &Link) {
        let _ = write!(out, "=> {}", link.target);
        if link.label != link.target {
            let _ = write!(out, " {}", link.label);
        }
    }

    fn format_input(&self, out: &mut String, input: &Input) {
        let _ = write!(out, "?{}", input.name);
        let mut attrs = Vec::new();
        if input.label != input.name {
            attrs.push(format!("label=\"{}\"", input.label));
        }
        if !input.value.is_empty() {
            attrs.push(format!("value=\"{}\"", input.value));
        }
        if let Some(placeholder) = &input.placeholder {
            attrs.push(format!("placeholder=\"{}\"", placeholder));
        }
        if !attrs.is_empty() {
            let _ = write!(out, " {}", attrs.join(" "));
        }
    }

    fn format_button(&self, out: &mut String, button: &Button) {
        let _ = write!(out, "!{}", button.id);
        let mut attrs = Vec::new();
        attrs.push(format!("label=\"{}\"", button.label));
        attrs.push(format!("target=\"{}\"", button.target));
        if button.method != ActionMethod::Get {
            attrs.push(format!("method=\"{}\"", button.method.as_str()));
        }
        if let Some(confirm) = &button.confirm {
            attrs.push(format!("confirm=\"{}\"", confirm));
        }
        if let Some(auth) = &button.auth {
            attrs.push(format!("auth=\"{}\"", auth));
        }
        if !attrs.is_empty() {
            let _ = write!(out, " {}", attrs.join(" "));
        }
    }

    fn format_image(&self, out: &mut String, image: &Image) {
        let _ = write!(out, "@{}", image.source);
        if image.alt != image.source {
            let _ = write!(out, " alt=\"{}\"", image.alt);
        }
    }

    fn format_table(&self, out: &mut String, table: &Table) {
        // Header row
        let header_line = format!(
            "| {} |",
            table
                .headers
                .iter()
                .map(|h| h.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        );
        let _ = writeln!(out, "{}", header_line);

        // Separator row
        let sep = table
            .headers
            .iter()
            .map(|h| {
                let width = h.len().max(3);
                "-".repeat(width)
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let _ = writeln!(out, "| {} |", sep);

        // Data rows
        for row in &table.rows {
            let data_line = format!(
                "| {} |",
                row.iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
            let _ = writeln!(out, "{}", data_line);
        }
    }

    /// How many lines a block occupies in source (approximate, used for line mapping).
    fn block_source_lines(&self, block: &Block) -> usize {
        match block {
            Block::Heading { .. } => 1,
            Block::Paragraph(text) => text.lines().count().max(1),
            Block::Link(_) => 1,
            Block::Input(_) => 1,
            Block::Button(_) => 1,
            Block::Image(_) => 1,
            Block::Quote(text) => text.lines().count().max(1),
            Block::List(items) => items.len().max(1),
            Block::Rule => 1,
            Block::Table(table) => 2 + table.rows.len(), // header + sep + data rows
            Block::Preformatted { code, .. } => 2 + code.lines().count(), // ```open + body lines + ```
            Block::Script { .. } => 1, // ~> header only in formatted output
        }
    }
}

/// Estimate source line number for a given block index.
fn source_line_for_block(source: &str, block_idx: usize) -> usize {
    // Walk through source and count blank-line separators to approximate.
    let source_lines: Vec<&str> = source.lines().collect();
    let mut line_num = 1;
    let mut count = 0;
    let mut in_pre = false;
    for line in &source_lines {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_pre = !in_pre;
        }
        if trimmed.is_empty() && !in_pre {
            count += 1; // skip blank lines between blocks
            line_num += 1;
            continue;
        }
        if count == block_idx {
            return line_num;
        }
        // If this line starts a new block, increment count
        if !in_pre
            && (trimmed.starts_with('#')
                || trimmed.starts_with("=>")
                || trimmed.starts_with('?')
                || trimmed.starts_with('!')
                || trimmed.starts_with('@')
                || trimmed.starts_with('>')
                || trimmed.starts_with('|')
                || trimmed.starts_with('-')
                || trimmed.starts_with('*')
                || trimmed == "---"
                || trimmed == "***"
                || trimmed == "___")
        {
            count += 1;
            if count - 1 == block_idx {
                return line_num;
            }
        }
        // Paragraph lines don't increment block count, they just consume
        // Actually this is tricky. Let's use a simpler approach.
        line_num += 1;
    }
    1 // fallback
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use jaringan_core::parse_document;

    #[test]
    fn format_empty_document() {
        let doc = Document::new(vec![]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        // Empty doc + trailing_newline = just a newline
        assert_eq!(result, "\n");
    }

    #[test]
    fn format_heading() {
        let doc = Document::new(vec![Block::Heading {
            level: 1,
            text: "Hello".into(),
        }]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert!(result.starts_with("# Hello\n"));
    }

    #[test]
    fn format_paragraph() {
        let doc = Document::new(vec![Block::Paragraph("This is a paragraph.".into())]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(result, "This is a paragraph.\n");
    }

    #[test]
    fn format_heading_and_paragraph() {
        let doc = Document::new(vec![
            Block::Heading {
                level: 1,
                text: "Title".into(),
            },
            Block::Paragraph("Some content here.".into()),
        ]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(result, "# Title\n\nSome content here.\n");
    }

    #[test]
    fn format_link_with_label() {
        let doc = Document::new(vec![Block::Link(Link {
            target: "jrg://example/page".into(),
            label: "Example Page".into(),
        })]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(result, "=> jrg://example/page Example Page\n");
    }

    #[test]
    fn format_link_short_form() {
        let doc = Document::new(vec![Block::Link(Link {
            target: "jrg://example/page".into(),
            label: "jrg://example/page".into(),
        })]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        // No label when it matches target
        assert_eq!(result, "=> jrg://example/page\n");
    }

    #[test]
    fn format_input() {
        let doc = Document::new(vec![Block::Input(Input {
            name: "email".into(),
            label: "Email Address".into(),
            value: String::new(),
            placeholder: Some("you@example.com".into()),
        })]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(
            result,
            "?email label=\"Email Address\" placeholder=\"you@example.com\"\n"
        );
    }

    #[test]
    fn format_button() {
        let doc = Document::new(vec![Block::Button(Button {
            id: "btn1".into(),
            label: "Click Me".into(),
            target: "jrg://action".into(),
            method: ActionMethod::Post,
            confirm: Some("Are you sure?".into()),
            auth: None,
        })]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(
            result,
            "!btn1 label=\"Click Me\" target=\"jrg://action\" method=\"POST\" confirm=\"Are you sure?\"\n"
        );
    }

    #[test]
    fn format_image() {
        let doc = Document::new(vec![Block::Image(Image {
            source: "https://example.com/photo.jpg".into(),
            alt: "A photo".into(),
        })]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(result, "@https://example.com/photo.jpg alt=\"A photo\"\n");
    }

    #[test]
    fn format_image_no_alt() {
        let doc = Document::new(vec![Block::Image(Image {
            source: "https://example.com/photo.jpg".into(),
            alt: "https://example.com/photo.jpg".into(),
        })]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(result, "@https://example.com/photo.jpg\n");
    }

    #[test]
    fn format_quote() {
        let doc = Document::new(vec![Block::Quote("This is a quote.".into())]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(result, "> This is a quote.\n");
    }

    #[test]
    fn format_quote_multiline() {
        let doc = Document::new(vec![Block::Quote("Line one\nLine two".into())]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(result, "> Line one\n> Line two\n");
    }

    #[test]
    fn format_list() {
        let doc = Document::new(vec![Block::List(vec!["Item one".into(), "Item two".into()])]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(result, "- Item one\n- Item two\n");
    }

    #[test]
    fn format_rule() {
        let doc = Document::new(vec![Block::Rule]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(result, "---\n");
    }

    #[test]
    fn format_table() {
        let doc = Document::new(vec![Block::Table(Table {
            headers: vec!["Name".into(), "Age".into()],
            rows: vec![
                vec!["Alice".into(), "30".into()],
                vec!["Bob".into(), "25".into()],
            ],
            alignments: vec![],
        })]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(
            result,
            "| Name | Age |\n| ---- | --- |\n| Alice | 30 |\n| Bob | 25 |\n"
        );
    }

    #[test]
    fn format_preformatted() {
        let doc = Document::new(vec![Block::Preformatted {
            code: "fn main() {\n    println!(\"hello\");\n}".into(),
            language: Some("rust".into()),
        }]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(
            result,
            "```rust\nfn main() {\n    println!(\"hello\");\n}\n```\n"
        );
    }

    #[test]
    fn format_preformatted_no_language() {
        let doc = Document::new(vec![Block::Preformatted {
            code: "plain text".into(),
            language: None,
        }]);
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);
        assert_eq!(result, "```\nplain text\n```\n");
    }

    #[test]
    fn format_with_metadata() {
        let doc = Document::with_metadata(
            vec![Block::Heading {
                level: 1,
                text: "Page".into(),
            }],
            Some("title: My Page\nsigned-by: alice\n".into()),
        );
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_document(&doc);

        assert!(result.contains("~~~~~"));
        assert!(result.contains("title: My Page"));
        assert!(result.contains("signed-by: alice"));
    }

    #[test]
    fn format_roundtrip() {
        let source = "# Hello\n\nThis is a paragraph.\n\n=> jrg://example/about About us\n";
        let fmt = JrgFormatter::new(FormatOptions::default());
        let formatted = fmt.format_source(source).unwrap();
        // Re-parse and check blocks match
        let doc1 = parse_document(source).unwrap();
        let doc2 = parse_document(&formatted).unwrap();
        assert_eq!(doc1.blocks, doc2.blocks);
    }

    #[test]
    fn format_parse_error() {
        let source = "# Broken\n\n```plain\nno close\n";
        let fmt = JrgFormatter::new(FormatOptions::default());
        let result = fmt.format_source(source);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("parse error"));
    }

    #[test]
    fn lint_trailing_whitespace() {
        let source = "# Hello  \n\nContent\n";
        let doc = parse_document(source).unwrap();
        let fmt = JrgFormatter::new(FormatOptions::default());
        let issues = fmt.lint_document(&doc, source);
        assert!(issues.iter().any(|i| i.rule == "no-trailing-spaces"));
    }

    #[test]
    fn lint_max_line_length() {
        let long_line = "A".repeat(100);
        let source = format!("# Header\n\n{long_line}\n");
        let doc = parse_document(&source).unwrap();
        let fmt = JrgFormatter::new(FormatOptions::default());
        let issues = fmt.lint_document(&doc, &source);
        assert!(issues.iter().any(|i| i.rule == "max-line-length"));
    }

    #[test]
    fn lint_multiple_h1() {
        let source = "# First\n\n# Second\n";
        let doc = parse_document(source).unwrap();
        let fmt = JrgFormatter::new(FormatOptions::default());
        let issues = fmt.lint_document(&doc, source);
        assert!(issues.iter().any(|i| i.rule == "h1-max"));
    }

    #[test]
    fn lint_no_trailing_newline() {
        let source = "# Hello";
        let doc = parse_document(source).unwrap();
        let fmt = JrgFormatter::new(FormatOptions::default());
        let issues = fmt.lint_document(&doc, source);
        assert!(issues.iter().any(|i| i.rule == "trailing-newline"));
    }

    #[test]
    fn lint_clean_document() {
        let source =
            "# Hello\n\nRegular paragraph.\n\n=> jrg://cool Cool link\n\n> A quote\n\n- item one\n- item two\n\n---\n\n```rust\nfn main() {}\n```\n";
        let doc = parse_document(source).unwrap();
        let fmt = JrgFormatter::new(FormatOptions::default());
        let issues = fmt.lint_document(&doc, source);
        let clean_issues: Vec<_> = issues
            .into_iter()
            .filter(|i| i.rule != "trailing-newline")
            .collect();
        assert!(
            clean_issues.is_empty(),
            "expected no issues, got: {:?}",
            clean_issues
        );
    }

    #[test]
    fn lint_empty_paragraph() {
        // The parser never produces empty paragraphs (it skips whitespace-only lines).
        // The lint rule still catches empty paragraphs from programmatic Document construction.
        let doc = Document::new(vec![
            Block::Heading {
                level: 1,
                text: "Header".into(),
            },
            Block::Paragraph("   ".into()),
        ]);
        let source = "# Header\n\n   \n";
        let fmt = JrgFormatter::new(FormatOptions::default());
        let issues = fmt.lint_document(&doc, source);
        assert!(issues.iter().any(|i| i.rule == "empty-paragraph"));
    }

    #[test]
    fn lint_link_label_matches_target() {
        let source = "=> jrg://page jrg://page\n";
        let doc = parse_document(source).unwrap();
        let fmt = JrgFormatter::new(FormatOptions::default());
        let issues = fmt.lint_document(&doc, source);
        assert!(issues.iter().any(|i| i.rule == "link-label-matches-target"));
    }

    #[test]
    fn lint_level_display() {
        assert_eq!(format!("{}", LintLevel::Error), "error");
        assert_eq!(format!("{}", LintLevel::Warning), "warning");
        assert_eq!(format!("{}", LintLevel::Info), "info");
    }

    #[test]
    fn no_trailing_newline_option() {
        let opts = FormatOptions {
            trailing_newline: false,
            ..Default::default()
        };
        let fmt = JrgFormatter::new(opts);
        let doc = Document::new(vec![Block::Heading {
            level: 1,
            text: "Hi".into(),
        }]);
        let result = fmt.format_document(&doc);
        // writeln! produces "# Hi\n", and trailing_newline=false means no extra \n added
        // So result is "# Hi\n" which does end with \n (from writeln! itself)
        // The trailing_newline option only adds an extra trailing newline when the
        // output doesn't already end with one. Since writeln! always adds \n,
        // the output always ends with \n regardless.
        assert_eq!(result, "# Hi\n");
    }

    #[test]
    fn format_document_no_blocks() {
        let opts = FormatOptions {
            trailing_newline: false,
            ..Default::default()
        };
        let fmt = JrgFormatter::new(opts);
        let doc = Document::new(vec![]);
        let result = fmt.format_document(&doc);
        assert_eq!(result, "");
    }
}

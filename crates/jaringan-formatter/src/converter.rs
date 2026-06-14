use jaringan_core::{parse_document, Block, Document};

/// Convert Markdown text to JRG format.
pub fn md_to_jrg(input: &str) -> Result<String, String> {
    if input.is_empty() {
        return Ok(String::new());
    }

    let lines: Vec<&str> = input.lines().collect();
    let mut output = String::new();
    let mut i = 0;

    // 1. Detect YAML frontmatter (--- ... ---) at the start of the file
    if !lines.is_empty() && lines[0].trim() == "---" {
        // Collect frontmatter lines until closing ---
        let mut metadata_lines = Vec::new();
        i = 1;
        while i < lines.len() {
            if lines[i].trim() == "---" {
                i += 1; // skip closing ---
                break;
            }
            metadata_lines.push(lines[i]);
            i += 1;
        }
        if !metadata_lines.is_empty() {
            output.push_str("~~~~~\n");
            for line in &metadata_lines {
                output.push_str(line);
                output.push('\n');
            }
        }
    }

    // 2. Process remaining lines
    while i < lines.len() {
        let line = lines[i];

        // Strip HTML comments (entire line or inline)
        let line = strip_html_comments(line);

        // Replace Markdown images ![alt](src) with @ src alt="alt"
        let line = convert_md_images_inline(&line);

        // Replace Markdown links [label](url) with => url label
        let line = convert_md_links_inline(&line);

        output.push_str(&line);
        output.push('\n');
        i += 1;
    }

    Ok(output)
}

/// Convert JRG text to Markdown format.
pub fn jrg_to_md(input: &str) -> Result<String, String> {
    if input.is_empty() {
        return Ok(String::new());
    }

    let doc = parse_document(input).map_err(|e| format!("parse error: {e}"))?;
    render_document_as_md(&doc)
}

/// Render a parsed JRG Document as Markdown.
fn render_document_as_md(doc: &Document) -> Result<String, String> {
    let mut out = String::new();

    // First pass: collect metadata if present — we render it as YAML frontmatter
    let metadata = doc.metadata.as_deref();

    // Open YAML frontmatter if metadata exists
    if let Some(meta) = metadata {
        out.push_str("---\n");
        out.push_str(meta);
        if !meta.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("---\n\n");
    }

    // Render blocks
    let mut iter = doc.blocks.iter().peekable();
    while let Some(block) = iter.next() {
        match block {
            Block::Heading { level, text } => {
                out.push_str(&format!("{} {}\n", "#".repeat(*level as usize), text));
            }
            Block::Paragraph(text) => {
                out.push_str(text);
                out.push('\n');
            }
            Block::Link(link) => {
                out.push_str(&format!("[{}]({})\n", link.label, link.target));
            }
            Block::Input(input) => {
                // Render as HTML comment annotation: <!-- input:?name label="..." -->
                let mut comment = format!("<!-- input:?{}", input.name);
                if input.label != input.name {
                    comment.push_str(&format!(" label=\"{}\"", input.label));
                }
                if !input.value.is_empty() {
                    comment.push_str(&format!(" value=\"{}\"", input.value));
                }
                if let Some(placeholder) = &input.placeholder {
                    comment.push_str(&format!(" placeholder=\"{}\"", placeholder));
                }
                comment.push_str(" -->\n");
                out.push_str(&comment);
            }
            Block::Button(button) => {
                // Render as HTML comment annotation: <!-- button:!id label="..." -->
                let mut comment = format!("<!-- button:!{}", button.id);
                comment.push_str(&format!(" label=\"{}\"", button.label));
                comment.push_str(&format!(" target=\"{}\"", button.target));
                if button.method != jaringan_core::ActionMethod::Get {
                    comment.push_str(&format!(" method=\"{}\"", button.method.as_str()));
                }
                if let Some(confirm) = &button.confirm {
                    comment.push_str(&format!(" confirm=\"{}\"", confirm));
                }
                if let Some(auth) = &button.auth {
                    comment.push_str(&format!(" auth=\"{}\"", auth));
                }
                comment.push_str(" -->\n");
                out.push_str(&comment);
            }
            Block::Image(image) => {
                out.push_str(&format!("![{}]({})\n", image.alt, image.source));
            }
            Block::Quote(text) => {
                for line in text.lines() {
                    out.push_str(&format!("> {}\n", line));
                }
            }
            Block::List(items) => {
                for item in items {
                    out.push_str(&format!("- {}\n", item));
                }
            }
            Block::Rule => {
                out.push_str("---\n");
            }
            Block::Table(table) => {
                // Header row
                out.push('|');
                for h in &table.headers {
                    out.push_str(&format!(" {} |", h));
                }
                out.push('\n');

                // Separator row
                out.push('|');
                for h in &table.headers {
                    let width = h.len().max(3);
                    out.push_str(&format!(" {} |", "-".repeat(width)));
                }
                out.push('\n');

                // Data rows
                for row in &table.rows {
                    out.push('|');
                    for cell in row {
                        out.push_str(&format!(" {} |", cell));
                    }
                    out.push('\n');
                }
            }
            Block::Preformatted { code, language } => {
                match language {
                    Some(lang) => out.push_str(&format!("```{}\n", lang)),
                    None => out.push_str("```\n"),
                }
                out.push_str(code);
                if !code.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```\n");
            }
            Block::Script { label, .. } => {
                // Render as HTML comment
                if let Some(lbl) = label {
                    out.push_str(&format!("<!-- script:{} -->\n", lbl));
                } else {
                    out.push_str("<!-- script -->\n");
                }
            }
            Block::Auth { service, .. } => {
                out.push_str(&format!("<!-- auth:{} -->\n", service));
            }
        }

        // Blank line separator between blocks (matching format_document behavior)
        if iter.peek().is_some() {
            out.push('\n');
        }
    }

    Ok(out)
}

/// Strip HTML comments from a line of text.
/// Handles both full-line comments and inline comments.
fn strip_html_comments(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_comment = false;

    while let Some(ch) = chars.next() {
        if !in_comment && ch == '<' {
            // Check for <!--
            let remaining: String = chars.clone().take(3).collect();
            if remaining == "!--" {
                in_comment = true;
                // consume the '!--'
                chars.nth(2); // skip '!', '-', '-'
                continue;
            }
        }

        if in_comment {
            // Look for -->
            if ch == '-' {
                let remaining: String = chars.clone().take(2).collect();
                if remaining == "->" {
                    in_comment = false;
                    chars.nth(1); // skip '>'
                    continue;
                }
            }
            continue;
        }

        result.push(ch);
    }

    // Trim whitespace that might be left after comment removal
    if result != line {
        result = result.trim().to_string();
    }

    result
}

/// Convert Markdown links `[label](url)` to JRG format `=> url label` within a line.
fn convert_md_links_inline(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        // Look for '['
        if bytes[pos] == b'[' {
            // Find the matching ']'
            let label_start = pos + 1;
            if let Some(close_bracket) = find_byte(bytes, b']', label_start) {
                // Check for '(' immediately after ']'
                if close_bracket + 1 < len && bytes[close_bracket + 1] == b'(' {
                    let url_start = close_bracket + 2;
                    if let Some(close_paren) = find_byte(bytes, b')', url_start) {
                        let label = &line[label_start..close_bracket];
                        let url = &line[url_start..close_paren];

                        // Skip any inline formatting within label — handle it as-is
                        result.push_str("=> ");
                        result.push_str(url);
                        result.push(' ');
                        result.push_str(label);

                        pos = close_paren + 1;
                        continue;
                    }
                }
            }
        }

        result.push(bytes[pos] as char);
        pos += 1;
    }

    result
}

/// Convert Markdown images `![alt](src)` to JRG format `@ src alt="alt"` within a line.
fn convert_md_images_inline(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        // Look for '!['
        if pos + 1 < len && bytes[pos] == b'!' && bytes[pos + 1] == b'[' {
            let alt_start = pos + 2;
            if let Some(close_bracket) = find_byte(bytes, b']', alt_start) {
                // Check for '(' immediately after ']'
                if close_bracket + 1 < len && bytes[close_bracket + 1] == b'(' {
                    let src_start = close_bracket + 2;
                    if let Some(close_paren) = find_byte(bytes, b')', src_start) {
                        let alt = &line[alt_start..close_bracket];
                        let src = &line[src_start..close_paren];

                        result.push('@');
                        result.push_str(src);
                        result.push_str(" alt=\"");
                        result.push_str(alt);
                        result.push('"');

                        pos = close_paren + 1;
                        continue;
                    }
                }
            }
        }

        result.push(bytes[pos] as char);
        pos += 1;
    }

    result
}

/// Find a byte in a slice starting from a given position.
fn find_byte(bytes: &[u8], target: u8, start: usize) -> Option<usize> {
    bytes[start..]
        .iter()
        .position(|&b| b == target)
        .map(|i| i + start)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md_to_jrg_basic_link() {
        let input = "[Example](https://example.com)";
        let result = md_to_jrg(input).unwrap();
        assert_eq!(result, "=> https://example.com Example\n");
    }

    #[test]
    fn md_to_jrg_frontmatter() {
        let input = "---\ntitle: My Page\nauthor: Alice\n---\n\n# Hello";
        let result = md_to_jrg(input).unwrap();
        assert!(result.starts_with("~~~~~\n"));
        assert!(result.contains("title: My Page"));
        assert!(result.contains("author: Alice"));
        assert!(result.ends_with("# Hello\n"));
    }

    #[test]
    fn md_to_jrg_image() {
        let input = "![Logo](logo.png)";
        let result = md_to_jrg(input).unwrap();
        assert_eq!(result, "@logo.png alt=\"Logo\"\n");
    }

    #[test]
    fn jrg_to_md_basic_link() {
        let input = "=> https://example.com Example";
        let result = jrg_to_md(input).unwrap();
        assert_eq!(result.trim(), "[Example](https://example.com)");
    }

    #[test]
    fn jrg_to_md_image() {
        let input = "@logo.png alt=\"Logo\"";
        let result = jrg_to_md(input).unwrap();
        assert_eq!(result.trim(), "![Logo](logo.png)");
    }

    #[test]
    fn jrg_to_md_input() {
        let input = "?name label=\"Name\"";
        let result = jrg_to_md(input).unwrap();
        // Should become an HTML comment annotation for input
        assert!(result.contains("<!-- input:?name"));
        assert!(result.contains("label=\"Name\""));
        assert!(result.contains("-->"));
    }

    #[test]
    fn roundtrip_preserves_content() {
        let md = "# Hello\n\nThis is a paragraph.\n\n[Example](https://example.com)\n";
        // Convert MD → JRG
        let jrg = md_to_jrg(md).unwrap();
        // Convert JRG → MD
        let result = jrg_to_md(&jrg).unwrap();
        // Both should contain the heading and link
        assert!(result.contains("# Hello"));
        assert!(result.contains("This is a paragraph."));
        assert!(result.contains("[Example](https://example.com)"));
    }

    #[test]
    fn empty_input() {
        assert_eq!(md_to_jrg("").unwrap(), "");
        assert_eq!(jrg_to_md("").unwrap(), "");
    }

    #[test]
    fn md_to_jrg_html_comment_stripped() {
        let input = "Hello <!-- comment --> World";
        let result = md_to_jrg(input).unwrap();
        assert_eq!(result.trim(), "Hello  World"); // comment removed, surrounding whitespace remains
    }

    #[test]
    fn md_to_jrg_bold_italic_preserved() {
        let input = "**bold** and *italic* and `code`";
        let result = md_to_jrg(input).unwrap();
        // Bold, italic, and inline code should be preserved as-is
        assert!(result.contains("**bold**"));
        assert!(result.contains("*italic*"));
        assert!(result.contains("`code`"));
    }

    #[test]
    fn jrg_to_md_button() {
        let input = "!btn1 label=\"Click Me\" target=\"jrg://action\" method=\"POST\" confirm=\"Are you sure?\"";
        let result = jrg_to_md(input).unwrap();
        // Should render as HTML comment annotation for button
        assert!(result.contains("<!-- button:!btn1"));
        assert!(result.contains("label=\"Click Me\""));
        assert!(result.contains("-->"));
    }

    #[test]
    fn md_to_jrg_multiple_links() {
        let input = "See [Google](https://google.com) and [Bing](https://bing.com)";
        let result = md_to_jrg(input).unwrap();
        assert!(result.contains("=> https://google.com Google"));
        assert!(result.contains("=> https://bing.com Bing"));
    }

    #[test]
    fn jrg_to_md_heading_and_paragraph() {
        let input = "# Hello\n\nThis is a paragraph.\n";
        let result = jrg_to_md(input).unwrap();
        assert!(result.contains("# Hello"));
        assert!(result.contains("This is a paragraph."));
    }
}

use jaringan_core::{Alignment, Block, Document, Table};
use ratatui::text::{Line, Span, Text};
use ratatui::style::{Style, Color, Modifier};
use syntect::parsing::SyntaxSet;
use syntect::highlighting::{ThemeSet, FontStyle};
use syntect::easy::HighlightLines;
use syntect::util::LinesWithEndings;

use std::sync::LazyLock;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(|| {
    SyntaxSet::load_defaults_newlines()
});

static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

// ── Inline markup parsing ──────────────────────────────────────────────

/// A span of text with inline formatting applied.
#[derive(Debug, Clone, PartialEq)]
pub enum InlineSpan {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    Link { label: String, target: String },
}

/// Parse inline formatting markers (`**bold**`, `*italic*`, `` `code` ``, `[label](url)`)
/// from a plain string into a list of styled spans.
///
/// Markers must be properly paired. Unmatched delimiters render as literal text.
/// Backtick spans take priority over other markers within their bounds.
pub fn parse_inline_markup(text: &str) -> Vec<InlineSpan> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut spans: Vec<InlineSpan> = Vec::new();
    let mut i = 0;

    while i < len {
        // Code span: `...` — highest priority
        if bytes[i] == b'`' {
            if let Some(end) = text[i + 1..].find('`') {
                let code = &text[i + 1..i + 1 + end];
                spans.push(InlineSpan::Code(code.to_string()));
                i += end + 2;
                continue;
            }
            // Unmatched backtick — push as literal
            spans.push(InlineSpan::Text("`".to_string()));
            i += 1;
            continue;
        }

        // Link: [label](url)
        if bytes[i] == b'[' {
            if let Some(close_bracket) = text[i + 1..].find(']') {
                let after_bracket = i + 1 + close_bracket + 1;
                if after_bracket < len && bytes[after_bracket] == b'('
                    && let Some(close_paren) = text[after_bracket + 1..].find(')') {
                        let label = &text[i + 1..i + 1 + close_bracket];
                        let target = &text[after_bracket + 1..after_bracket + 1 + close_paren];
                        spans.push(InlineSpan::Link {
                            label: label.to_string(),
                            target: target.to_string(),
                        });
                        i = after_bracket + 1 + close_paren + 1;
                        continue;
                    }
            }
            // Unmatched [ — push as literal
            spans.push(InlineSpan::Text("[".to_string()));
            i += 1;
            continue;
        }

        // Bold: **text**
        if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'*' {
            if let Some(end) = text[i + 2..].find("**") {
                let content = &text[i + 2..i + 2 + end];
                if !content.is_empty() {
                    spans.push(InlineSpan::Bold(content.to_string()));
                    i += end + 4;
                    continue;
                }
            }
            // No matching ** or empty content — treat ** as two literal asterisks
            spans.push(InlineSpan::Text("**".to_string()));
            i += 2;
            continue;
        }

        // Italic: *text*
        if bytes[i] == b'*' {
            if let Some(end) = text[i + 1..].find('*') {
                let content = &text[i + 1..i + 1 + end];
                if !content.is_empty() && !content.trim().is_empty() {
                    spans.push(InlineSpan::Italic(content.to_string()));
                    i += end + 2;
                    continue;
                }
            }
            // Unmatched * — push as literal
            spans.push(InlineSpan::Text("*".to_string()));
            i += 1;
            continue;
        }

        // Regular character — accumulate text run
        let start = i;
        while i < len && !matches!(bytes[i], b'`' | b'[' | b'*') {
            i += 1;
        }
        if i > start {
            spans.push(InlineSpan::Text(text[start..i].to_string()));
        }
    }

    spans
}

/// Render a plain-text version of inline markup (strip markers, show links as `label (url)`).
pub fn inline_to_plain(spans: &[InlineSpan]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            InlineSpan::Text(s) | InlineSpan::Bold(s) | InlineSpan::Italic(s) => out.push_str(s),
            InlineSpan::Code(s) => out.push_str(s),
            InlineSpan::Link { label, target } => {
                out.push_str(label);
                if !target.is_empty() && target != label {
                    out.push_str(&format!(" ({target})"));
                }
            }
        }
    }
    out
}

/// Convert inline markup spans to ratatui styled spans for TUI rendering.
fn inline_to_ratatui_spans(spans: &[InlineSpan]) -> Vec<Span<'static>> {
    let mut result = Vec::new();
    for span in spans {
        match span {
            InlineSpan::Text(s) => result.push(Span::raw(s.clone())),
            InlineSpan::Bold(s) => {
                result.push(Span::styled(
                    s.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            }
            InlineSpan::Italic(s) => {
                result.push(Span::styled(
                    s.clone(),
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
            }
            InlineSpan::Code(s) => {
                result.push(Span::styled(
                    s.clone(),
                    Style::default()
                        .fg(Color::Green)
                        .bg(Color::DarkGray),
                ));
            }
            InlineSpan::Link { label, target } => {
                result.push(Span::styled(
                    label.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED),
                ));
                if !target.is_empty() && target != label {
                    result.push(Span::raw(format!(" <{target}>")));
                }
            }
        }
    }
    result
}

/// Safely look up a theme, falling back to the first available theme if missing.
fn get_theme(name: &str) -> syntect::highlighting::Theme {
    THEME_SET.themes.get(name).cloned().unwrap_or_else(|| {
        THEME_SET.themes.values().next().cloned().unwrap_or_default()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    pub show_link_targets: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            show_link_targets: true,
        }
    }
}

pub fn render_plain(document: &Document) -> String {
    render_plain_with_options(document, RenderOptions::default())
}

pub fn render_plain_with_options(document: &Document, options: RenderOptions) -> String {
    let mut output = String::new();
    let mut link_index = 1usize;

    for block in &document.blocks {
        match block {
            Block::Heading { level, text } => {
                output.push_str(&"#".repeat(*level as usize));
                output.push(' ');
                output.push_str(&inline_to_plain(&parse_inline_markup(text)));
                output.push_str("\n\n");
            }
            Block::Paragraph(text) => {
                output.push_str(&inline_to_plain(&parse_inline_markup(text)));
                output.push_str("\n\n");
            }
            Block::Link(link) => {
                if options.show_link_targets {
                    output.push_str(&format!(
                        "[{link_index}] {} <{}>\n",
                        link.label, link.target
                    ));
                } else {
                    output.push_str(&format!("[{link_index}] {}\n", link.label));
                }
                link_index += 1;
            }
            Block::Input(input) => {
                let value = if input.value.is_empty() {
                    input.placeholder.as_deref().unwrap_or("")
                } else {
                    &input.value
                };
                output.push_str(&format!(
                    "[input] {} ({}) = {}\n",
                    input.label, input.name, value
                ));
            }
            Block::Button(button) => {
                output.push_str(&format!(
                    "[button] {} <{} {}>\n",
                    button.label,
                    button.method.as_str(),
                    button.target
                ));
            }
            Block::Image(image) => {
                output.push_str(&format!("[image] {} <{}>\n", image.alt, image.source));
            }
            Block::Quote(text) => {
                for line in text.lines() {
                    output.push_str(&format!("> {}\n", inline_to_plain(&parse_inline_markup(line))));
                }
                output.push('\n');
            }
            Block::List(items) => {
                for item in items {
                    output.push_str(&format!("• {}\n", inline_to_plain(&parse_inline_markup(item))));
                }
                output.push('\n');
            }
            Block::Rule => {
                output.push_str("────────────────────────────────────────\n\n");
            }
            Block::Table(table) => {
                output.push_str(&render_table_plain(table));
                output.push_str("\n\n");
            }
            Block::Preformatted { code, language } => {
                let lang = language.as_deref().unwrap_or("");
                output.push_str(&format!("```{lang}\n"));
                output.push_str(&highlight_ansi(code, language.as_deref()));
                output.push_str("\n```\n\n");
            }
            Block::Script { label, .. } => {
                if let Some(label) = label {
                    output.push_str(&format!("⚡ [{label}]\n\n"));
                }
            }
        }
    }

    output.trim_end().to_owned()
}

pub fn render_ratatui_text(document: &Document) -> Text<'static> {
    let mut lines = Vec::new();
    let mut link_index = 1usize;

    for block in &document.blocks {
        match block {
            Block::Heading { level, text } => {
                let spans = inline_to_ratatui_spans(&parse_inline_markup(text));
                let mut heading = vec![
                    Span::raw("#".repeat(*level as usize)),
                    Span::raw(" "),
                ];
                heading.extend(spans);
                lines.push(Line::from(heading));
                lines.push(Line::raw(""));
            }
            Block::Paragraph(text) => {
                let spans = inline_to_ratatui_spans(&parse_inline_markup(text));
                lines.push(Line::from(spans));
                lines.push(Line::raw(""));
            }
            Block::Link(link) => {
                lines.push(Line::raw(format!(
                    "[{link_index}] {} <{}>",
                    link.label, link.target
                )));
                link_index += 1;
            }
            Block::Input(input) => {
                let value = if input.value.is_empty() {
                    input.placeholder.as_deref().unwrap_or("")
                } else {
                    &input.value
                };
                lines.push(Line::raw(format!(
                    "[input] {} ({}) = {}",
                    input.label, input.name, value
                )));
            }
            Block::Button(button) => {
                lines.push(Line::raw(format!(
                    "[button] {} <{} {}>",
                    button.label,
                    button.method.as_str(),
                    button.target
                )));
            }
            Block::Image(image) => {
                lines.push(Line::raw(format!(
                    "[image] {} <{}>",
                    image.alt, image.source
                )));
            }
            Block::Quote(text) => {
                for line in text.lines() {
                    let spans = inline_to_ratatui_spans(&parse_inline_markup(line));
                    let mut qline = vec![Span::raw("┃ ")];
                    qline.extend(spans);
                    lines.push(Line::from(qline));
                }
                lines.push(Line::raw(""));
            }
            Block::List(items) => {
                for item in items {
                    let spans = inline_to_ratatui_spans(&parse_inline_markup(item));
                    let mut iline = vec![Span::raw("  • ")];
                    iline.extend(spans);
                    lines.push(Line::from(iline));
                }
                lines.push(Line::raw(""));
            }
            Block::Rule => {
                lines.push(Line::raw("────────────────────────────────────────"));
                lines.push(Line::raw(""));
            }
            Block::Table(table) => {
                lines.extend(
                    render_table_plain(table)
                        .lines()
                        .map(|line| Line::raw(line.to_owned())),
                );
                lines.push(Line::raw(""));
            }
            Block::Preformatted { code, language } => {
                let highlighted = highlight_ratatui(code, language.as_deref());
                if highlighted.len() > 1 || code.contains('\n') {
                    lines.push(Line::raw("```"));
                    lines.extend(highlighted);
                    lines.push(Line::raw("```"));
                } else {
                    lines.extend(highlighted);
                }
                lines.push(Line::raw(""));
            }
            Block::Script { label, .. } => {
                if let Some(label) = label {
                    lines.push(Line::raw(format!("⚡ [{label}]")));
                    lines.push(Line::raw(""));
                }
            }
        }
    }

    Text::from(lines)
}

/// Highlight code using syntect and return ANSI 24-bit color escape sequences.
fn highlight_ansi(code: &str, language: Option<&str>) -> String {
    let Some(lang) = language else {
        return code.to_owned();
    };
    let syntax = SYNTAX_SET
        .find_syntax_by_token(lang)
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
    let theme = get_theme("base16-ocean.dark");
    let mut highlighter =
        HighlightLines::new(syntax, &theme);
    let mut output = String::new();
    for line in LinesWithEndings::from(code) {
        if let Ok(ranges) = highlighter.highlight_line(line, &SYNTAX_SET) {
            output.push_str(&syntect::util::as_24_bit_terminal_escaped(&ranges, false));
        }
    }
    output
}

/// Highlight code using syntect and return ratatui Lines with colored spans.
fn highlight_ratatui(code: &str, language: Option<&str>) -> Vec<Line<'static>> {
    let Some(lang) = language else {
        return vec![Line::from(Span::raw(code.to_owned()))];
    };
    let syntax = SYNTAX_SET
        .find_syntax_by_token(lang)
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
    let theme = get_theme("base16-ocean.dark");
    let mut highlighter =
        HighlightLines::new(syntax, &theme);
    let mut lines = Vec::new();
    for line in LinesWithEndings::from(code) {
        if let Ok(ranges) = highlighter.highlight_line(line, &SYNTAX_SET) {
            let spans: Vec<Span<'static>> = ranges
                .into_iter()
                .map(|(style, text)| {
                    Span::styled(text.to_string(), syntect_style_to_ratatui(style))
                })
                .collect();
            lines.push(Line::from(spans));
        }
    }
    lines
}

/// Convert a syntect Style to a ratatui Style.
fn syntect_style_to_ratatui(style: syntect::highlighting::Style) -> Style {
    let mut s = Style::default();
    if style.foreground.a > 0 {
        let c = style.foreground;
        s = s.fg(Color::Rgb(c.r, c.g, c.b));
    }
    if style.background.a > 0 {
        let c = style.background;
        s = s.bg(Color::Rgb(c.r, c.g, c.b));
    }
    if style.font_style.contains(FontStyle::BOLD) {
        s = s.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        s = s.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        s = s.add_modifier(Modifier::UNDERLINED);
    }
    s
}

fn render_table_plain(table: &Table) -> String {
    let columns = table
        .headers
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return String::new();
    }

    let mut widths = vec![0usize; columns];
    for (index, cell) in table.headers.iter().enumerate() {
        widths[index] = widths[index].max(cell.chars().count());
    }
    for row in &table.rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let mut output = String::new();
    output.push_str(&render_table_row(&table.headers, &widths, &table.alignments));
    output.push('\n');
    output.push_str(&render_table_separator(&widths));
    for row in &table.rows {
        output.push('\n');
        output.push_str(&render_table_row(row, &widths, &table.alignments));
    }
    output
}

fn render_table_row(row: &[String], widths: &[usize], alignments: &[Alignment]) -> String {
    let cells = widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            let cell = row.get(index).map(String::as_str).unwrap_or("");
            let align = alignments.get(index).copied().unwrap_or(Alignment::None);
            match align {
                Alignment::None | Alignment::Left => format!(" {cell:<width$} "),
                Alignment::Center => format!(" {cell:^width$} "),
                Alignment::Right => format!(" {cell:>width$} "),
            }
        })
        .collect::<Vec<_>>()
        .join("|");
    format!("|{cells}|")
}

fn render_table_separator(widths: &[usize]) -> String {
    let cells = widths
        .iter()
        .map(|width| "─".repeat(width + 2))
        .collect::<Vec<_>>()
        .join("┼");
    format!("├{cells}┤")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Inline markup parsing tests ─────────────────────────────────────

    #[test]
    fn test_parse_inline_markup_plain_text() {
        let spans = parse_inline_markup("hello world");
        assert_eq!(spans, vec![InlineSpan::Text("hello world".to_string())]);
    }

    #[test]
    fn test_parse_inline_markup_bold() {
        let spans = parse_inline_markup("**bold**");
        assert_eq!(spans, vec![InlineSpan::Bold("bold".to_string())]);
    }

    #[test]
    fn test_parse_inline_markup_italic() {
        let spans = parse_inline_markup("*italic*");
        assert_eq!(spans, vec![InlineSpan::Italic("italic".to_string())]);
    }

    #[test]
    fn test_parse_inline_markup_code() {
        let spans = parse_inline_markup("`code`");
        assert_eq!(spans, vec![InlineSpan::Code("code".to_string())]);
    }

    #[test]
    fn test_parse_inline_markup_link() {
        let spans = parse_inline_markup("[label](url)");
        assert_eq!(
            spans,
            vec![InlineSpan::Link {
                label: "label".to_string(),
                target: "url".to_string(),
            }]
        );
    }

    #[test]
    fn test_parse_inline_markup_mixed() {
        let spans = parse_inline_markup("**bold** and *italic* and `code`");
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[0], InlineSpan::Bold("bold".to_string()));
        assert_eq!(spans[1], InlineSpan::Text(" and ".to_string()));
        assert_eq!(spans[2], InlineSpan::Italic("italic".to_string()));
        assert_eq!(spans[3], InlineSpan::Text(" and ".to_string()));
        assert_eq!(spans[4], InlineSpan::Code("code".to_string()));
    }

    #[test]
    fn test_parse_inline_markup_link_in_text() {
        let spans = parse_inline_markup("visit [example](https://example.com) now");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0], InlineSpan::Text("visit ".to_string()));
        assert_eq!(
            spans[1],
            InlineSpan::Link {
                label: "example".to_string(),
                target: "https://example.com".to_string(),
            }
        );
        assert_eq!(spans[2], InlineSpan::Text(" now".to_string()));
    }

    #[test]
    fn test_inline_to_plain_strips_markers() {
        let spans = parse_inline_markup("**bold** and *italic* and `code`");
        let plain = inline_to_plain(&spans);
        assert_eq!(plain, "bold and italic and code");
    }

    #[test]
    fn test_inline_to_plain_shows_link_url() {
        let spans = parse_inline_markup("try [Jaringan](https://jaringan.dev)");
        let plain = inline_to_plain(&spans);
        assert_eq!(plain, "try Jaringan (https://jaringan.dev)");
    }

    #[test]
    fn test_unmatched_markers_render_as_literal() {
        let spans = parse_inline_markup("*unmatched");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0], InlineSpan::Text("*".to_string()));
        assert_eq!(spans[1], InlineSpan::Text("unmatched".to_string()));

        let spans = parse_inline_markup("**partial");
        assert_eq!(spans, vec![InlineSpan::Text("**".to_string()), InlineSpan::Text("partial".to_string())]);
    }

    #[test]
    fn test_empty_bold_does_not_parse() {
        let spans = parse_inline_markup("****");
        assert!(spans.iter().all(|s| matches!(s, InlineSpan::Text(_))));
    }

    #[test]
    fn test_parse_inline_markup_empty() {
        assert!(parse_inline_markup("").is_empty());
    }

    // ── Legacy plain render tests ───────────────────────────────────────

    #[test]
    fn renders_plain_text_with_numbered_links() {
        let doc = jaringan_core::parse_document("# Hello\n\nWelcome.\n\n=> jrg://example/about About\n").unwrap();
        let rendered = render_plain(&doc);

        assert!(rendered.contains("# Hello"));
        assert!(rendered.contains("Welcome."));
        assert!(rendered.contains("[1] About <jrg://example/about>"));
    }

    #[test]
    fn renders_ratatui_text() {
        let doc = jaringan_core::parse_document("# Hello\n\n=> jrg://example/about About\n").unwrap();
        let text = render_ratatui_text(&doc);

        assert!(text.lines.len() >= 3);
    }

    #[test]
    fn renders_buttons_and_images_as_terminal_native_controls() {
        let doc = jaringan_core::parse_document(
            "# Rich\n\n! save label=\"Save\" target=\"save\"\n@ ./cover.png alt=\"Cover\"\n",
        )
        .unwrap();
        let rendered = render_plain(&doc);

        assert!(rendered.contains("[button] Save"));
        assert!(rendered.contains("[image] Cover <./cover.png>"));
    }

    #[test]
    fn renders_tables_quotes_lists_and_rules() {
        let doc = jaringan_core::parse_document(
            "# Layout\n\n> Polished terminal pages.\n\n- Tables\n- Quotes\n\n---\n\n| Name | Role |\n| --- | --- |\n| Simon | Builder |\n",
        )
        .unwrap();
        let rendered = render_plain(&doc);

        assert!(rendered.contains("> Polished terminal pages."));
        assert!(rendered.contains("• Tables"));
        assert!(rendered.contains("────────────────"));
        assert!(rendered.contains("| Name  | Role    |"));
        assert!(rendered.contains("| Simon | Builder |"));

        let tui = render_ratatui_text(&doc);
        assert!(tui
            .lines
            .iter()
            .any(|line| line.spans.iter().any(|span| span.content.contains("Name"))));
    }
}
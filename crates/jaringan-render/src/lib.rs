use jaringan_core::{Block, Document, Table};
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

static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(|| ThemeSet::load_defaults());

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
                output.push_str(text);
                output.push_str("\n\n");
            }
            Block::Paragraph(text) => {
                output.push_str(text);
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
                    output.push_str(&format!("> {line}\n"));
                }
                output.push('\n');
            }
            Block::List(items) => {
                for item in items {
                    output.push_str(&format!("• {item}\n"));
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
                lines.push(Line::from(vec![
                    Span::raw("#".repeat(*level as usize)),
                    Span::raw(" "),
                    Span::raw(text.clone()),
                ]));
                lines.push(Line::raw(""));
            }
            Block::Paragraph(text) => {
                lines.push(Line::raw(text.clone()));
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
                    lines.push(Line::raw(format!("┃ {line}")));
                }
                lines.push(Line::raw(""));
            }
            Block::List(items) => {
                for item in items {
                    lines.push(Line::raw(format!("  • {item}")));
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
    output.push_str(&render_table_row(&table.headers, &widths));
    output.push('\n');
    output.push_str(&render_table_separator(&widths));
    for row in &table.rows {
        output.push('\n');
        output.push_str(&render_table_row(row, &widths));
    }
    output
}

fn render_table_row(row: &[String], widths: &[usize]) -> String {
    let cells = widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            let cell = row.get(index).map(String::as_str).unwrap_or("");
            format!(" {cell:<width$} ")
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
    use jaringan_core::parse_document;

    use super::*;

    #[test]
    fn renders_plain_text_with_numbered_links() {
        let doc = parse_document("# Hello\n\nWelcome.\n\n=> jrg://example/about About\n").unwrap();
        let rendered = render_plain(&doc);

        assert!(rendered.contains("# Hello"));
        assert!(rendered.contains("Welcome."));
        assert!(rendered.contains("[1] About <jrg://example/about>"));
    }

    #[test]
    fn renders_ratatui_text() {
        let doc = parse_document("# Hello\n\n=> jrg://example/about About\n").unwrap();
        let text = render_ratatui_text(&doc);

        assert!(text.lines.len() >= 3);
    }

    #[test]
    fn renders_buttons_and_images_as_terminal_native_controls() {
        let doc = parse_document(
            "# Rich\n\n! save label=\"Save\" target=\"save\"\n@ ./cover.png alt=\"Cover\"\n",
        )
        .unwrap();
        let rendered = render_plain(&doc);

        assert!(rendered.contains("[button] Save"));
        assert!(rendered.contains("[image] Cover <./cover.png>"));
    }

    #[test]
    fn renders_tables_quotes_lists_and_rules() {
        let doc = parse_document(
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
        assert!(
            tui.lines
                .iter()
                .any(|line| line.spans.iter().any(|span| span.content.contains("Name")))
        );
    }
}

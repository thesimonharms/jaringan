use jaringan_core::{Block, Document};
use ratatui::text::{Line, Span, Text};

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
            Block::Preformatted(text) => {
                output.push_str("```\n");
                output.push_str(text);
                output.push_str("\n```\n\n");
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
            Block::Preformatted(text) => {
                lines.push(Line::raw("```"));
                lines.extend(text.lines().map(|line| Line::raw(line.to_owned())));
                lines.push(Line::raw("```"));
                lines.push(Line::raw(""));
            }
        }
    }

    Text::from(lines)
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
}

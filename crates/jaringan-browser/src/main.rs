use std::{
    fs,
    io::{self, Stdout},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use jaringan_browser::{BrowserState, PageLocation, go_back, navigate_to, resolve_target};
use jaringan_core::{Block, Document, Image, parse_document};
use jaringan_render::render_plain;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block as TuiBlock, Borders, Clear, Paragraph, Wrap},
};

#[derive(Debug, Parser)]
#[command(name = "jaringan-browser")]
#[command(about = "Terminal-native browser prototype for Jaringan pages")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse and render a local Jaringan page file.
    Sample { path: PathBuf },
    /// Open a local Jaringan page in the interactive terminal browser.
    Open { path: PathBuf },
}

#[derive(Debug, Clone)]
struct LoadedPage {
    path: PathBuf,
    document: Document,
    items: Vec<InteractiveItem>,
}

#[derive(Debug, Clone)]
enum InteractiveItem {
    Link { label: String, target: String },
    Button { label: String, target: String },
    Image(Image),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Sample { path } => {
            let source = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let document = parse_document(&source)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            println!("{}", render_plain(&document));
        }
        Command::Open { path } => run_tui(path)?,
    }

    Ok(())
}

fn run_tui(path: PathBuf) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, path);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    start_path: PathBuf,
) -> anyhow::Result<()> {
    let first = canonicalish(&start_path);
    let mut state = BrowserState::new(PageLocation::File(first.clone()));
    let mut page = load_page(&first)?;
    let started = Instant::now();

    loop {
        clamp_selection(&mut state, page.items.len());
        terminal.draw(|frame| draw(frame, &state, &page, started.elapsed()))?;

        if event::poll(Duration::from_millis(120))? {
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Down | KeyCode::Char('j') => {
                        if !page.items.is_empty() {
                            state.selected = (state.selected + 1).min(page.items.len() - 1);
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        state.selected = state.selected.saturating_sub(1);
                    }
                    KeyCode::Enter => activate_selected(&mut state, &mut page)?,
                    KeyCode::Char('b') | KeyCode::Backspace => {
                        if go_back(&mut state) {
                            if let PageLocation::File(path) = &state.current {
                                page = load_page(path)?;
                            }
                        }
                    }
                    KeyCode::Char('r') => {
                        page = load_page(&page.path)?;
                        state.status = String::from("Reloaded");
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    Ok(())
}

fn activate_selected(state: &mut BrowserState, page: &mut LoadedPage) -> anyhow::Result<()> {
    let Some(item) = page.items.get(state.selected).cloned() else {
        state.status = String::from("No selectable item");
        return Ok(());
    };

    match item {
        InteractiveItem::Link { label, target } => match resolve_target(&page.path, &target) {
            PageLocation::File(path) => {
                state.status = format!("⠋ Loading {}", path.display());
                let loaded = load_page(&path)?;
                navigate_to(state, PageLocation::File(loaded.path.clone()));
                state.status = format!("Opened {label}");
                *page = loaded;
            }
            PageLocation::Unsupported(target) => {
                state.status = format!("Unsupported target for now: {target}");
            }
        },
        InteractiveItem::Button { label, target } => {
            state.status = format!("Button pressed: {label} → {target}");
        }
        InteractiveItem::Image(image) => {
            state.status = image_status(&page.path, &image);
        }
    }

    Ok(())
}

fn image_status(page_path: &Path, image: &Image) -> String {
    if image.source.contains("://") {
        return format!(
            "Remote image declared: {} — downloader coming next",
            image.source
        );
    }

    let path = page_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&image.source);
    if path.exists() {
        format!("Local image available: {}", path.display())
    } else {
        format!("Image missing: {}", path.display())
    }
}

fn load_page(path: &Path) -> anyhow::Result<LoadedPage> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("jrg") {
        bail!(
            "Jaringan pages must use the .jrg extension: {}",
            path.display()
        );
    }

    let path = canonicalish(path);
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let document =
        parse_document(&source).with_context(|| format!("failed to parse {}", path.display()))?;
    let items = collect_items(&document);

    Ok(LoadedPage {
        path,
        document,
        items,
    })
}

fn collect_items(document: &Document) -> Vec<InteractiveItem> {
    document
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Link(link) => Some(InteractiveItem::Link {
                label: link.label.clone(),
                target: link.target.clone(),
            }),
            Block::Button(button) => Some(InteractiveItem::Button {
                label: button.label.clone(),
                target: button.target.clone(),
            }),
            Block::Image(image) => Some(InteractiveItem::Image(image.clone())),
            _ => None,
        })
        .collect()
}

fn draw(
    frame: &mut ratatui::Frame<'_>,
    state: &BrowserState,
    page: &LoadedPage,
    elapsed: Duration,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(3),
        ])
        .split(area);

    let title = page.document.title().unwrap_or("Untitled Jaringan page");
    let header = Paragraph::new(Line::from(vec![
        Span::styled("✦ jaringan ", Style::default().fg(Color::Cyan).bold()),
        Span::styled(title.to_owned(), Style::default().fg(Color::White).bold()),
    ]))
    .block(
        TuiBlock::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(header, chunks[0]);

    let body = Paragraph::new(render_lines(page, state.selected))
        .block(
            TuiBlock::default()
                .borders(Borders::LEFT | Borders::RIGHT)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(body, chunks[1]);

    let spinner = spinner(elapsed);
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {spinner} "), Style::default().fg(Color::Magenta)),
        Span::styled(&state.status, Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(
            "↑/k ↓/j select • enter open/press/view • b back • r reload • q quit",
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(format!("  {}", page.path.display())),
    ]))
    .block(
        TuiBlock::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(footer, chunks[2]);
}

fn render_lines(page: &LoadedPage, selected: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut item_index = 0usize;

    for block in &page.document.blocks {
        match block {
            Block::Heading { level, text } => {
                let color = match level {
                    1 => Color::Cyan,
                    2 => Color::Blue,
                    _ => Color::White,
                };
                lines.push(Line::from(Span::styled(
                    format!("{} {}", "#".repeat(*level as usize), text),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::raw(""));
            }
            Block::Paragraph(text) => {
                lines.push(Line::from(Span::styled(
                    text.clone(),
                    Style::default().fg(Color::Gray),
                )));
                lines.push(Line::raw(""));
            }
            Block::Link(link) => {
                lines.push(selectable_line(
                    selected == item_index,
                    format!("↳ {}", link.label),
                    format!("{}", link.target),
                    Color::Green,
                ));
                item_index += 1;
            }
            Block::Button(button) => {
                lines.push(selectable_line(
                    selected == item_index,
                    format!("◉ {}", button.label),
                    button.target.clone(),
                    Color::Magenta,
                ));
                item_index += 1;
            }
            Block::Image(image) => {
                lines.push(selectable_line(
                    selected == item_index,
                    format!("▧ {}", image.alt),
                    image.source.clone(),
                    Color::Yellow,
                ));
                item_index += 1;
            }
            Block::Preformatted(text) => {
                lines.push(Line::from(Span::styled(
                    "╭─",
                    Style::default().fg(Color::DarkGray),
                )));
                for line in text.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                        Span::styled(line.to_owned(), Style::default().fg(Color::White)),
                    ]));
                }
                lines.push(Line::from(Span::styled(
                    "╰─",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::raw(""));
            }
        }
    }

    lines
}

fn selectable_line(selected: bool, label: String, target: String, color: Color) -> Line<'static> {
    let marker = if selected { "❯ " } else { "  " };
    let style = if selected {
        Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };

    Line::from(vec![
        Span::styled(marker, style),
        Span::styled(label, style),
        Span::styled("  ", style),
        Span::styled(target, style.add_modifier(Modifier::DIM)),
    ])
}

fn clamp_selection(state: &mut BrowserState, item_count: usize) {
    if item_count == 0 {
        state.selected = 0;
    } else if state.selected >= item_count {
        state.selected = item_count - 1;
    }
}

fn spinner(elapsed: Duration) -> &'static str {
    const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let index = (elapsed.as_millis() / 120) as usize % FRAMES.len();
    FRAMES[index]
}

fn canonicalish(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

use std::{
    fs,
    io::{self, Stdout},
    net::TcpListener,
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
use jaringan_browser::{
    BrowserMode, BrowserState, PageLocation, cache_filename_for_url, go_back, navigate_to,
    resolve_target, scroll_down, scroll_up, selection_down, selection_up, switch_mode, toggle_mode,
};
use jaringan_core::{Block, Document, Image, parse_document};
use jaringan_protocol::{
    ContentType, JaringanUrl, LocalFileResolver, PageResolver, Request, Response, ResponseTag,
    StatusCode, fetch_tcp, serve,
};
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
    /// Fetch a jrg:// URL from a local document root using protocol path rules.
    Fetch { root: PathBuf, url: String },
    /// Fetch a jrg:// URL over the TCP wire protocol.
    Get {
        url: String,
        /// Follow Tag-Redirect responses before printing the final response.
        #[arg(long)]
        follow: bool,
    },
    /// Serve a local document root over the TCP wire protocol.
    Serve {
        root: PathBuf,
        #[arg(long, default_value = "127.0.0.1:7070")]
        bind: String,
    },
    /// Open a local path or jrg:// URL in the interactive terminal browser.
    Open { target: String },
}

#[derive(Debug, Clone)]
struct LoadedPage {
    location: PageLocation,
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
        Command::Fetch { root, url } => {
            let url = JaringanUrl::parse(&url)?;
            let resolver = LocalFileResolver::new(root);
            let response = resolver.fetch(&Request::new(url))?;
            print_response(response);
        }
        Command::Get { url, follow } => {
            let url = JaringanUrl::parse(&url)?;
            let response = if follow {
                fetch_response_following_redirects(url)?.1
            } else {
                fetch_tcp(&url)?
            };
            print_response(response);
        }
        Command::Serve { root, bind } => {
            let listener = TcpListener::bind(&bind)
                .with_context(|| format!("failed to bind Jaringan server to {bind}"))?;
            eprintln!("serving {} at jrg://{bind}/", root.display());
            serve(listener, LocalFileResolver::new(root))?;
        }
        Command::Open { target } => run_tui(target)?,
    }

    Ok(())
}

fn print_response(response: Response) {
    println!(
        "JRG/0.1 {} {}",
        response.status.as_u16(),
        response.status.reason_phrase()
    );
    println!("Content-Type: {}", response.content_type.as_str());
    for tag in response.tags {
        match tag {
            ResponseTag::Redirect { target } => println!("Tag-Redirect: {target}"),
        }
    }
    println!();
    print!("{}", response.body);
}

fn fetch_response_following_redirects(
    mut url: JaringanUrl,
) -> anyhow::Result<(JaringanUrl, Response)> {
    const MAX_REDIRECTS: usize = 5;

    for redirect_count in 0..=MAX_REDIRECTS {
        let response = fetch_tcp(&url)?;
        let Some(target) = redirect_target(&response) else {
            return Ok((url, response));
        };

        if redirect_count == MAX_REDIRECTS {
            bail!("too many redirects while fetching {url}");
        }
        url = url
            .resolve(target)
            .with_context(|| format!("bad redirect target `{target}` while fetching {url}"))?;
    }

    unreachable!("redirect loop exits by returning once MAX_REDIRECTS is reached")
}

fn run_tui(target: String) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, target);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    target: String,
) -> anyhow::Result<()> {
    let first = parse_start_location(&target)?;
    let mut state = BrowserState::new(first.clone());
    let mut page = load_location(&first)?;
    let started = Instant::now();

    loop {
        clamp_selection(&mut state, page.items.len());
        terminal.draw(|frame| draw(frame, &state, &page, started.elapsed()))?;

        if event::poll(Duration::from_millis(120))? {
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Tab => toggle_mode(&mut state),
                    KeyCode::Char('s') => switch_mode(&mut state, BrowserMode::Scroll),
                    KeyCode::Char('v') => switch_mode(&mut state, BrowserMode::Selection),
                    KeyCode::Down | KeyCode::Char('j') => match state.mode {
                        BrowserMode::Selection => selection_down(&mut state, page.items.len()),
                        BrowserMode::Scroll => {
                            let line_count = render_lines(&page, state.selected).len();
                            let viewport_height = terminal.size()?.height.saturating_sub(6);
                            scroll_down(&mut state, line_count, viewport_height);
                        }
                    },
                    KeyCode::Up | KeyCode::Char('k') => match state.mode {
                        BrowserMode::Selection => selection_up(&mut state),
                        BrowserMode::Scroll => scroll_up(&mut state),
                    },
                    KeyCode::Enter => activate_selected(&mut state, &mut page)?,
                    KeyCode::Char('b') | KeyCode::Backspace => {
                        if go_back(&mut state) {
                            match &state.current {
                                location @ (PageLocation::File(_) | PageLocation::Network(_)) => {
                                    page = load_location(location)?;
                                }
                                PageLocation::Unsupported(_) => {}
                            }
                        }
                    }
                    KeyCode::Char('r') => {
                        page = load_location(&page.location)?;
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
        InteractiveItem::Link { label, target } => match resolve_target(&page.location, &target) {
            location @ (PageLocation::File(_) | PageLocation::Network(_)) => {
                state.status = format!("⠋ Loading {}", location_label(&location));
                let loaded = load_location(&location)?;
                navigate_to(state, loaded.location.clone());
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
            state.status = image_status(&page.location, &image);
        }
    }

    Ok(())
}

fn image_status(page_location: &PageLocation, image: &Image) -> String {
    if image.source.starts_with("http://") || image.source.starts_with("https://") {
        return download_remote_image(&image.source);
    }

    let PageLocation::File(page_path) = page_location else {
        return format!("Network image reference: {}", image.source);
    };

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

fn download_remote_image(url: &str) -> String {
    let cache_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/jaringan/images");

    if let Err(error) = fs::create_dir_all(&cache_dir) {
        return format!("Could not create image cache: {error}");
    }

    let output_path = cache_dir.join(cache_filename_for_url(url));
    let status = std::process::Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(&output_path)
        .arg(url)
        .status();

    match status {
        Ok(status) if status.success() => format!("Downloaded image: {}", output_path.display()),
        Ok(status) => format!("Image download failed with status: {status}"),
        Err(error) => format!("Image download requires curl: {error}"),
    }
}

fn load_location(location: &PageLocation) -> anyhow::Result<LoadedPage> {
    match location {
        PageLocation::File(path) => load_file_page(path),
        PageLocation::Network(url) => load_network_page(url),
        PageLocation::Unsupported(target) => bail!("unsupported target: {target}"),
    }
}

fn load_file_page(path: &Path) -> anyhow::Result<LoadedPage> {
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
        location: PageLocation::File(path),
        document,
        items,
    })
}

fn load_network_page(url: &JaringanUrl) -> anyhow::Result<LoadedPage> {
    const MAX_REDIRECTS: usize = 5;

    let mut current = url.clone();
    for redirect_count in 0..=MAX_REDIRECTS {
        let response = match fetch_tcp(&current) {
            Ok(response) => response,
            Err(error) => {
                return Ok(network_error_page(
                    current,
                    format!("failed to fetch {url}: {error}"),
                ));
            }
        };

        if let Some(target) = redirect_target(&response) {
            if redirect_count == MAX_REDIRECTS {
                return Ok(network_error_page(
                    current,
                    format!("too many redirects while fetching {url}"),
                ));
            }

            current = match current.resolve(target) {
                Ok(next) => next,
                Err(error) => {
                    return Ok(network_error_page(
                        current,
                        format!("bad redirect target `{target}` while fetching {url}: {error}"),
                    ));
                }
            };
            continue;
        }

        let document = document_from_response(&response)
            .with_context(|| format!("failed to parse response from {current}"))?;
        let items = collect_items(&document);

        return Ok(LoadedPage {
            location: PageLocation::Network(current),
            document,
            items,
        });
    }

    unreachable!("redirect loop exits by returning once MAX_REDIRECTS is reached")
}

fn redirect_target(response: &Response) -> Option<&str> {
    response.tags.first().map(|tag| match tag {
        ResponseTag::Redirect { target } => target.as_str(),
    })
}

fn network_error_page(location: JaringanUrl, message: String) -> LoadedPage {
    let document = Document::new(vec![
        Block::Heading {
            level: 1,
            text: "Network error".to_owned(),
        },
        Block::Preformatted(message),
    ]);

    LoadedPage {
        location: PageLocation::Network(location),
        document,
        items: Vec::new(),
    }
}

fn document_from_response(response: &Response) -> anyhow::Result<Document> {
    if response.content_type == ContentType::JaringanPage {
        return parse_document(&response.body).context("failed to parse Jaringan page body");
    }

    let status = response.status;
    let text = if status == StatusCode::Ok {
        response.body.clone()
    } else {
        format!(
            "JRG/0.1 {} {}\n\n{}",
            status.as_u16(),
            status.reason_phrase(),
            response.body
        )
    };

    Ok(Document::new(vec![Block::Preformatted(text)]))
}

fn parse_start_location(target: &str) -> anyhow::Result<PageLocation> {
    if target.starts_with("jrg://") {
        return Ok(PageLocation::Network(JaringanUrl::parse(target)?));
    }
    Ok(PageLocation::File(canonicalish(Path::new(target))))
}

fn location_label(location: &PageLocation) -> String {
    match location {
        PageLocation::File(path) => path.display().to_string(),
        PageLocation::Network(url) => url.to_string(),
        PageLocation::Unsupported(target) => target.clone(),
    }
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
        .scroll((state.scroll_offset, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(body, chunks[1]);

    let spinner = spinner(elapsed);
    let mode_label = match state.mode {
        BrowserMode::Selection => "SELECTION",
        BrowserMode::Scroll => "SCROLL",
    };
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {spinner} "), Style::default().fg(Color::Magenta)),
        Span::styled(format!("{mode_label} "), Style::default().fg(Color::Cyan).bold()),
        Span::styled(&state.status, Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(
            "tab toggle • s scroll • v selection • j/k move • enter open/press/view • b back • r reload • q quit",
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(format!("  {}", location_label(&page.location))),
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
                    link.target.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct RedirectResolver;

    impl PageResolver for RedirectResolver {
        fn fetch(&self, request: &Request) -> Result<Response, jaringan_protocol::ResolveError> {
            match request.url.path() {
                "/old.jrg" => Ok(Response::page(StatusCode::Found, "redirecting").with_tag(
                    ResponseTag::Redirect {
                        target: "new.jrg".to_owned(),
                    },
                )),
                "/new.jrg" => Ok(Response::page(StatusCode::Ok, "# New Page\n")),
                path => Ok(Response::text(
                    StatusCode::NotFound,
                    format!("missing {path}"),
                )),
            }
        }
    }

    #[test]
    fn network_loader_follows_redirect_tags_to_final_page() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let resolver = RedirectResolver;
            jaringan_protocol::serve_one(listener.try_clone().unwrap(), resolver.clone()).unwrap();
            jaringan_protocol::serve_one(listener, resolver).unwrap();
        });

        let loaded =
            load_network_page(&JaringanUrl::parse(&format!("jrg://{addr}/old.jrg")).unwrap())
                .unwrap();
        server.join().unwrap();

        assert_eq!(
            loaded.location,
            PageLocation::Network(JaringanUrl::parse(&format!("jrg://{addr}/new.jrg")).unwrap())
        );
        assert_eq!(loaded.document.title(), Some("New Page"));
    }

    #[test]
    fn network_loader_returns_error_page_when_fetch_fails() {
        let loaded =
            load_network_page(&JaringanUrl::parse("jrg://127.0.0.1:1/missing.jrg").unwrap())
                .unwrap();
        let lines = render_plain(&loaded.document);

        assert!(lines.contains("Network error"));
        assert!(lines.contains("jrg://127.0.0.1:1/missing.jrg"));
    }

    #[test]
    fn cli_fetch_follow_redirects_returns_final_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let resolver = RedirectResolver;
            jaringan_protocol::serve_one(listener.try_clone().unwrap(), resolver.clone()).unwrap();
            jaringan_protocol::serve_one(listener, resolver).unwrap();
        });

        let (final_url, response) = fetch_response_following_redirects(
            JaringanUrl::parse(&format!("jrg://{addr}/old.jrg")).unwrap(),
        )
        .unwrap();
        server.join().unwrap();

        assert_eq!(final_url.as_str(), format!("jrg://{addr}/new.jrg"));
        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.body, "# New Page\n");
    }
}

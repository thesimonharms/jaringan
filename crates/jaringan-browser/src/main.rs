use std::{
    env, fs,
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
    ActionConfirmation, BrowserMode, BrowserState, PageLocation, cache_filename_for_url, go_back,
    go_forward, navigate_to, resolve_target, scroll_down, scroll_page_down, scroll_page_up,
    scroll_to_bottom, scroll_to_top, scroll_up, selection_down, selection_first, selection_last,
    selection_up, switch_mode, toggle_help, toggle_mode,
};
use jaringan_core::{
    ActionMethod, Block, Document, Image, Input, PublicKeyring, SearchEntry, SearchIndex,
    SignatureStatus, Table, parse_document, verify_source_signature,
};
use jaringan_protocol::{
    ContentType, EncryptedTcpConfig, EncryptionKey, JaringanUrl, LocalFileResolver, PageResolver,
    Request, Response, ResponseTag, StatusCode, fetch_tcp, fetch_tcp_encrypted, post_tcp,
    post_tcp_with_action_token, serve, serve_encrypted,
};
use jaringan_render::render_plain;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
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
        /// Use encrypted TCP with this key id and JARINGAN_ENCRYPTION_KEY_HEX.
        #[arg(long)]
        encrypted_key_id: Option<String>,
    },
    /// Serve a local document root over the TCP wire protocol.
    Serve {
        root: PathBuf,
        #[arg(long, default_value = "127.0.0.1:7070")]
        bind: String,
        /// Require encrypted TCP with this key id and JARINGAN_ENCRYPTION_KEY_HEX.
        #[arg(long)]
        encrypted_key_id: Option<String>,
    },
    /// Print a local search index of all .jrg pages under a root.
    Index {
        root: PathBuf,
        /// Persist the generated index to this file.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Search local .jrg pages under a root.
    Search {
        root: PathBuf,
        query: String,
        /// Load an existing persisted index instead of crawling the root.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// Open a local path or jrg:// URL in the interactive terminal browser.
    Open { target: String },
}

#[derive(Debug, Clone)]
struct LoadedPage {
    location: PageLocation,
    document: Document,
    items: Vec<InteractiveItem>,
    signature_status: SignatureStatus,
}

#[derive(Debug, Clone)]
struct ButtonAction {
    id: String,
    label: String,
    target: String,
    method: ActionMethod,
    confirm: Option<String>,
    auth: Option<String>,
}

#[derive(Debug, Clone)]
enum InteractiveItem {
    Link { label: String, target: String },
    Input(Input),
    Button(ButtonAction),
    Image(Image),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InputEdit {
    Append(char),
    Backspace,
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
        Command::Get {
            url,
            follow,
            encrypted_key_id,
        } => {
            let url = JaringanUrl::parse(&url)?;
            let encrypted_config = encrypted_key_id
                .as_deref()
                .map(encrypted_tcp_config_from_env)
                .transpose()?;
            let response = if follow {
                fetch_response_following_redirects_with_encryption(url, encrypted_config.as_ref())?
                    .1
            } else if let Some(config) = encrypted_config.as_ref() {
                fetch_tcp_encrypted(&url, config)?
            } else {
                fetch_tcp(&url)?
            };
            print_response(response);
        }
        Command::Serve {
            root,
            bind,
            encrypted_key_id,
        } => {
            let listener = TcpListener::bind(&bind)
                .with_context(|| format!("failed to bind Jaringan server to {bind}"))?;
            if let Some(key_id) = encrypted_key_id {
                let config = encrypted_tcp_config_from_env(&key_id)?;
                eprintln!("serving encrypted {} at jrg://{bind}/", root.display());
                serve_encrypted(listener, LocalFileResolver::new(root), &config)?;
            } else {
                eprintln!("serving {} at jrg://{bind}/", root.display());
                serve(listener, LocalFileResolver::new(root))?;
            }
        }
        Command::Index { root, output } => {
            let index = build_local_search_index(&root)?;
            if let Some(path) = output {
                save_search_index(&index, &path)?;
            }
            for entry in index.entries() {
                println!("{}\t{}", entry.url, entry.title);
            }
        }
        Command::Search { root, query, index } => {
            let index = match index {
                Some(path) => load_search_index(&path)?,
                None => build_local_search_index(&root)?,
            };
            for result in index.search(&query) {
                println!(
                    "{}\t{}\t{}\t{}",
                    result.score, result.entry.url, result.entry.title, result.snippet
                );
            }
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

fn build_local_search_index(root: &Path) -> anyhow::Result<SearchIndex> {
    let root = canonicalish(root);
    let mut index = SearchIndex::default();
    collect_search_entries(&root, &root, &mut index)?;
    Ok(index)
}

fn save_search_index(index: &SearchIndex, path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, index.to_index_text())
        .with_context(|| format!("failed to write search index {}", path.display()))
}

fn load_search_index(path: &Path) -> anyhow::Result<SearchIndex> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read search index {}", path.display()))?;
    SearchIndex::from_index_text(&source)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("failed to parse search index {}", path.display()))
}

fn collect_search_entries(
    root: &Path,
    current: &Path,
    index: &mut SearchIndex,
) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(current)
        .with_context(|| format!("failed to read directory {}", current.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_search_entries(root, &path, index)?;
        } else if path.extension().is_some_and(|extension| extension == "jrg") {
            let source = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let document = parse_document(&source)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            index.add(SearchEntry::from_document(
                jrg_url_for_path(root, &path),
                &document,
            ));
        }
    }
    Ok(())
}

fn jrg_url_for_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let relative = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    format!("jrg://local/{relative}")
}

fn fetch_response_following_redirects_with_encryption(
    mut url: JaringanUrl,
    encrypted_config: Option<&EncryptedTcpConfig>,
) -> anyhow::Result<(JaringanUrl, Response)> {
    const MAX_REDIRECTS: usize = 5;

    for redirect_count in 0..=MAX_REDIRECTS {
        let response = if let Some(config) = encrypted_config {
            fetch_tcp_encrypted(&url, config)?
        } else {
            fetch_tcp(&url)?
        };
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

fn encrypted_tcp_config_from_env(key_id: &str) -> anyhow::Result<EncryptedTcpConfig> {
    let key_hex = env::var("JARINGAN_ENCRYPTION_KEY_HEX")
        .context("JARINGAN_ENCRYPTION_KEY_HEX is required for encrypted TCP")?;
    Ok(EncryptedTcpConfig::new(
        key_id.to_owned(),
        EncryptionKey::from_bytes(parse_32_byte_hex_key(&key_hex)?),
    ))
}

fn parse_32_byte_hex_key(input: &str) -> anyhow::Result<[u8; 32]> {
    let trimmed = input.trim();
    if trimmed.len() != 64 {
        bail!("JARINGAN_ENCRYPTION_KEY_HEX must be exactly 64 hex characters");
    }

    let mut bytes = [0; 32];
    for (index, chunk) in trimmed.as_bytes().chunks_exact(2).enumerate() {
        let hex = std::str::from_utf8(chunk).context("encryption key contains invalid UTF-8")?;
        bytes[index] = u8::from_str_radix(hex, 16)
            .with_context(|| format!("invalid hex byte `{hex}` in JARINGAN_ENCRYPTION_KEY_HEX"))?;
    }
    Ok(bytes)
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
        let frame_result = terminal.draw(|frame| draw(frame, &state, &page, started.elapsed()));
        if state.show_help {
            terminal.draw(draw_help_overlay)?;
        } else if let Err(e) = frame_result {
            return Err(e.into());
        }

        if event::poll(Duration::from_millis(120))? {
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char(ch) if is_selected_input(&page, state.selected) => {
                        edit_selected_input(&mut state, &mut page, InputEdit::Append(ch));
                    }
                    KeyCode::Backspace if is_selected_input(&page, state.selected) => {
                        edit_selected_input(&mut state, &mut page, InputEdit::Backspace);
                    }
                    KeyCode::Tab => toggle_mode(&mut state),
                    KeyCode::Char('s') => switch_mode(&mut state, BrowserMode::Scroll),
                    KeyCode::Char('v') => switch_mode(&mut state, BrowserMode::Selection),
                    KeyCode::Char('?') | KeyCode::Char('h') => toggle_help(&mut state),
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
                    KeyCode::PageDown | KeyCode::Char(' ') => match state.mode {
                        BrowserMode::Selection => selection_down(&mut state, page.items.len()),
                        BrowserMode::Scroll => {
                            let line_count = render_lines(&page, state.selected).len();
                            let viewport_height = terminal.size()?.height.saturating_sub(6);
                            scroll_page_down(&mut state, line_count, viewport_height);
                        }
                    },
                    KeyCode::PageUp => match state.mode {
                        BrowserMode::Selection => selection_up(&mut state),
                        BrowserMode::Scroll => {
                            let line_count = render_lines(&page, state.selected).len();
                            let viewport_height = terminal.size()?.height.saturating_sub(6);
                            scroll_page_up(&mut state, line_count, viewport_height);
                        }
                    },
                    KeyCode::Home => selection_first(&mut state),
                    KeyCode::End => selection_last(&mut state, page.items.len()),
                    KeyCode::Char('g') => scroll_to_top(&mut state),
                    KeyCode::Char('G') => {
                        let line_count = render_lines(&page, state.selected).len();
                        let viewport_height = terminal.size()?.height.saturating_sub(6);
                        scroll_to_bottom(&mut state, line_count, viewport_height);
                    }
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
                    KeyCode::Char('f') => {
                        if go_forward(&mut state) {
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
        InteractiveItem::Button(action) => activate_button(state, page, action)?,
        InteractiveItem::Input(input) => {
            state.pending_confirmation = None;
            state.status = input_status(&input);
        }
        InteractiveItem::Image(image) => {
            state.pending_confirmation = None;
            state.status = image_status(&page.location, &image);
        }
    }

    Ok(())
}

fn activate_button(
    state: &mut BrowserState,
    page: &mut LoadedPage,
    action: ButtonAction,
) -> anyhow::Result<()> {
    let ButtonAction {
        id,
        label,
        target,
        method,
        confirm,
        auth,
    } = action;

    if let Some(prompt) = confirm {
        let already_confirmed = state
            .pending_confirmation
            .as_ref()
            .is_some_and(|action| action.id == id && action.target == target);
        if !already_confirmed {
            state.pending_confirmation = Some(ActionConfirmation { id, target });
            state.status = format!("{prompt} Press Enter again to confirm.");
            return Ok(());
        }
    }

    state.pending_confirmation = None;
    let payload = input_payload(page);
    let target_with_payload = target_with_payload(&target, &payload);

    match (&page.location, method) {
        (PageLocation::Network(current), ActionMethod::Post) => {
            let action_url = current
                .resolve(&target)
                .with_context(|| format!("bad action target `{target}`"))?;
            let response = if let Some(token) = auth.as_deref() {
                post_tcp_with_action_token(&action_url, payload, token)?
            } else {
                post_tcp(&action_url, payload)?
            };
            let document = document_from_response(&response)?;
            navigate_to(state, PageLocation::Network(action_url));
            state.status = format!("Submitted POST action: {target_with_payload}");
            *page = LoadedPage {
                location: state.current.clone(),
                items: collect_items(&document),
                document,
                signature_status: SignatureStatus::Unsigned,
            };
        }
        (PageLocation::File(current_file), ActionMethod::Post) if target.starts_with("/") => {
            let root = current_file.parent().unwrap_or_else(|| Path::new("."));
            let action_url = JaringanUrl::parse(&format!("jrg://localhost{target}"))?;
            let resolver = LocalFileResolver::new(root);
            let mut request = Request::post(action_url, payload);
            if let Some(token) = auth.as_deref() {
                request = request.with_action_token(token);
            }
            let response = resolver.fetch(&request)?;
            let document = document_from_response(&response)?;
            state.status = format!("Submitted local POST action: {target_with_payload}");
            *page = LoadedPage {
                location: page.location.clone(),
                items: collect_items(&document),
                document,
                signature_status: SignatureStatus::Unsigned,
            };
        }
        (PageLocation::File(current_file), ActionMethod::Get) if target == "/search" => {
            let root = current_file.parent().unwrap_or_else(|| Path::new("."));
            let query = payload_value(&payload, "q").unwrap_or_default();
            let index = build_local_search_index(root)?;
            let document = local_search_results_document(&index, &query);
            state.status = format!("Searched local index for: {query}");
            *page = LoadedPage {
                location: page.location.clone(),
                items: collect_items(&document),
                document,
                signature_status: SignatureStatus::Unsigned,
            };
        }
        _ => {
            state.status = match method {
                ActionMethod::Get => format!("Confirmed GET action: {target_with_payload}"),
                ActionMethod::Post => format!("Confirmed POST action: {target_with_payload}"),
            };
        }
    }

    if label.is_empty() {
        state.status.push_str(" (unnamed action)");
    }
    Ok(())
}

fn is_selected_input(page: &LoadedPage, selected: usize) -> bool {
    matches!(page.items.get(selected), Some(InteractiveItem::Input(_)))
}

fn edit_selected_input(state: &mut BrowserState, page: &mut LoadedPage, edit: InputEdit) {
    let Some(InteractiveItem::Input(selected_input)) = page.items.get(state.selected) else {
        return;
    };
    let selected_name = selected_input.name.clone();

    for block in &mut page.document.blocks {
        let Block::Input(input) = block else {
            continue;
        };
        if input.name != selected_name {
            continue;
        }
        match edit {
            InputEdit::Append(ch) => input.value.push(ch),
            InputEdit::Backspace => {
                input.value.pop();
            }
        }
        state.pending_confirmation = None;
        state.status = format!("Updated {} = {}", input.name, input.value);
        page.items = collect_items(&page.document);
        return;
    }
}

fn input_payload(page: &LoadedPage) -> String {
    page.document
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Input(input) => Some(format!(
                "{}={}",
                percent_encode(&input.name),
                percent_encode(&input.value)
            )),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn target_with_payload(target: &str, payload: &str) -> String {
    if payload.is_empty() {
        return target.to_owned();
    }
    let separator = if target.contains('?') { '&' } else { '?' };
    format!("{target}{separator}{payload}")
}

fn payload_value(payload: &str, name: &str) -> Option<String> {
    payload.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (percent_decode(key) == name).then(|| percent_decode(value))
    })
}

fn percent_decode(input: &str) -> String {
    let mut output = Vec::new();
    let bytes = input.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let decoded = std::str::from_utf8(&bytes[index + 1..index + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(value) = decoded {
                output.push(value);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn local_search_results_document(index: &SearchIndex, query: &str) -> Document {
    let mut blocks = vec![Block::Heading {
        level: 1,
        text: format!("Search results for {query}"),
    }];
    let results = index.search(query);
    if results.is_empty() {
        blocks.push(Block::Paragraph(String::from("No results found.")));
    } else {
        for result in results {
            blocks.push(Block::Link(jaringan_core::Link {
                target: local_result_target(&result.entry.url),
                label: format!("{} — {}", result.entry.title, result.snippet),
            }));
        }
    }
    Document::new(blocks)
}

fn local_result_target(url: &str) -> String {
    url.strip_prefix("jrg://local/").unwrap_or(url).to_owned()
}

fn percent_encode(input: &str) -> String {
    let mut output = String::new();
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn input_status(input: &Input) -> String {
    let value = if input.value.is_empty() {
        input.placeholder.as_deref().unwrap_or("")
    } else {
        &input.value
    };
    format!("Input {} = {}", input.name, value)
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
    let keyring = default_keyring();
    match location {
        PageLocation::File(path) => load_file_page_with_keyring(path, &keyring),
        PageLocation::Network(url) => load_network_page_with_keyring(url, &keyring),
        PageLocation::Unsupported(target) => bail!("unsupported target: {target}"),
    }
}

fn load_file_page_with_keyring(path: &Path, keyring: &PublicKeyring) -> anyhow::Result<LoadedPage> {
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
    let signature_status = verify_source_signature(&source, keyring);

    Ok(LoadedPage {
        location: PageLocation::File(path),
        document,
        items,
        signature_status,
    })
}

fn load_network_page_with_keyring(
    url: &JaringanUrl,
    keyring: &PublicKeyring,
) -> anyhow::Result<LoadedPage> {
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
            signature_status: verify_source_signature(&response.body, keyring),
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
        signature_status: SignatureStatus::Unsigned,
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

fn default_keyring() -> PublicKeyring {
    let path = default_keyring_path();
    if !path.exists() {
        return PublicKeyring::default();
    }

    match load_keyring_file(&path) {
        Ok(keyring) => keyring,
        Err(error) => {
            eprintln!(
                "warning: failed to load keyring {}: {error}",
                path.display()
            );
            PublicKeyring::default()
        }
    }
}

fn default_keyring_path() -> PathBuf {
    std::env::var_os("JARINGAN_KEYRING")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".config/jaringan/keyring"))
        })
        .unwrap_or_else(|| PathBuf::from(".config/jaringan/keyring"))
}

fn load_keyring_file(path: &Path) -> anyhow::Result<PublicKeyring> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read keyring {}", path.display()))?;
    PublicKeyring::from_text(&source)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("failed to parse keyring {}", path.display()))
}

fn security_label(status: &SignatureStatus) -> String {
    match status {
        SignatureStatus::Secure { signer } => format!("secure: signed by {signer}"),
        SignatureStatus::Unsigned => String::from("not secure: unsigned"),
        SignatureStatus::UnknownSigner { signer } => {
            format!("not secure: unknown signer {signer}")
        }
        SignatureStatus::Invalid { reason } => format!("not secure: {reason}"),
    }
}

fn security_style(status: &SignatureStatus) -> Style {
    match status {
        SignatureStatus::Secure { .. } => Style::default().fg(Color::Green).bold(),
        SignatureStatus::Unsigned => Style::default().fg(Color::DarkGray),
        SignatureStatus::UnknownSigner { .. } | SignatureStatus::Invalid { .. } => {
            Style::default().fg(Color::Yellow).bold()
        }
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
            Block::Input(input) => Some(InteractiveItem::Input(input.clone())),
            Block::Button(button) => Some(InteractiveItem::Button(ButtonAction {
                id: button.id.clone(),
                label: button.label.clone(),
                target: button.target.clone(),
                method: button.method,
                confirm: button.confirm.clone(),
                auth: button.auth.clone(),
            })),
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
        Span::raw("  "),
        Span::styled(
            security_label(&page.signature_status),
            security_style(&page.signature_status),
        ),
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

fn draw_help_overlay(frame: &mut ratatui::Frame<'_>) {
    let area = frame.area();
    let overlay_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Max(area.height.saturating_sub(4)),
        ])
        .split(area);

    let help_block = Paragraph::new(Text::from(help_lines()))
        .block(
            TuiBlock::default()
                .title(" Help ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan).bold()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, overlay_area[1]);
    frame.render_widget(help_block, overlay_area[1]);
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
            Block::Input(input) => {
                lines.push(selectable_line(
                    selected == item_index,
                    format!("▣ {}", input.label),
                    input.value.clone(),
                    Color::LightBlue,
                ));
                item_index += 1;
            }
            Block::Button(button) => {
                lines.push(selectable_line(
                    selected == item_index,
                    format!("◉ {}", button.label),
                    format!("{} {}", button.method.as_str(), button.target),
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
            Block::Quote(text) => {
                for line in text.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("┃ ", Style::default().fg(Color::LightMagenta).bold()),
                        Span::styled(
                            line.to_owned(),
                            Style::default().fg(Color::LightMagenta).italic(),
                        ),
                    ]));
                }
                lines.push(Line::raw(""));
            }
            Block::List(items) => {
                for item in items {
                    lines.push(Line::from(vec![
                        Span::styled("  ◆ ", Style::default().fg(Color::Cyan)),
                        Span::styled(item.clone(), Style::default().fg(Color::Gray)),
                    ]));
                }
                lines.push(Line::raw(""));
            }
            Block::Rule => {
                lines.push(Line::from(Span::styled(
                    "  ────────────────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::raw(""));
            }
            Block::Table(table) => {
                for line in render_browser_table(table) {
                    lines.push(line);
                }
                lines.push(Line::raw(""));
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

fn render_browser_table(table: &Table) -> Vec<Line<'static>> {
    let columns = table
        .headers
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return Vec::new();
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

    let mut lines = Vec::new();
    lines.push(browser_table_row(&table.headers, &widths, true));
    lines.push(Line::from(Span::styled(
        browser_table_separator(&widths),
        Style::default().fg(Color::DarkGray),
    )));
    for row in &table.rows {
        lines.push(browser_table_row(row, &widths, false));
    }
    lines
}

fn browser_table_row(row: &[String], widths: &[usize], header: bool) -> Line<'static> {
    let mut spans = vec![Span::styled("  │", Style::default().fg(Color::DarkGray))];
    for (index, width) in widths.iter().enumerate() {
        let cell = row.get(index).map(String::as_str).unwrap_or("");
        let style = if header {
            Style::default().fg(Color::Cyan).bold()
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(format!(" {cell:<width$} "), style));
        spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
    }
    Line::from(spans)
}

fn browser_table_separator(widths: &[usize]) -> String {
    let cells = widths
        .iter()
        .map(|width| "─".repeat(width + 2))
        .collect::<Vec<_>>()
        .join("┼");
    format!("  ├{cells}┤")
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

fn help_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Browser keys",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  ? / h", Style::default().fg(Color::Yellow)),
            Span::raw("      Toggle help overlay"),
        ]),
        Line::from(vec![
            Span::styled("  j / k / ↓ / ↑", Style::default().fg(Color::Yellow)),
            Span::raw("  Move selection"),
        ]),
        Line::from(vec![
            Span::styled(
                "  PgDn / PgUp / Space / b",
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("  Scroll page / back"),
        ]),
        Line::from(vec![
            Span::styled("  g / G", Style::default().fg(Color::Yellow)),
            Span::raw("      Jump to top / bottom"),
        ]),
        Line::from(vec![
            Span::styled("  Home / End", Style::default().fg(Color::Yellow)),
            Span::raw("  First item / Last item"),
        ]),
        Line::from(vec![
            Span::styled("  Enter", Style::default().fg(Color::Yellow)),
            Span::raw("      Open link / Press button / Edit input"),
        ]),
        Line::from(vec![
            Span::styled("  Tab / s / v", Style::default().fg(Color::Yellow)),
            Span::raw("      Toggle / Scroll / Selection mode"),
        ]),
        Line::from(vec![
            Span::styled("  b", Style::default().fg(Color::Yellow)),
            Span::raw("      Go back"),
        ]),
        Line::from(vec![
            Span::styled("  f", Style::default().fg(Color::Yellow)),
            Span::raw("      Go forward"),
        ]),
        Line::from(vec![
            Span::styled("  r", Style::default().fg(Color::Yellow)),
            Span::raw("      Reload page"),
        ]),
        Line::from(vec![
            Span::styled("  q / Esc", Style::default().fg(Color::Yellow)),
            Span::raw("      Quit"),
        ]),
    ]
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

        let loaded = load_network_page_with_keyring(
            &JaringanUrl::parse(&format!("jrg://{addr}/old.jrg")).unwrap(),
            &PublicKeyring::default(),
        )
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
        let loaded = load_network_page_with_keyring(
            &JaringanUrl::parse("jrg://127.0.0.1:1/missing.jrg").unwrap(),
            &PublicKeyring::default(),
        )
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

        let (final_url, response) = fetch_response_following_redirects_with_encryption(
            JaringanUrl::parse(&format!("jrg://{addr}/old.jrg")).unwrap(),
            None,
        )
        .unwrap();
        server.join().unwrap();

        assert_eq!(final_url.as_str(), format!("jrg://{addr}/new.jrg"));
        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.body, "# New Page\n");
    }

    #[test]
    fn help_overlay_lists_ergonomic_browser_shortcuts() {
        let rendered = help_lines()
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Browser keys"));
        assert!(rendered.contains("? / h"));
        assert!(rendered.contains("back"));
        assert!(rendered.contains("forward"));
        assert!(rendered.contains("PgDn / PgUp"));
        assert!(rendered.contains("g / G"));
    }

    #[test]
    fn browser_render_lines_include_polished_rich_blocks() {
        let document = parse_document(
            "# Layout\n\n> Calm focus.\n\n- fast\n- readable\n\n---\n\n| Element | Render |\n| --- | --- |\n| Table | Aligned |\n",
        )
        .unwrap();
        let page = LoadedPage {
            location: PageLocation::File(PathBuf::from("/tmp/layout.jrg")),
            items: collect_items(&document),
            document,
            signature_status: SignatureStatus::Unsigned,
        };

        let rendered = render_lines(&page, 0)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("┃ Calm focus."));
        assert!(rendered.contains("◆ fast"));
        assert!(rendered.contains("────────────────"));
        assert!(rendered.contains("Element"));
        assert!(rendered.contains("Aligned"));
    }

    #[test]
    fn confirmed_post_button_requires_second_enter_before_action_runs() {
        let root = std::env::temp_dir().join(format!(
            "jaringan-browser-action-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let document = parse_document(
            "# Tools\n\n? q label=\"Query\" value=\"laksa\"\n! search label=\"Search\" method=\"POST\" target=\"/actions/search\" confirm=\"Submit search?\" auth=\"demo-search\"\n",
        )
        .unwrap();
        let mut page = LoadedPage {
            location: PageLocation::File(root.join("tools.jrg")),
            items: collect_items(&document),
            document,
            signature_status: SignatureStatus::Unsigned,
        };
        let mut state = BrowserState::new(page.location.clone());
        state.selected = 1;

        activate_selected(&mut state, &mut page).unwrap();
        assert_eq!(state.status, "Submit search? Press Enter again to confirm.");
        assert_eq!(
            state
                .pending_confirmation
                .as_ref()
                .map(|action| action.id.as_str()),
            Some("search")
        );

        activate_selected(&mut state, &mut page).unwrap();
        assert_eq!(
            state.status,
            "Submitted local POST action: /actions/search?q=laksa"
        );
        assert_eq!(page.document.title(), Some("Search Results"));
        assert!(state.pending_confirmation.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_post_button_without_auth_token_renders_forbidden_and_skips_side_effect() {
        let root = std::env::temp_dir().join(format!(
            "jaringan-browser-action-auth-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let document = parse_document(
            "# Tools\n\n? q label=\"Query\" value=\"laksa\"\n! search label=\"Search\" method=\"POST\" target=\"/actions/search\"\n",
        )
        .unwrap();
        let mut page = LoadedPage {
            location: PageLocation::File(root.join("tools.jrg")),
            items: collect_items(&document),
            document,
            signature_status: SignatureStatus::Unsigned,
        };
        let mut state = BrowserState::new(page.location.clone());
        state.selected = 1;

        activate_selected(&mut state, &mut page).unwrap();

        assert_eq!(
            state.status,
            "Submitted local POST action: /actions/search?q=laksa"
        );
        assert!(matches!(
            page.document.blocks.first(),
            Some(Block::Preformatted(body))
                if body.contains("JRG/0.1 403 Forbidden")
                    && body.contains("missing or invalid action capability token")
        ));
        assert!(!root.join(".jrg-actions.log").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn edited_inputs_are_collected_into_action_payload() {
        let document = parse_document(
            "# Tools\n\n? q label=\"Query\" value=\"laksa\"\n! search label=\"Search\" method=\"POST\" target=\"/actions/search\"\n",
        )
        .unwrap();
        let mut page = LoadedPage {
            location: PageLocation::File(PathBuf::from("/tmp/tools.jrg")),
            items: collect_items(&document),
            document,
            signature_status: SignatureStatus::Unsigned,
        };
        let mut state = BrowserState::new(page.location.clone());

        for _ in 0..5 {
            edit_selected_input(&mut state, &mut page, InputEdit::Backspace);
        }
        for ch in "nasi goreng".chars() {
            edit_selected_input(&mut state, &mut page, InputEdit::Append(ch));
        }

        assert_eq!(input_payload(&page), "q=nasi%20goreng");
        assert_eq!(state.status, "Updated q = nasi goreng");
    }
    #[test]
    fn local_search_index_discovers_nested_jrg_pages() {
        let root =
            std::env::temp_dir().join(format!("jaringan-search-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("food")).unwrap();
        fs::write(
            root.join("index.jrg"),
            "# Home\n\n=> food/laksa.jrg Laksa guide\n\n~~~~~\ntitle: Food Home\ntags: index\n",
        )
        .unwrap();
        fs::write(
            root.join("food/laksa.jrg"),
            "# Laksa\n\n## Hawker stalls\n\n~~~~~\ntitle: Penang Laksa\ntags: laksa, hawker\n",
        )
        .unwrap();
        fs::write(root.join("ignore.txt"), "laksa but not a page").unwrap();

        let index = build_local_search_index(&root).unwrap();
        let results = index.search("hawker");

        assert_eq!(index.entries().len(), 2);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.url, "jrg://local/food/laksa.jrg");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persisted_search_index_can_be_loaded_for_queries() {
        let root = std::env::temp_dir().join(format!(
            "jaringan-persisted-search-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("index.jrg"),
            "# Home\n\nAn evening laksa notebook.\n\n~~~~~\ntitle: Food Home\n",
        )
        .unwrap();
        let index_path = root.join(".jrg-search-index");

        let index = build_local_search_index(&root).unwrap();
        save_search_index(&index, &index_path).unwrap();
        let loaded = load_search_index(&index_path).unwrap();

        assert_eq!(loaded.entries(), index.entries());
        assert_eq!(
            loaded.search("evening laksa")[0].entry.url,
            "jrg://local/index.jrg"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_get_search_action_replaces_page_with_selectable_results() {
        let root =
            std::env::temp_dir().join(format!("jaringan-tui-search-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("home.jrg"),
            "# Home\n\n? q label=\"Search\" value=\"laksa\"\n! find label=\"Find\" method=\"GET\" target=\"/search\"\n",
        )
        .unwrap();
        fs::write(root.join("food.jrg"), "# Food\n\nEvening laksa guide.\n").unwrap();
        let document = parse_document(&fs::read_to_string(root.join("home.jrg")).unwrap()).unwrap();
        let mut page = LoadedPage {
            location: PageLocation::File(root.join("home.jrg")),
            items: collect_items(&document),
            document,
            signature_status: SignatureStatus::Unsigned,
        };
        let mut state = BrowserState::new(page.location.clone());
        state.selected = 1;

        activate_selected(&mut state, &mut page).unwrap();

        assert_eq!(page.document.title(), Some("Search results for laksa"));
        assert_eq!(state.status, "Searched local index for: laksa");
        assert!(
            matches!(page.items.first(), Some(InteractiveItem::Link { target, .. }) if target == "food.jrg")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn security_label_marks_unsigned_and_signed_pages() {
        assert_eq!(
            security_label(&SignatureStatus::Unsigned),
            "not secure: unsigned"
        );
        assert_eq!(
            security_label(&SignatureStatus::Secure {
                signer: "alice".into()
            }),
            "secure: signed by alice"
        );
    }

    #[test]
    fn file_loader_verifies_signed_pages_against_loaded_keyring() {
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};
        use jaringan_core::canonical_signature_payload;

        let root = std::env::temp_dir().join(format!(
            "jaringan-keyring-loader-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let signing_key = SigningKey::from_bytes(&[11; 32]);
        let public_key = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes());
        let keyring_path = root.join("keyring");
        fs::write(&keyring_path, format!("alice ed25519:{public_key}\n")).unwrap();

        let unsigned = "# Signed local page\n\nTrusted from disk.\n\n~~~~~\nsigned-by: alice\n";
        let signature = signing_key.sign(canonical_signature_payload(unsigned).as_bytes());
        let source = format!(
            "{unsigned}signature: ed25519:{}\n",
            base64::engine::general_purpose::STANDARD.encode(signature.to_bytes())
        );
        let page_path = root.join("signed.jrg");
        fs::write(&page_path, source).unwrap();

        let keyring = load_keyring_file(&keyring_path).unwrap();
        let page = load_file_page_with_keyring(&page_path, &keyring).unwrap();

        assert_eq!(
            page.signature_status,
            SignatureStatus::Secure {
                signer: "alice".into()
            }
        );
        fs::remove_dir_all(root).unwrap();
    }
}

use std::{
    collections::HashMap,
    env, fs,
    io::{self, BufRead, Stdout},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, bail};
use base64::Engine;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use crossterm::event::{EnableMouseCapture, DisableMouseCapture};
use jaringan_browser::{
    ActionConfirmation, BrowserMode, BrowserState, DownloadPrompt, FindState, PageLocation, SavedTab, TextSelectPos,
    cache_filename_for_url, config::parse_color, go_back, go_forward, load_tabs, navigate_to,
    record_goto, resolve_target, save_tabs, scroll_down, scroll_page_down, scroll_page_up, scroll_to_bottom,
    scroll_up, selection_down, selection_first, selection_last, selection_up,
    switch_mode, toggle_mode, toggle_overlay, web_to_jrg_url,
};
use jaringan_core::{
    ActionMethod, Block, Document, Image, Input, PublicKeyring, SearchEntry, SearchIndex,
    SignatureStatus, Table, parse_document, verify_source_signature,
};
use jaringan_protocol::{
    ContentType, EncryptedTcpConfig, EncryptionKey, JaringanUrl, LocalFileResolver, PageResolver,
    Request, Response, ResponseTag, StatusCode, fetch_tcp, fetch_tcp_encrypted, fetch_tcp_stream,
    post_tcp, post_tcp_with_action_token, serve, serve_encrypted,
};
use jaringan_plugins::PluginRegistry;
use jaringan_plugins::plugin::PluginHook;
use jaringan_render::render_plain;
use jaringan_script::{BridgeState, ScriptInput, WasmRuntime, blocks_to_script_blocks, execute_document_scripts};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
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
    /// Inspect a jrg:// URL and print a JSON manifest of interactive elements.
    Inspect {
        /// The jrg:// URL to inspect.
        url: String,
    },
    /// Fill named input fields on a jrg:// page and print the updated state as JSON.
    Fill {
        /// The jrg:// URL to fill.
        url: String,
        /// Input field name=value pairs to set.
        #[arg(long, short)]
        field: Vec<String>,
    },
    /// Click a button by id or label on a jrg:// page and print the result page as JSON blocks.
    Click {
        /// The jrg:// URL to fetch and interact with.
        url: String,
        /// The button id or label to click.
        button: String,
        /// Input field name=value pairs to set before clicking.
        #[arg(long, short)]
        field: Vec<String>,
    },
    /// Start an interactive session, reading commands from stdin.
    Session {
        /// The jrg:// URL to start the session at.
        url: String,
    },
    /// Execute a WASM script from a jrg:// page and print the output as JSON.
    Run {
        /// The jrg:// URL to fetch and execute scripts from.
        url: String,
        /// Optional script label to run. If omitted, runs the first script block.
        #[arg(long, short)]
        script: Option<String>,
    },
    /// Serve a local document root over the TCP wire protocol.
    Serve {
        root: PathBuf,
        #[arg(long, default_value = "127.0.0.1:7070")]
        bind: String,
        /// Require encrypted TCP with this key id and JARINGAN_ENCRYPTION_KEY_HEX.
        #[arg(long)]
        encrypted_key_id: Option<String>,
        /// Advertise this ed25519 key id in every response (key must be in keyring file).
        #[arg(long)]
        advertise_key: Option<String>,
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
    /// Scaffold a new Jaringan site in the given directory.
    Init {
        /// Target directory for the new site.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// View a page (jrg://, http://, or file path) and render it to stdout.
    View {
        /// URL or file path to view.
        target: String,
        /// Show the raw JRG response headers before the rendered content.
        #[arg(long)]
        headers: bool,
    },
    /// Show the raw content/response for a URL (no rendering).
    Raw {
        /// URL or file path to dump raw.
        target: String,
    },
    /// Open a local path or jrg:// URL in the interactive terminal browser.
    Open {
        /// URLs or file paths to open (default: welcome page).
        #[arg(num_args = 0..)]
        targets: Vec<String>,
    },
    /// Run the JRG-HTTP two-way gateway.
    Gateway {
        #[command(subcommand)]
        command: GatewayCommand,
    },
    /// Manage authentication tokens for Jaringan services
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// List installed WASM plugins
    Plugins {
        /// Path to plugins directory (default: ~/.config/jaringan/plugins/)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Build WASM scripts for Jaringan pages.
    Script {
        #[command(subcommand)]
        command: ScriptCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ScriptCommand {
    /// Compile a Rust crate to WASM and optionally generate a .jrg template.
    Build {
        /// Path to the Rust crate root (containing Cargo.toml).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Output .wasm file path (default: <target-dir>/<crate-name>.wasm).
        #[arg(long)]
        output: Option<PathBuf>,
        /// Generate a .jrg page template with the WASM embedded as base64.
        #[arg(long)]
        template: Option<PathBuf>,
        /// Title for the generated .jrg template.
        #[arg(long, default_value = "Scripted Page")]
        title: String,
        /// Label for the script block in the template.
        #[arg(long, default_value = "Script")]
        label: String,
    },
}

#[derive(Debug, Subcommand)]
enum GatewayCommand {
    /// Run the HTTP→JRG gateway: accept HTTP, proxy to a JRG resolver
    ServeHttp {
        /// Address to listen on for HTTP requests
        #[arg(long, default_value = "127.0.0.1:8080")]
        http_listen: String,
        /// Target JRG host to proxy requests to
        #[arg(long, default_value = "127.0.0.1:7070")]
        jrg_host: String,
        /// Enable the /http/* bridge for fetching arbitrary HTTP URLs
        #[arg(long)]
        enable_http_bridge: bool,
        /// Request timeout in seconds
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Run the JRG→HTTP gateway: accept JRG TCP, proxy to HTTP servers
    JrgToHttp {
        /// Address to listen on for JRG TCP connections
        #[arg(long, default_value = "127.0.0.1:7071")]
        jrg_listen: String,
        /// User-Agent for HTTP requests
        #[arg(long, default_value = "Jaringan/0.1")]
        user_agent: String,
        /// Request timeout in seconds
        #[arg(long, default_value_t = 15)]
        timeout: u64,
        /// Maximum response body size in bytes
        #[arg(long, default_value_t = 1_048_576)]
        max_response_size: usize,
    },
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// Register or log in to a service via its register.jrg page
    Register {
        /// Service URL (e.g., api.site.tld or https://api.site.tld)
        service: String,
        /// Form field name=value pairs for headless registration
        #[arg(long, short)]
        field: Vec<String>,
    },
    /// Log in to a service (interactive or with existing credentials)
    Login {
        /// Service URL
        service: String,
    },
    /// List all stored tokens
    List,
    /// Remove a stored token
    Revoke {
        /// Service to revoke token for
        service: String,
    },
}

#[derive(Debug, Clone)]
struct LoadedPage {
    location: PageLocation,
    document: Document,
    items: Vec<InteractiveItem>,
    signature_status: SignatureStatus,
    stream_rx: Option<Arc<Mutex<mpsc::Receiver<String>>>>,
    raw_body: Option<String>,
    content_type: String,
    body_size: usize,
    load_time_ms: u64,
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

/// A single browser tab — page content, state, and file-watch mtime.
#[derive(Debug, Clone)]
struct Tab {
    page: LoadedPage,
    state: BrowserState,
    file_mtime: Option<SystemTime>,
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
        Command::Inspect { url } => {
            let parsed = JaringanUrl::parse(&url)?;
            let response = fetch_tcp(&parsed)
                .with_context(|| format!("failed to fetch {url}"))?;
            let doc = parse_document(&response.body)
                .with_context(|| format!("failed to parse document from {url}"))?;

            let inputs: Vec<_> = doc.blocks.iter().filter_map(|b| match b {
                Block::Input(input) => Some(serde_json::json!({
                    "name": input.name,
                    "label": input.label,
                    "value": input.value,
                    "placeholder": input.placeholder,
                })),
                _ => None,
            }).collect();

            let buttons: Vec<_> = doc.blocks.iter().filter_map(|b| match b {
                Block::Button(btn) => Some(serde_json::json!({
                    "id": btn.id,
                    "label": btn.label,
                    "method": format!("{:?}", btn.method),
                    "target": btn.target,
                    "confirm": btn.confirm,
                })),
                _ => None,
            }).collect();

            let scripts: Vec<_> = doc.blocks.iter().filter_map(|b| match b {
                Block::Script { label, .. } => Some(serde_json::json!({
                    "label": label,
                })),
                _ => None,
            }).collect();

            let links: Vec<_> = doc.blocks.iter().enumerate().filter_map(|(i, b)| match b {
                Block::Link(link) => Some(serde_json::json!({
                    "index": i,
                    "label": link.label,
                    "target": link.target,
                })),
                _ => None,
            }).collect();

            let block_types: Vec<_> = doc.blocks.iter().map(|b| {
                let type_name = match b {
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
                    Block::Auth { .. } => "auth",
                };
                let text = match b {
                    Block::Paragraph(s) => Some(s.clone()),
                    Block::Quote(s) => Some(s.clone()),
                    Block::Heading { text, .. } => Some(text.clone()),
                    Block::Preformatted { code, .. } => Some(code.clone()),
                    _ => None,
                };
                serde_json::json!({
                    "type": type_name,
                    "text": text,
                })
            }).collect();

            let manifest = serde_json::json!({
                "url": url,
                "title": doc.title(),
                "inputs": inputs,
                "buttons": buttons,
                "scripts": scripts,
                "links": links,
                "blocks": block_types,
            });

            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
        Command::Fill { url, field } => {
            let parsed = JaringanUrl::parse(&url)?;
            let response = fetch_tcp(&parsed)
                .with_context(|| format!("failed to fetch {url}"))?;
            let doc = parse_document(&response.body)
                .with_context(|| format!("failed to parse document from {url}"))?;

            let mut fields = HashMap::new();
            for f in &field {
                if let Some((name, value)) = f.split_once('=') {
                    fields.insert(name.to_string(), value.to_string());
                }
            }

            let block_types: Vec<_> = doc.blocks.iter().map(|b| {
                let type_name = match b {
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
                    Block::Auth { .. } => "auth",
                };
                let text = match b {
                    Block::Paragraph(s) => Some(s.clone()),
                    Block::Quote(s) => Some(s.clone()),
                    Block::Heading { text, .. } => Some(text.clone()),
                    Block::Preformatted { code, .. } => Some(code.clone()),
                    _ => None,
                };
                serde_json::json!({
                    "type": type_name,
                    "text": text,
                })
            }).collect();

            let mut updated_blocks = doc.blocks.clone();
            for block in &mut updated_blocks {
                if let Block::Input(input) = block
                    && let Some(value) = fields.get(&input.name) {
                        input.value = value.clone();
                    }
            }

            let inputs: Vec<_> = updated_blocks.iter().filter_map(|b| match b {
                Block::Input(input) => Some(serde_json::json!({
                    "name": input.name,
                    "label": input.label,
                    "value": input.value,
                    "placeholder": input.placeholder,
                })),
                _ => None,
            }).collect();

            let manifest = serde_json::json!({
                "url": url,
                "inputs": inputs,
                "blocks": block_types,
            });

            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
        Command::Click { url, button, field } => {
            let parsed = JaringanUrl::parse(&url)?;
            let response = fetch_tcp(&parsed)
                .with_context(|| format!("failed to fetch {url}"))?;
            let doc = parse_document(&response.body)
                .with_context(|| format!("failed to parse document from {url}"))?;

            // Parse -f name=value pairs
            let mut fields = HashMap::new();
            for f in &field {
                if let Some((name, value)) = f.split_once('=') {
                    fields.insert(name.to_string(), value.to_string());
                }
            }

            // Find the first Button block whose id or label matches
            let found = doc.blocks.iter().find_map(|b| {
                if let Block::Button(btn) = b
                    && (btn.id == button || btn.label == button) {
                        return Some(btn);
                    }
                None
            });

            let found = match found {
                Some(btn) => btn,
                None => {
                    let err = serde_json::json!({"error": "button not found"});
                    println!("{}", serde_json::to_string_pretty(&err)?);
                    return Ok(());
                }
            };

            // Apply field overrides to document inputs
            let mut doc_blocks = doc.blocks.clone();
            for block in &mut doc_blocks {
                if let Block::Input(input) = block
                    && let Some(value) = fields.get(&input.name) {
                        input.value = value.clone();
                    }
            }

            // Resolve target (relative or absolute)
            let target_url = JaringanUrl::parse(&found.target).unwrap_or_else(|_| {
                parsed.resolve(&found.target)
                    .expect("failed to resolve relative button target")
            });

            // Execute the button action
            let response = match found.method {
                ActionMethod::Get => {
                    fetch_tcp(&target_url)
                        .with_context(|| format!("failed to fetch button target {}", target_url))?
                }
                ActionMethod::Post => {
                    // Build POST body: input values + button id
                    let mut body = input_payload_from_blocks(&doc_blocks);
                    if !body.is_empty() {
                        body.push('&');
                    }
                    body.push_str(&format!("button={}", percent_encode(&found.id)));
                    if let Some(ref auth) = found.auth {
                        // Look up stored token by service name; button works without one
                        match lookup_stored_token(auth) {
                            Some(token) => post_tcp_with_action_token(&target_url, body, &token)
                                .with_context(|| format!("failed to POST button action to {}", target_url))?,
                            None => post_tcp(&target_url, body)
                                .with_context(|| format!("failed to POST button action to {}", target_url))?,
                        }
                    } else {
                        post_tcp(&target_url, body)
                            .with_context(|| format!("failed to POST button action to {}", target_url))?
                    }
                }
            };

            let result_doc = parse_document(&response.body)
                .with_context(|| format!("failed to parse response from {}", target_url))?;

            let blocks: Vec<_> = result_doc.blocks.iter().map(|b| {
                let type_name = match b {
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
                    Block::Auth { .. } => "auth",
                };
                let text = match b {
                    Block::Paragraph(s) => Some(s.clone()),
                    Block::Quote(s) => Some(s.clone()),
                    Block::Heading { text, .. } => Some(text.clone()),
                    Block::Preformatted { code, .. } => Some(code.clone()),
                    _ => None,
                };
                serde_json::json!({
                    "type": type_name,
                    "text": text,
                })
            }).collect();

            // Build inputs output (matching Fill behavior)
            let inputs: Vec<_> = doc_blocks.iter().filter_map(|b| match b {
                Block::Input(input) => Some(serde_json::json!({
                    "name": input.name,
                    "label": input.label,
                    "value": input.value,
                    "placeholder": input.placeholder,
                })),
                _ => None,
            }).collect();

            let manifest = serde_json::json!({
                "url": format!("{}", target_url),
                "title": result_doc.title(),
                "button": button,
                "inputs": inputs,
                "blocks": blocks,
            });

            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
        Command::Session { url } => {
            let mut session = jaringan_browser::session::SessionState::start(&url)
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            // Print initial state as JSON
            let initial = serde_json::json!({
                "url": url,
                "status": "session_started",
                "blocks": session.document().blocks.iter().map(block_summary_json).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&initial)?);

            // Read commands from stdin
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                let line = line?;
                let line = line.trim().to_string();
                if line.is_empty() || line == "exit" || line == "quit" {
                    break;
                }

                // Parse command: "inspect", "fill name=value", "refresh"
                let response = match line.split_once(' ') {
                    Some(("fill", rest)) => {
                        // Fill inputs
                        let mut updated = session.blocks.clone();
                        if let Some((name, value)) = rest.split_once('=') {
                            for block in &mut updated {
                                if let Block::Input(input) = block
                                    && input.name == name.trim() {
                                        input.value = value.trim().to_string();
                                    }
                            }
                        }
                        session.blocks = updated;
                        serde_json::json!({"status": "filled", "blocks": session.document().blocks.iter().map(block_summary_json).collect::<Vec<_>>()})
                    }
                    Some(("navigate", target)) => {
                        // Navigate to a new URL (relative or absolute)
                        let new_url = if target.starts_with("jrg://") {
                            target.to_string()
                        } else {
                            // relative — just join
                            let base = url.trim_end_matches('/');
                            format!("{}/{}", base, target.trim_start_matches('/'))
                        };
                        match jaringan_browser::session::SessionState::start(&new_url) {
                            Ok(new_session) => {
                                session = new_session;
                                serde_json::json!({"status": "navigated", "url": session.url, "blocks": session.document().blocks.iter().map(block_summary_json).collect::<Vec<_>>()})
                            }
                            Err(e) => serde_json::json!({"status": "error", "error": e}),
                        }
                    }
                    _ if line == "refresh" => {
                        match session.refresh() {
                            Ok(()) => serde_json::json!({"status": "refreshed", "blocks": session.document().blocks.iter().map(block_summary_json).collect::<Vec<_>>()}),
                            Err(e) => serde_json::json!({"status": "error", "error": e}),
                        }
                    }
                    _ if line == "inspect" => {
                        let doc = session.document();
                        serde_json::json!({
                            "inputs": doc.blocks.iter().filter_map(|b| {
                                if let Block::Input(i) = b { Some(serde_json::json!({"name": i.name, "label": i.label, "value": i.value, "placeholder": i.placeholder})) } else { None }
                            }).collect::<Vec<_>>(),
                            "buttons": doc.blocks.iter().filter_map(|b| {
                                if let Block::Button(b2) = b { Some(serde_json::json!({"id": b2.id, "label": b2.label, "method": b2.method.as_str(), "target": b2.target})) } else { None }
                            }).collect::<Vec<_>>(),
                            "scripts": doc.blocks.iter().filter_map(|b| {
                                if let Block::Script { label, .. } = b { Some(serde_json::json!({"label": label})) } else { None }
                            }).collect::<Vec<_>>(),
                        })
                    }
                    _ => serde_json::json!({"status": "error", "error": format!("unknown command: {}", line)}),
                };

                println!("{}", serde_json::to_string_pretty(&response)?);
            }
        }
        Command::Run { url, script } => {
            let parsed = JaringanUrl::parse(&url)?;
            let response = fetch_tcp(&parsed)
                .with_context(|| format!("failed to fetch {url}"))?;
            let doc = parse_document(&response.body)
                .with_context(|| format!("failed to parse document from {url}"))?;

            // Find the matching Script block
            let (wasm, label) = match script {
                Some(ref label) => {
                    let found = doc.blocks.iter().find_map(|b| {
                        if let Block::Script { wasm, label: lbl } = b
                            && lbl.as_deref() == Some(label.as_str()) {
                                return Some((wasm.clone(), lbl.clone()));
                            }
                        None
                    });
                    match found {
                        Some(pair) => pair,
                        None => {
                            let err = serde_json::json!({"error": format!("script label '{}' not found", label)});
                            println!("{}", serde_json::to_string_pretty(&err)?);
                            return Ok(());
                        }
                    }
                }
                None => {
                    let found = doc.blocks.iter().find_map(|b| {
                        if let Block::Script { wasm, label } = b {
                            Some((wasm.clone(), label.clone()))
                        } else {
                            None
                        }
                    });
                    match found {
                        Some(pair) => pair,
                        None => {
                            let err = serde_json::json!({"error": "no script blocks found in document"});
                            println!("{}", serde_json::to_string_pretty(&err)?);
                            return Ok(());
                        }
                    }
                }
            };

            // Build the script input from the document's blocks
            let input = ScriptInput {
                title: doc.title().map(|s| s.to_string()),
                inputs: Vec::new(),
                page_metadata: None,
                blocks: blocks_to_script_blocks(&doc.blocks),
                tui: None,
                form_fields: None,
            };

            // Create runtime and execute
            let runtime = WasmRuntime::new()
                .with_context(|| "failed to create WASM runtime")?;

            let bridge = BridgeState::empty();
            let output = runtime.execute_with_bridge(&wasm, &input, bridge)
                .with_context(|| "script execution failed")?;

            // Build JSON output
            let result = serde_json::json!({
                "url": url,
                "script_label": label,
                "output_blocks": output.blocks,
            });

            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Serve {
            root,
            bind,
            encrypted_key_id,
            advertise_key,
        } => {
            let bind = ensure_port(&bind);
            let v4_listener = TcpListener::bind(&bind)
                .with_context(|| format!("failed to bind Jaringan server to {bind}"))?;

            // Build the resolver (shared across IPv4 and IPv6 listeners)
            let resolver: Arc<dyn PageResolver> = if let Some(key_id) = encrypted_key_id {
                let config = encrypted_tcp_config_from_env(&key_id)?;
                eprintln!("serving encrypted {} at jrg://{bind}/", root.display());
                let r = LocalFileResolver::new(root);
                serve_encrypted(v4_listener, Arc::new(r), &config)?;
                return Ok(());
            } else if let Some(key_id) = advertise_key {
                let key_base64 = lookup_keyring_key(&key_id)?;
                let r = LocalFileResolver::new_with_key(root.clone(), key_id, key_base64);
                eprintln!("serving {} at jrg://{bind}/ (advertising key)", root.display());
                Arc::new(r) as Arc<dyn PageResolver>
            } else {
                eprintln!("serving {} at jrg://{bind}/", root.display());
                Arc::new(LocalFileResolver::new(root)) as Arc<dyn PageResolver>
            };

            // Try IPv6 localhost (dual-stack) so `localhost` -> `::1` resolves correctly
            let v4_port = v4_listener.local_addr()?.port();
            let v6_bind = format!("[::1]:{v4_port}");
            let v6_handle = match TcpListener::bind(&v6_bind) {
                Ok(v6_listener) => {
                    eprintln!("  (also listening on {v6_bind})");
                    let v6_resolver = resolver.clone();
                    Some(std::thread::spawn(move || {
                        let _ = serve(v6_listener, v6_resolver);
                    }))
                }
                Err(e) => {
                    eprintln!("  (IPv6 localhost not available: {e})");
                    None
                }
            };

            serve(v4_listener, resolver)?;

            // Wait for IPv6 thread if it was spawned (serve only returns on error)
            if let Some(handle) = v6_handle {
                let _ = handle.join();
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
        Command::Init { path } => init_jrg_site(&path)?,
        Command::View { target, headers } => {
            let loc = parse_start_location(&target)?;
            let page = load_location(&loc, &None, &None)?;

            if headers {
                println!("Location: {}", loc.display_url());
                println!("Title: {}\n", page.document.title().unwrap_or("Untitled"));
            }

            let text = jaringan_render::render_plain(&page.document);
            print!("{text}");

            // If streaming, grab the first block too
            if let Some(ref rx) = page.stream_rx {
                let rx = rx.lock().unwrap();
                if let Ok(block) = rx.try_recv() {
                    let doc = parse_document(&block)
                        .unwrap_or_else(|_| Document::new(vec![Block::Preformatted { code: block, language: None }]));
                    println!("\n{}", render_plain(&doc));
                }
            }
        }
        Command::Raw { target } => {
            let loc = parse_start_location(&target)?;
            match loc {
                PageLocation::File(path) => {
                    let content = fs::read_to_string(&path)
                        .with_context(|| format!("failed to read {}", path.display()))?;
                    print!("{content}");
                }
                PageLocation::Network(ref url) => {
                    let response = fetch_tcp(url)?;
                    print_response(response);
                }
                PageLocation::Web(ref url) => {
                    let resolver = jaringan_gateway::JrgToHttpResolver::new(
                        jaringan_gateway::JrgToHttpResolverConfig::default(),
                    );
                    let jrg_url = web_to_jrg_url(url);
                    let parsed = JaringanUrl::parse(&jrg_url)?;
                    let request = Request::new(parsed);
                    let response = resolver.fetch(&request).map_err(|e| {
                        anyhow::anyhow!("failed to fetch web page {url}: {e}")
                    })?;
                    print_response(response);
                }
                PageLocation::Unsupported(target) => bail!("unsupported target: {target}"),
            }
        }
        Command::Open { targets } => {
            let targets = if targets.is_empty() {
                // Fall back to config default_target if nothing provided
                let config_default = jaringan_browser::config::load()
                    .ok()
                    .flatten()
                    .and_then(|c| c.default_target);
                config_default.map(|t| vec![t]).unwrap_or_default()
            } else {
                targets
            };
            run_tui(targets)?
        }
        Command::Plugins { dir } => {
            let dir = dir.unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config/jaringan/plugins")
            });
            let mut registry = PluginRegistry::new(&dir)
                .map_err(|e| anyhow::anyhow!("failed to create plugin registry: {e}"))?;
            registry.load_all()
                .map_err(|e| anyhow::anyhow!("failed to load plugins: {e}"))?;
            let plugins = registry.plugins();
            if plugins.is_empty() {
                println!("No WASM plugins found in {}", dir.display());
            } else {
                for plugin in plugins {
                    println!("{} v{}", plugin.registration.name, plugin.registration.version);
                    if let Some(ref desc) = plugin.registration.description {
                        println!("  {desc}");
                    }
                    println!(
                        "  Hooks: {}",
                        plugin.registration.hooks.iter().map(|h| format!("{h:?}")).collect::<Vec<_>>().join(", ")
                    );
                    println!("  Path: {}", plugin.path.display());
                    println!();
                }
            }
        }
        Command::Script { command: ScriptCommand::Build { path, output, template, title, label } } => {
            cmd_script_build(&path, output.as_deref(), template.as_deref(), &title, &label)?;
        }
        Command::Gateway { command } => match command {
            GatewayCommand::ServeHttp {
                http_listen,
                jrg_host,
                enable_http_bridge,
                timeout,
            } => {
                let http_listen = ensure_port(&http_listen);
                let jrg_host = ensure_port(&jrg_host);
                let rt = tokio::runtime::Runtime::new()
                    .context("failed to create tokio runtime")?;
                rt.block_on(async {
                    let config = jaringan_gateway::HttpToJrgGatewayConfig {
                        listen_addr: http_listen,
                        jrg_host,
                        enable_http_bridge,
                        timeout_secs: timeout,
                        ..Default::default()
                    };
                    let gateway = jaringan_gateway::HttpToJrgGateway::new(config);
                    eprintln!("Starting HTTP→JRG gateway...");
                    if let Err(e) = gateway.serve().await {
                        eprintln!("Gateway error: {e}");
                        std::process::exit(1);
                    }
                });
            }
            GatewayCommand::JrgToHttp {
                jrg_listen,
                user_agent,
                timeout,
                max_response_size,
            } => {
                let jrg_listen = ensure_port(&jrg_listen);
                let resolver = jaringan_gateway::JrgToHttpResolver::new(
                    jaringan_gateway::JrgToHttpResolverConfig {
                        user_agent,
                        timeout_secs: timeout,
                        max_response_size,
                        ..Default::default()
                    },
                );
                let listener = std::net::TcpListener::bind(&jrg_listen)
                    .with_context(|| format!("failed to bind {jrg_listen}"))?;
                eprintln!("JRG→HTTP gateway listening on tcp://{jrg_listen}");
                eprintln!(
                    "  Usage: jrg://http/<domain>/<path> for HTTP, jrg://https.<domain>/<path> for HTTPS"
                );
                serve(listener, resolver)?;
            }
        },
        Command::Auth { command } => match command {
            AuthCommand::List => auth_list()?,
            AuthCommand::Revoke { service } => auth_revoke(&service)?,
            AuthCommand::Register { service, field } => {
                auth_register_or_login(&service, "register.jrg", &field)?
            }
            AuthCommand::Login { service } => {
                auth_register_or_login(&service, "login.jrg", &[])?
            }
        },
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Auth helpers — token storage and register/login via HTTP
// ---------------------------------------------------------------------------

/// Get the tokens storage directory (~/.config/jaringan/tokens/).
fn tokens_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/jaringan/tokens")
}

/// Store a token for a service on disk.
fn store_token(
    service: &str,
    token: &str,
    scope: Option<&str>,
    expires_at: Option<&str>,
) -> io::Result<()> {
    let dir = tokens_dir().join(service);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("token"), token)?;
    if let Some(s) = scope {
        fs::write(dir.join("scope"), s)?;
    }
    if let Some(e) = expires_at {
        fs::write(dir.join("expires_at"), e)?;
    }
    Ok(())
}

/// List all stored tokens.
fn auth_list() -> anyhow::Result<()> {
    let dir = tokens_dir();
    if !dir.exists() {
        println!("No tokens stored.");
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in &entries {
        let service = entry.file_name().to_string_lossy().to_string();
        let token_path = entry.path().join("token");
        let scope_path = entry.path().join("scope");
        let expires_path = entry.path().join("expires_at");
        let token = fs::read_to_string(&token_path).unwrap_or_default();
        let scope = fs::read_to_string(&scope_path).unwrap_or_default();
        let expires = fs::read_to_string(&expires_path).unwrap_or_default();
        println!(
            "  {} ({} chars, scope: {}, expires: {})",
            service,
            token.len(),
            scope.trim(),
            expires.trim()
        );
    }
    Ok(())
}

/// Look up a stored token by service name using prefix matching.
/// E.g., auth_service="microblog" matches stored dir "microblog.localhost:7072".
fn lookup_stored_token(auth_service: &str) -> Option<String> {
    let dir = tokens_dir();
    if !dir.exists() {
        return None;
    }
    // First try exact match
    let exact = dir.join(auth_service);
    if exact.join("token").exists() {
        return fs::read_to_string(exact.join("token")).ok();
    }
    // Then try prefix match: dir name starts with "auth_service." or "auth_service:"
    let prefix1 = format!("{}.", auth_service);
    let prefix2 = format!("{}:", auth_service);
    for entry in fs::read_dir(&dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name == auth_service || name.starts_with(&prefix1) || name.starts_with(&prefix2) {
            let token_path = entry.path().join("token");
            if token_path.exists() {
                return fs::read_to_string(token_path).ok();
            }
        }
    }
    None
}

/// Revoke (remove) a stored token for a service.
fn auth_revoke(service: &str) -> anyhow::Result<()> {
    let token_dir = tokens_dir().join(service);
    if token_dir.exists() {
        fs::remove_dir_all(&token_dir)?;
        println!("Token for '{}' revoked.", service);
    } else {
        println!("No token stored for '{}'.", service);
    }
    Ok(())
}

/// Register or log in to a service by fetching its register.jrg or login.jrg
/// page over the JRG TCP protocol, optionally filling form fields and submitting.
fn auth_register_or_login(
    service: &str,
    page: &str,
    fields: &[String],
) -> anyhow::Result<()> {
    // Determine if this looks like a JRG host:port or needs HTTP fallback
    let use_http = service.starts_with("http://") || service.starts_with("https://");

    if !use_http {
        // ——— JRG TCP path ———
        // Parse host and optional port from the service string
        let parts: Vec<&str> = service.split(':').collect();
        let host = if parts[0] == "localhost" { "127.0.0.1" } else { parts[0] };
        let port: u16 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(7070);

        let page_url_str = format!("jrg://{host}:{port}/{page}");
        let page_url = JaringanUrl::parse(&page_url_str)
            .with_context(|| format!("invalid JRG URL: {page_url_str}"))?;

        // Fetch the register/login page
        let resp = fetch_tcp(&page_url)
            .with_context(|| format!("failed to fetch {page_url_str} — is the service running on JRG TCP port {port}?"))?;

        if resp.status != StatusCode::Ok {
            bail!("JRG {} when fetching {page_url_str}", resp.status.as_u16());
        }

        // Parse as JRG document to extract form inputs
        let doc = parse_document(&resp.body)
            .with_context(|| format!("failed to parse JRG document from {page_url_str}"))?;

        let inputs: Vec<_> = doc
            .blocks
            .iter()
            .filter_map(|b| {
                if let Block::Input(input) = b {
                    Some(input.clone())
                } else {
                    None
                }
            })
            .collect();

        if fields.is_empty() {
            // Print form fields and exit
            let page_label = if page == "register.jrg" { "/register.jrg" } else { "/login.jrg" };
            println!("Form fields on {host}:{port}{page_label}:");
            if inputs.is_empty() {
                println!("  (no input fields found — try with -f name=value pairs)");
            } else {
                for input in &inputs {
                    println!(
                        "  - {} (name={}{})",
                        input.label,
                        input.name,
                        input.placeholder.as_ref()
                            .map(|p| format!(", placeholder={p}"))
                            .unwrap_or_default()
                    );
                }
            }
            println!();
            println!("Use -f name=value to fill fields and submit, e.g.:");
            if let Some(first) = inputs.first() {
                println!("  jaringan auth register {service} -f {}='<value>'", first.name);
            }
            return Ok(());
        }

        // Build form body from fields
        let body = fields.iter()
            .filter_map(|f| {
                let (name, value) = f.split_once('=')?;
                Some(format!("{}={}", name, value))
            })
            .collect::<Vec<_>>()
            .join("&");

        // POST the form data via JRG TCP with action token from the button
        let action_url_str = format!("jrg://{host}:{port}/actions/{page_type}",
            page_type = page.strip_suffix(".jrg").unwrap_or(page));
        let action_url = JaringanUrl::parse(&action_url_str)
            .with_context(|| format!("invalid JRG URL: {action_url_str}"))?;

        // Find the button's auth token from stored tokens, using the button's auth attribute as service name
        let auth_service: Option<String> = doc.blocks.iter().find_map(|b| {
            if let Block::Button(btn) = b {
                btn.auth.clone()
            } else {
                None
            }
        });
        let auth_token: Option<String> = auth_service.as_ref()
            .and_then(|s| lookup_stored_token(s));

        let post_resp = match &auth_token {
            Some(token) => post_tcp_with_action_token(&action_url, body.clone(), token)
                .with_context(|| format!("failed to POST to {action_url_str}"))?,
            None => post_tcp(&action_url, body.clone())
                .with_context(|| format!("failed to POST to {action_url_str}"))?,
        };

        // Check response tags for Token
        for tag in &post_resp.tags {
            if let ResponseTag::Token {
                service: tok_service,
                value: tok_value,
                expires_at: tok_expires,
            } = tag
            {
                let service_name = if tok_service.is_empty() {
                    format!("{host}:{port}")
                } else {
                    tok_service.clone()
                };
                store_token(
                    &service_name,
                    tok_value,
                    None,
                    tok_expires.as_deref(),
                )?;
                println!("✅ Token stored for '{}' ({} chars).", service_name, tok_value.len());
                return Ok(());
            }
        }

        // No token tag — print the response body
        println!("Response from {action_url_str}:");
        let response_doc = parse_document(&post_resp.body)
            .unwrap_or(Document::new(vec![]));
        println!("{}", render_plain(&response_doc));
    } else {
        // ——— HTTP fallback path (existing behavior) ———
        let http_base = service.trim_end_matches('/').to_string();
        let url = format!("{}/{}", http_base, page);
        eprintln!("Fetching {url} ...");

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .context("failed to build HTTP client")?;

        let resp = client
            .get(&url)
            .send()
            .with_context(|| format!("failed to fetch {url}"))?;

        let status = resp.status();
        let body = resp.text().with_context(|| format!("failed to read response body from {url}"))?;

        if !status.is_success() {
            bail!("HTTP {status} when fetching {url}");
        }

        let doc = parse_document(&body)
            .with_context(|| format!("failed to parse JRG document from {url}"))?;

        let inputs: Vec<_> = doc
            .blocks
            .iter()
            .filter_map(|b| {
                if let Block::Input(input) = b {
                    Some(input.clone())
                } else {
                    None
                }
            })
            .collect();

        if fields.is_empty() {
            println!("Form fields for {url}:");
            if inputs.is_empty() {
                println!("  (no input fields found — try with -f name=value pairs)");
            } else {
                for input in &inputs {
                    println!(
                        "  - {} (name={}{})",
                        input.label,
                        input.name,
                        input.placeholder.as_ref()
                            .map(|p| format!(", placeholder={p}"))
                            .unwrap_or_default()
                    );
                }
            }
            println!();
            println!("Use -f name=value to fill fields and submit, e.g.:");
            if let Some(first) = inputs.first() {
                println!("  jaringan auth register {service} -f {}='<value>'", first.name);
            }
            return Ok(());
        }

        let mut form_data = HashMap::new();
        for f in fields {
            if let Some((name, value)) = f.split_once('=') {
                form_data.insert(name.to_string(), value.to_string());
            } else {
                eprintln!("Warning: skipping malformed field '{f}' (expected name=value)");
            }
        }

        let post_resp = client
            .post(&url)
            .form(&form_data)
            .send()
            .with_context(|| format!("failed to POST to {url}"))?;

        let post_status = post_resp.status();
        let post_body = post_resp.text().with_context(|| format!("failed to read POST response from {url}"))?;

        if !post_status.is_success() {
            bail!("HTTP {post_status} when submitting form to {url}");
        }

        // Try JrgToHttpResolver to get token tags
        let resolver = jaringan_gateway::JrgToHttpResolver::new(
            jaringan_gateway::JrgToHttpResolverConfig::default(),
        );

        let jrg_url_str = format!("jrg://https.{}/{}", http_base
            .trim_start_matches("https://")
            .trim_start_matches("http://"), page);
        let jrg_url = JaringanUrl::parse(&jrg_url_str)
            .context("failed to parse JRG URL for token check")?;

        let jrg_req = Request::new(jrg_url);
        match resolver.fetch(&jrg_req) {
            Ok(jrg_resp) => {
                for tag in &jrg_resp.tags {
                    if let ResponseTag::Token {
                        service: tok_service,
                        value: tok_value,
                        expires_at: tok_expires,
                    } = tag
                    {
                        let service_name = if tok_service.is_empty() {
                            http_base
                                .trim_start_matches("https://")
                                .trim_start_matches("http://")
                                .to_string()
                        } else {
                            tok_service.clone()
                        };
                        store_token(
                            &service_name,
                            tok_value,
                            None,
                            tok_expires.as_deref(),
                        )?;
                        println!("Token stored for '{}' ({} chars).", service_name, tok_value.len());
                        return Ok(());
                    }
                }
                let post_doc = parse_document(&post_body)
                    .with_context(|| format!("failed to parse POST response from {url}"))?;
                println!("Response from {url}:");
                println!("{}", render_plain(&post_doc));
            }
            Err(e) => {
                eprintln!("Note: could not check for token tags via gateway ({e})");
                let post_doc = parse_document(&post_body)
                    .with_context(|| format!("failed to parse POST response from {url}"))?;
                println!("Response from {url}:");
                println!("{}", render_plain(&post_doc));
            }
        }
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
            ResponseTag::Stream => println!("Tag-Stream: true"),
            ResponseTag::Key { key_id, key_base64 } => {
                println!("Tag-Key: {key_id} ed25519:{key_base64}")
            }
            ResponseTag::ContentType { value } => println!("Tag-ContentType: {value}"),
            ResponseTag::Token {
                service,
                value,
                expires_at,
            } => println!(
                "Tag-Token: service={service} value={value}{}",
                expires_at
                    .map(|e| format!(" expires_at={e}"))
                    .unwrap_or_default()
            ),
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

/// Produce a JSON summary of a single block for the session subcommand output.
fn block_summary_json(block: &Block) -> serde_json::Value {
    let type_name = match block {
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
        Block::Auth { .. } => "auth",
    };
    let text = match block {
        Block::Paragraph(s) => Some(s.clone()),
        Block::Quote(s) => Some(s.clone()),
        Block::Heading { text, .. } => Some(text.clone()),
        Block::Preformatted { code, .. } => Some(code.clone()),
        _ => None,
    };
    serde_json::json!({
        "type": type_name,
        "text": text,
    })
}

fn run_tui(targets: Vec<String>) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, targets);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    targets: Vec<String>,
) -> anyhow::Result<()> {
    // Load config
    let cfg = jaringan_browser::config::load()
        .ok()
        .flatten()
        .unwrap_or_default();

    // Apply data_dir override from config
    if let Some(ref data_dir) = cfg.data_dir {
        // SAFETY: single-threaded startup, no other code reads JARINGAN_DATA yet
        unsafe { std::env::set_var("JARINGAN_DATA", data_dir); }
    }

    // Create a WASM runtime for page-level scripts
    let script_runtime = WasmRuntime::new().ok();

    // Create a bridge that lets WASM scripts call host functions
    let bridge_timeout = cfg.gateway.timeout_secs;
    let bridge: Option<BridgeState> = Some(BridgeState {
        fetch_fn: Some(std::sync::Arc::new(move |url: &str, _token: Option<&str>| {
            (|| -> Result<String, String> {
                let result = if url.starts_with("http://") || url.starts_with("https://") {
                    // Web URL: use JrgToHttpResolver gateway (with user's timeout)
                    let resolver = jaringan_gateway::JrgToHttpResolver::new(
                        jaringan_gateway::JrgToHttpResolverConfig {
                            timeout_secs: bridge_timeout,
                            ..Default::default()
                        },
                    );
                    let jrg_url = web_to_jrg_url(url);
                    let parsed = JaringanUrl::parse(&jrg_url)
                        .map_err(|e| format!("bridge: bad JRG URL '{jrg_url}': {e}"))?;
                    let resp = resolver
                        .fetch(&Request::new(parsed))
                        .map_err(|e| format!("bridge: gateway fetch failed: {e}"))?;
                    (resp.status.as_u16(), resp.content_type.as_str().to_string(), resp.body)
                } else if url.starts_with("jrg://") {
                    // JRG URL: use TCP protocol directly
                    let parsed = JaringanUrl::parse(url)
                        .map_err(|e| format!("bridge: bad JRG URL '{url}': {e}"))?;
                    let resp = fetch_tcp(&parsed)
                        .map_err(|e| format!("bridge: JRG fetch failed: {e}"))?;
                    (resp.status.as_u16(), resp.content_type.as_str().to_string(), resp.body)
                } else {
                    // Local file path
                    match std::fs::read_to_string(url) {
                        Ok(body) => (200, "text/plain; charset=utf-8".into(), body),
                        Err(e) => return Err(format!("bridge: failed to read '{url}': {e}")),
                    }
                };
                let (status, content_type, body) = result;
                Ok(serde_json::json!({
                    "status": status,
                    "content_type": content_type,
                    "body": body,
                    "error": null
                }).to_string())
            })()
        })),
        navigate_fn: Some(std::sync::Arc::new(|url: &str| {
            eprintln!("[jaringan] bridge navigate: {url}");
            Ok(r#"{}"#.into())
        })),
        log_fn: Some(std::sync::Arc::new(|level: &str, msg: &str| {
            eprintln!("[jaringan:bridge/{level}] {msg}");
        })),
        store: None,
        session_store: None,
        resolve_fn: None,
        page_inputs: None,
        tokens: None,
    });

    // Initialize plugin registry from ~/.config/jaringan/plugins/
    let plugins_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/jaringan/plugins");
    let mut plugin_registry = match PluginRegistry::new(&plugins_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[jaringan] warning: plugin system init: {e}");
            PluginRegistry::empty()
        }
    };
    if let Err(e) = plugin_registry.load_all() {
        eprintln!("[jaringan] warning: plugin loading: {e}");
    }

    let (first, page) = if targets.is_empty() {
        // Welcome page
        let doc = welcome_document();
        let p = LoadedPage {
            location: PageLocation::File(PathBuf::from("welcome")),
            items: collect_items(&doc),
            document: doc,
            signature_status: SignatureStatus::Unsigned,
            stream_rx: None,
            raw_body: None,
            content_type: String::new(),
            body_size: 0,
            load_time_ms: 0,
        };
        (p.location.clone(), p)
    } else {
        // First target is the active page
        let loc = parse_start_location(&targets[0])?;
        let p = load_location(&loc, &script_runtime, &bridge)?;
        (p.location.clone(), p)
    };

    let mut state = BrowserState::new(first.clone(), cfg.clone());
    state.record_current(page.document.title().unwrap_or("Untitled"));
    let file_mtime = file_mtime_of(&page.location);

    // Trigger OnPageLoad hook for initial page
    let plugin_input = ScriptInput {
        title: page.document.title().map(|s| s.to_owned()),
        inputs: Vec::new(),
        page_metadata: None,
        blocks: Vec::new(),
        tui: None,
        form_fields: None,
    };
    plugin_registry.trigger_hook(&PluginHook::OnPageLoad, &plugin_input);
    let mut tabs: Vec<Tab>;

    // Restore persisted tabs if config has tab_persistence enabled
    if cfg.tab_persistence {
        let saved_tabs = load_tabs();
        if !saved_tabs.is_empty() {
            tabs = Vec::new();
            for saved in &saved_tabs {
                let loc = parse_start_location(&saved.url).unwrap_or_else(|_| {
                    PageLocation::Unsupported(saved.url.clone())
                });
                if matches!(loc, PageLocation::File(_) | PageLocation::Network(_) | PageLocation::Web(_)) {
                    match load_location(&loc, &script_runtime, &bridge) {
                        Ok(page) => {
                            let mut s = BrowserState::new(loc.clone(), cfg.clone());
                            s.record_current(page.document.title().unwrap_or("Untitled"));
                            tabs.push(Tab {
                                page,
                                state: s,
                                file_mtime: file_mtime_of(&loc),
                            });
                        }
                        Err(_) => {
                            let doc = Document::new(vec![Block::Preformatted {
                                code: format!("Unable to load: {}", saved.url),
                                language: None,
                            }]);
                            let page = LoadedPage {
                                location: loc.clone(),
                                items: collect_items(&doc),
                                document: doc,
                                signature_status: SignatureStatus::Unsigned,
                                stream_rx: None,
                                raw_body: None,
                                content_type: String::new(),
                                body_size: 0,
                                load_time_ms: 0,
                            };
                            let s = BrowserState::new(loc, cfg.clone());
                            tabs.push(Tab { page, state: s, file_mtime: None });
                        }
                    }
                }
            }
            // Ensure at least one tab
            if tabs.is_empty() {
                tabs = vec![Tab { page, state, file_mtime }];
            }
        } else {
            tabs = vec![Tab { page, state, file_mtime }];
        }
    } else {
        tabs = vec![Tab { page, state, file_mtime }];
    }

    // If extra targets, load them as additional tabs
    for extra in targets.iter().skip(1) {
        if let Ok(loc) = parse_start_location(extra)
            && matches!(loc, PageLocation::File(_) | PageLocation::Network(_) | PageLocation::Web(_))
                && let Ok(extra_page) = load_location(&loc, &script_runtime, &bridge) {
                    let extra_location = extra_page.location.clone();
                    let mut extra_state = BrowserState::new(extra_location.clone(), cfg.clone());
                    extra_state.record_current(extra_page.document.title().unwrap_or("Untitled"));
                    tabs.push(Tab {
                        page: extra_page,
                        state: extra_state,
                        file_mtime: file_mtime_of(&extra_location),
                    });
                }
    }

    let mut active_tab: usize = 0;
    let started = Instant::now();

    loop {
        // Clone the active tab out, work on it, put it back.
        let mut tab = tabs[active_tab].clone();
        clamp_selection(&mut tab.state, tab.page.items.len());

        // Live reload for file-backed pages (files and directories)
        if tab.state.config.live_reload {
            match &tab.page.location {
                PageLocation::File(p) if p.is_file() || p.is_dir() => {
                    check_live_reload(&mut tab.page, &mut tab.state, &mut tab.file_mtime, &script_runtime, &bridge);
                }
                _ => {}
            }
        }

        // Streaming blocks
        if let Some(ref rx) = tab.page.stream_rx
            && let Ok(block) = rx.lock().unwrap().try_recv()
                && let Ok(new_doc) = parse_document(&block) {
                    let block_count = new_doc.blocks.len();
                    for b in new_doc.blocks {
                        tab.page.document.blocks.push(b);
                    }
                    tab.page.items = collect_items(&tab.page.document);
                    tab.state.status = format!("Stream update: +{block_count} blocks");
                }

        // Draw the current tab (immutable borrow of tabs from the stack)
        let frame_result = terminal.draw(|frame| {
            draw_frame(frame, &tabs, active_tab, started.elapsed())
        });
        if let Err(e) = frame_result {
            return Err(e.into());
        }

        // Write the tab back, releasing the clone
        tabs[active_tab] = tab;

        // Poll for keyboard events — handle_key_event borrows tabs mutably
        if !event::poll(Duration::from_millis(120))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) => {
                handle_key_event(&mut tabs, &mut active_tab, terminal, key, &script_runtime, &bridge, &mut plugin_registry)?;
            }
            Event::Mouse(mouse) => {
                handle_mouse_event(&mut tabs, &mut active_tab, mouse, &script_runtime, &bridge)?;
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

/// Handle a single keyboard event, including tab switching and find mode.
fn handle_key_event(
    tabs: &mut Vec<Tab>,
    active_tab: &mut usize,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    key: crossterm::event::KeyEvent,
    script_runtime: &Option<WasmRuntime>,
    bridge: &Option<BridgeState>,
    plugin_registry: &mut PluginRegistry,
) -> anyhow::Result<()> {
    let ctrl = key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(crossterm::event::KeyModifiers::ALT);
    let shift = key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT);

    // ── Tab management keybindings (no borrows of current tab needed) ──
    match key.code {
        KeyCode::Char('t') if ctrl => {
            let doc = welcome_document();
            let p = LoadedPage {
                location: PageLocation::File(PathBuf::from("welcome")),
                items: collect_items(&doc),
                document: doc,
                signature_status: SignatureStatus::Unsigned,
                stream_rx: None,
                raw_body: None,
                content_type: String::new(),
                body_size: 0,
                load_time_ms: 0,
            };
            let mut s = BrowserState::new(p.location.clone(), Default::default());
            s.record_current(p.document.title().unwrap_or("Untitled"));
            tabs.push(Tab {
                page: p,
                state: s,
                file_mtime: None,
            });
            *active_tab = tabs.len() - 1;
            return Ok(());
        }
        KeyCode::Char('w') if ctrl && tabs.len() > 1 => {
            tabs.remove(*active_tab);
            if *active_tab >= tabs.len() {
                *active_tab = tabs.len() - 1;
            }
            return Ok(());
        }
        KeyCode::Tab if ctrl && tabs.len() > 1 => {
            *active_tab = (*active_tab + 1) % tabs.len();
            return Ok(());
        }
        KeyCode::BackTab if ctrl && tabs.len() > 1 => {
            *active_tab = if *active_tab == 0 {
                tabs.len() - 1
            } else {
                *active_tab - 1
            };
            return Ok(());
        }
        // Ctrl+Shift+Left/Right: reorder tabs
        KeyCode::Left if ctrl && shift && *active_tab > 0 => {
            tabs.swap(*active_tab, *active_tab - 1);
            *active_tab -= 1;
            return Ok(());
        }
        KeyCode::Right if ctrl && shift && *active_tab + 1 < tabs.len() => {
            tabs.swap(*active_tab, *active_tab + 1);
            *active_tab += 1;
            return Ok(());
        }
        _ => {}
    }

    // Alt+1-9: jump to tab
    if alt {
        match key.code {
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                let idx = (ch as u8 - b'1') as usize;
                if idx < tabs.len() {
                    *active_tab = idx;
                }
                return Ok(());
            }
            _ => {}
        }
    }

    // Clone the current tab to avoid Vec borrow conflicts
    let mut tab = tabs[*active_tab].clone();
    let page = &mut tab.page;
    let state = &mut tab.state;
    let file_mtime = &mut tab.file_mtime;

    // ── Overlay handling ──────────────────────────────────────────
    if state.overlay.is_some() {
        // GoTo overlay: capture all input for URL/path entry
        if state.overlay == Some(jaringan_browser::Overlay::GoTo) {
            match key.code {
                KeyCode::Esc => {
                    state.overlay = None;
                    state.goto_buffer.clear();
                    state.goto_history_idx = None;
                    state.status = String::from("Cancelled");
                }
                KeyCode::Enter => {
                    let url = state.goto_buffer.clone();
                    if !url.is_empty() {
                        let location = parse_start_location(&url).unwrap_or_else(|_| {
                            PageLocation::Unsupported(url.clone())
                        });
                        if matches!(location, PageLocation::File(_) | PageLocation::Network(_) | PageLocation::Web(_)) {
                            match load_location(&location, script_runtime, bridge) {
                                Ok(loaded) => {
                                    state.overlay = None;
                                    navigate_to(state, loaded.location.clone());
                                    state.record_current(loaded.document.title().unwrap_or("Untitled"));
                                    update_auth_status(state, &loaded.document);
                                    state.status = format!("Navigated to {}", url);
                                    record_goto(&mut state.goto_history, &url);
                                    *page = loaded;
                                    *file_mtime = file_mtime_of(&page.location);
                                }
                                Err(e) => {
                                    state.status = format!("Failed to load {}: {}", url, e);
                                }
                            }
                        } else {
                            state.status = format!("Unsupported location: {}", url);
                        }
                    }
                    state.goto_buffer.clear();
                    state.goto_history_idx = None;
                }
                KeyCode::Up | KeyCode::Char('k') if !state.goto_history.is_empty() => {
                    // Cycle backward through goto history
                    let idx = match state.goto_history_idx {
                        Some(i) if i > 0 => i - 1,
                        Some(_) | None => state.goto_history.len() - 1,
                    };
                    state.goto_history_idx = Some(idx);
                    state.goto_buffer = state.goto_history[idx].clone();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    // Cycle forward through goto history
                    match state.goto_history_idx {
                        Some(i) if i + 1 < state.goto_history.len() => {
                            let idx = i + 1;
                            state.goto_history_idx = Some(idx);
                            state.goto_buffer = state.goto_history[idx].clone();
                        }
                        Some(_) => {
                            // Past the end: show empty
                            state.goto_history_idx = None;
                            state.goto_buffer.clear();
                        }
                        None if !state.goto_history.is_empty() => {
                            // Start at newest (last entry)
                            let idx = state.goto_history.len() - 1;
                            state.goto_history_idx = Some(idx);
                            state.goto_buffer = state.goto_history[idx].clone();
                        }
                        _ => {}
                    }
                }
                KeyCode::Backspace => {
                    state.goto_buffer.pop();
                }
                KeyCode::Char(ch) if !ch.is_control() => {
                    state.goto_buffer.push(ch);
                }
                _ => {}
            }
            // Save changes back to tabs and return
            tabs[*active_tab] = tab;
            return Ok(());
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('?')
                if state.overlay != Some(jaringan_browser::Overlay::Find) =>
            {
                state.overlay = None;
                state.status = String::from("Closed");
            }
            KeyCode::Down | KeyCode::Char('j')
                if state.overlay != Some(jaringan_browser::Overlay::Find) =>
            {
                let count = match state.overlay {
                    Some(jaringan_browser::Overlay::History) => state.history.len(),
                    Some(jaringan_browser::Overlay::Bookmarks) => state.bookmarks.len(),
                    _ => 0,
                };
                if count > 0 {
                    state.overlay_selected = (state.overlay_selected + 1).min(count - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k')
                if state.overlay != Some(jaringan_browser::Overlay::Find) =>
            {
                state.overlay_selected = state.overlay_selected.saturating_sub(1);
            }
            KeyCode::Enter => {
                let url = match state.overlay {
                    Some(jaringan_browser::Overlay::History) => {
                        let rev_idx = state.history.len().saturating_sub(1).saturating_sub(state.overlay_selected);
                        state.history.get(rev_idx).map(|e| e.url.clone())
                    }
                    Some(jaringan_browser::Overlay::Bookmarks) => {
                        state.bookmarks.get(state.overlay_selected).map(|b| b.url.clone())
                    }
                    _ => None,
                };
                if let Some(url) = url {
                    state.overlay = None;
                    let location = parse_start_location(&url).unwrap_or_else(|_| {
                        PageLocation::Unsupported(url.clone())
                    });
                    if matches!(location, PageLocation::File(_) | PageLocation::Network(_) | PageLocation::Web(_)) {
                        state.status = format!("⠋ Loading {url}");
                        let loaded = load_location(&location, script_runtime, bridge)?;
                        navigate_to(state, loaded.location.clone());
                        state.record_current(loaded.document.title().unwrap_or("Untitled"));
                        update_auth_status(state, &loaded.document);
                        state.status = "Opened from history".to_string();
                        *page = loaded;
                        *file_mtime = file_mtime_of(&page.location);
                    }
                }
            }
            KeyCode::Char(ch) if matches!(state.overlay, Some(jaringan_browser::Overlay::Find)) => {
                // Find mode: typing characters into the search query
                if ch.is_control() {
                    // ignore control chars in find input
                } else {
                    state.find_state.query.push(ch);
                    state.find_state.matches = compute_find_matches(page, &state.find_state.query);
                    state.find_state.match_idx = 0;
                }
            }
            KeyCode::Backspace
                if matches!(state.overlay, Some(jaringan_browser::Overlay::Find)) =>
            {
                state.find_state.query.pop();
                state.find_state.matches = compute_find_matches(page, &state.find_state.query);
                state.find_state.match_idx = 0;
            }
            _ => {}
        }
        tabs[*active_tab] = tab;
        return Ok(());
    }

    // ── Text selection mode ────────────────────────────────────────
    if state.text_select_active {
        match key.code {
            KeyCode::Char('V') | KeyCode::Esc => {
                state.text_select_active = false;
                state.status = String::from("Text selection off");
            }
            KeyCode::Char('h') | KeyCode::Left => {
                state.text_select_end.col = state.text_select_end.col.saturating_sub(1);
                state.status = format!("Select: row {} col {}", state.text_select_end.row, state.text_select_end.col);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                state.text_select_end.col = state.text_select_end.col.saturating_add(1);
                state.status = format!("Select: row {} col {}", state.text_select_end.row, state.text_select_end.col);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                state.text_select_end.row = state.text_select_end.row.saturating_add(1);
                state.status = format!("Select: row {} col {}", state.text_select_end.row, state.text_select_end.col);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.text_select_end.row = state.text_select_end.row.saturating_sub(1);
                state.status = format!("Select: row {} col {}", state.text_select_end.row, state.text_select_end.col);
            }
            KeyCode::Char('0') => {
                state.text_select_end.col = 0;
            }
            KeyCode::Char('$') => {
                state.text_select_end.col = u16::MAX;
            }
            KeyCode::Char('y') | KeyCode::Enter => {
                // Build the selected text from rendered lines
                let find_color = find_color_for(state);
                let lines = render_lines(page, state.selected, &state.find_state, find_color, state.show_source);
                let selected_text = extract_selected_text(&lines, state.text_select_start, state.text_select_end);
                if !selected_text.is_empty() {
                    if copy_to_clipboard(&selected_text) {
                        state.status = format!("Copied selection ({} chars)", selected_text.chars().count());
                    } else {
                        // Print to stdout as fallback — gets lost in TUI, but status bar shows it
                        eprintln!("[jaringan] selection: {selected_text}");
                        state.status = format!("Selection ({} chars, clipboard unavailable)", selected_text.chars().count());
                    }
                } else {
                    state.status = String::from("Nothing selected");
                }
                state.text_select_active = false;
            }
            KeyCode::Char('g') => {
                // gg → go to first line
                state.text_select_end.row = 0;
                state.text_select_end.col = 0;
            }
            KeyCode::Char('G') => {
                // G → go to last line
                let find_color = find_color_for(state);
                let line_count = render_lines(page, state.selected, &state.find_state, find_color, state.show_source).len();
                state.text_select_end.row = line_count.saturating_sub(1) as u16;
            }
            _ => {}
        }
        tabs[*active_tab] = tab;
        return Ok(());
    }

    // ── Main keybindings ──────────────────────────────────────────
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc if !is_selected_input(page, state.selected) => {
            // Save tabs before quitting if persistence enabled
            if state.config.tab_persistence {
                let saved: Vec<SavedTab> = tabs
                    .iter()
                    .map(|t| SavedTab {
                        url: t.page.location.display_url(),
                        title: t
                            .page
                            .document
                            .title()
                            .unwrap_or("Untitled")
                            .to_owned(),
                    })
                    .collect();
                save_tabs(&saved);
            }
            // quit
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "quit").into());
        }
        KeyCode::Char(ch) if is_selected_input(page, state.selected) => {
            edit_selected_input(state, page, InputEdit::Append(ch));
        }
        KeyCode::Backspace if is_selected_input(page, state.selected) => {
            edit_selected_input(state, page, InputEdit::Backspace);
        }
        KeyCode::Tab => toggle_mode(state),
        KeyCode::Char('s') => switch_mode(state, BrowserMode::Scroll),
        KeyCode::Char('v') => switch_mode(state, BrowserMode::Selection),
        KeyCode::Char('V') => {
            state.text_select_active = true;
            state.text_select_start = TextSelectPos { row: 0, col: 0 };
            state.text_select_end = TextSelectPos { row: 0, col: 0 };
            state.status = String::from("Text selection: h/j/k/l to move, y to copy, Esc to cancel");
        }
        KeyCode::Char('?') => toggle_overlay(state, jaringan_browser::Overlay::Help),
        KeyCode::Char('h') => toggle_overlay(state, jaringan_browser::Overlay::Help),
        KeyCode::Char('H') => toggle_overlay(state, jaringan_browser::Overlay::History),
        KeyCode::Char('B') => toggle_overlay(state, jaringan_browser::Overlay::Bookmarks),
        KeyCode::Char('y') if state.pending_download.is_none() => {
            let url = state.current.display_url();
            if copy_to_clipboard(&url) {
                state.status = "Copied URL to clipboard".to_string();
            } else {
                state.status = format!("Copy not available: {}", url);
            }
        }
        KeyCode::Char('\\') | KeyCode::Char('u') if ctrl => {
            state.show_source = !state.show_source;
            state.status = if state.show_source {
                "Source view".to_string()
            } else {
                "Rendered view".to_string()
            };
        }
        KeyCode::Char('f') if ctrl => {
            toggle_overlay(state, jaringan_browser::Overlay::Find);
            state.find_state = jaringan_browser::FindState {
                query: String::new(),
                matches: Vec::new(),
                match_idx: 0,
            };
        }
        KeyCode::Down | KeyCode::Char('j') => match state.mode {
            BrowserMode::Selection => selection_down(state, page.items.len()),
            BrowserMode::Scroll => {
                let line_count = render_lines(page, state.selected, &state.find_state, find_color_for(state), state.show_source).len();
                if let Ok(size) = terminal.size() {
                    let viewport_height = size.height.saturating_sub(8);
                    scroll_down(state, line_count, viewport_height);
                }
            }
        },
        KeyCode::Up | KeyCode::Char('k') => match state.mode {
            BrowserMode::Selection => selection_up(state),
            BrowserMode::Scroll => scroll_up(state),
        },
        KeyCode::PageDown | KeyCode::Char(' ') => match state.mode {
            BrowserMode::Selection => selection_down(state, page.items.len()),
            BrowserMode::Scroll => {
                let line_count = render_lines(page, state.selected, &state.find_state, find_color_for(state), state.show_source).len();
                if let Ok(size) = terminal.size() {
                    let viewport_height = size.height.saturating_sub(8);
                    scroll_page_down(state, line_count, viewport_height);
                }
            }
        },
        KeyCode::PageUp => match state.mode {
            BrowserMode::Selection => selection_up(state),
            BrowserMode::Scroll => {
                let line_count = render_lines(page, state.selected, &state.find_state, find_color_for(state), state.show_source).len();
                if let Ok(size) = terminal.size() {
                    let viewport_height = size.height.saturating_sub(8);
                    scroll_page_up(state, line_count, viewport_height);
                }
            }
        },
        KeyCode::Home => selection_first(state),
        KeyCode::End => selection_last(state, page.items.len()),
        KeyCode::Char('g') if !ctrl => {
            state.overlay = Some(jaringan_browser::Overlay::GoTo);
            state.goto_buffer.clear();
            state.status = String::from("Go to: (type URL or path, Enter to go, Esc to cancel)");
        }
        KeyCode::Char('G') => {
            let line_count = render_lines(page, state.selected, &state.find_state, find_color_for(state), state.show_source).len();
            if let Ok(size) = terminal.size() {
                let viewport_height = size.height.saturating_sub(8);
                scroll_to_bottom(state, line_count, viewport_height);
            }
        }
        KeyCode::Char('d') if ctrl => {
            state.toggle_bookmark_current(page.document.title().unwrap_or("Untitled"));
        }
        KeyCode::Char('i') if ctrl => {
            toggle_overlay(state, jaringan_browser::Overlay::PageInfo);
        }
        KeyCode::Char('l') if ctrl => {
            terminal.clear()?;
        }
        KeyCode::Enter if ctrl => {
            // Ctrl+Enter: open link/button in a new background tab
            let Some(item) = page.items.get(state.selected).cloned() else {
                state.status = String::from("No selectable item");
                tabs[*active_tab] = tab;
                return Ok(());
            };
            match item {
                InteractiveItem::Link { label, target } => {
                    let location = resolve_target(&page.location, &target);
                    if matches!(location, PageLocation::File(_) | PageLocation::Network(_) | PageLocation::Web(_)) {
                        match load_location(&location, script_runtime, bridge) {
                            Ok(loaded) => {
                                let mut s = BrowserState::new(loaded.location.clone(), state.config.clone());
                                s.record_current(loaded.document.title().unwrap_or("Untitled"));
                                tabs.push(Tab {
                                    page: loaded,
                                    state: s,
                                    file_mtime: file_mtime_of(&location),
                                });
                                state.status = format!("Opened {label} in new tab (#{})", tabs.len());
                            }
                            Err(e) => {
                                state.status = format!("Failed to load {target}: {e}");
                            }
                        }
                    } else {
                        state.status = format!("Unsupported target: {target}");
                    }
                }
                InteractiveItem::Button(action) => {
                    let mut fresh_page = page.clone();
                    let loc = fresh_page.location.clone();
                    match load_location(&loc, script_runtime, bridge) {
                        Ok(loaded) => {
                            let new_loc = loaded.location.clone();
                            let mut s = BrowserState::new(new_loc.clone(), state.config.clone());
                            s.record_current(loaded.document.title().unwrap_or("Untitled"));
                            tabs.push(Tab {
                                page: loaded,
                                state: s,
                                file_mtime: file_mtime_of(&new_loc),
                            });
                            state.status = format!("Opened in new tab (#{})", tabs.len());
                        }
                        Err(e) => {
                            state.status = format!("Failed to open in new tab: {e}");
                        }
                    }
                }
                _ => {
                    state.status = String::from("This item cannot be opened in a new tab");
                }
            }
            tabs[*active_tab] = tab;
            return Ok(());
        }
        KeyCode::Enter => {
            activate_selected(state, page, script_runtime, bridge)?;
            *file_mtime = file_mtime_of(&page.location);
        }
        KeyCode::Char('b') | KeyCode::Backspace => {
            if go_back(state) {
                match &state.current {
                    location @ (PageLocation::File(_) | PageLocation::Network(_) | PageLocation::Web(_)) => {
                        let loaded = load_location(location, script_runtime, bridge)?;
                        *page = loaded;
                        *file_mtime = file_mtime_of(&page.location);
                        state.record_current(page.document.title().unwrap_or("Untitled"));
                        update_auth_status(state, &page.document);
                    }
                    PageLocation::Unsupported(_) => {}
                }
            }
        }
        KeyCode::Char('f') if !ctrl => {
            if go_forward(state) {
                match &state.current {
                    location @ (PageLocation::File(_) | PageLocation::Network(_) | PageLocation::Web(_)) => {
                        let loaded = load_location(location, script_runtime, bridge)?;
                        *page = loaded;
                        *file_mtime = file_mtime_of(&page.location);
                        state.record_current(page.document.title().unwrap_or("Untitled"));
                        update_auth_status(state, &page.document);
                    }
                    PageLocation::Unsupported(_) => {}
                }
            }
        }
        KeyCode::Char('r') => {
            let loaded = load_location(&page.location, script_runtime, bridge)?;
            *page = loaded;
            *file_mtime = file_mtime_of(&page.location);
            state.status = String::from("Reloaded");
            update_auth_status(state, &page.document);
        }
        KeyCode::Char('n') if ctrl && state.overlay == Some(jaringan_browser::Overlay::Find) => {
            // Next match in find mode
            if !state.find_state.matches.is_empty() {
                state.find_state.match_idx =
                    (state.find_state.match_idx + 1) % state.find_state.matches.len();
                state.status = format!(
                    "Match {}/{}",
                    state.find_state.match_idx + 1,
                    state.find_state.matches.len()
                );
            }
        }
        KeyCode::Char('p') if ctrl && state.overlay == Some(jaringan_browser::Overlay::Find) => {
            // Previous match in find mode
            if !state.find_state.matches.is_empty() {
                state.find_state.match_idx = if state.find_state.match_idx == 0 {
                    state.find_state.matches.len() - 1
                } else {
                    state.find_state.match_idx - 1
                };
                state.status = format!(
                    "Match {}/{}",
                    state.find_state.match_idx + 1,
                    state.find_state.matches.len()
                );
            }
        }
        // Plugin reload: Ctrl+Shift+R — hot-reload all WASM plugins
        KeyCode::Char('R')
            if ctrl
                && key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) =>
        {
            if let Err(e) = plugin_registry.load_all() {
                state.status = format!("Plugin reload failed: {e}");
            } else {
                let count = plugin_registry.plugins().len();
                state.status = format!("Reloaded {count} plugin(s)");
            }
        }
        // Download prompt: y/n
        KeyCode::Char('y') if state.pending_download.is_some() => {
            if let Some(ref dl) = state.pending_download.clone() {
                let result = perform_download(dl);
                state.status = result;
                state.pending_download = None;
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') if state.pending_download.is_some() => {
            state.pending_download = None;
            state.status = String::from("Download cancelled");
        }
        _ => {}
    }

    // Dispatch unhandled keys to plugins
    let plugin_key_input = ScriptInput {
        title: None,
        inputs: Vec::new(),
        page_metadata: None,
        blocks: Vec::new(),
        tui: Some(jaringan_script::TuiContext {
            current_url: Some(page.location.display_url()),
            current_title: page.document.title().map(|s| s.to_owned()),
            scroll_offset: state.scroll_offset as u64,
            selected_index: state.selected,
            mode: format!("{:?}", state.mode),
        }),
        form_fields: None,
    };
    plugin_registry.trigger_hook(&PluginHook::OnKey, &plugin_key_input);

    // Write the cloned tab back (drop borrows first)
    tabs[*active_tab] = tab;
    Ok(())
}

/// Handle mouse events for scrolling, clicking links/buttons, and tab switching.
fn handle_mouse_event(
    tabs: &mut Vec<Tab>,
    active_tab: &mut usize,
    mouse: crossterm::event::MouseEvent,
    script_runtime: &Option<WasmRuntime>,
    bridge: &Option<BridgeState>,
) -> anyhow::Result<()> {
    use crossterm::event::MouseEventKind;

    match mouse.kind {
        MouseEventKind::ScrollDown => {
            let mut tab = tabs[*active_tab].clone();
            let state = &mut tab.state;
            let page = &mut tab.page;
            if state.mode == BrowserMode::Scroll {
                let find_color = find_color_for(state);
                let line_count = render_lines(page, state.selected, &state.find_state, find_color, state.show_source).len();
                if let Ok(size) = crossterm::terminal::size() {
                    let viewport_height = size.1.saturating_sub(8);
                    scroll_down(state, line_count, viewport_height);
                }
            }
            tabs[*active_tab] = tab;
        }
        MouseEventKind::ScrollUp => {
            let mut tab = tabs[*active_tab].clone();
            let state = &mut tab.state;
            if state.mode == BrowserMode::Scroll {
                scroll_up(state);
            }
            tabs[*active_tab] = tab;
        }
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            let row = mouse.row;
            let col = mouse.column;

            // Tab bar is at row 0
            if row == 0 {
                // Map column to tab index based on tab widths
                // Each tab shown as " N:title " — scan through them
                let mut cursor = 0u16;
                for (i, tab) in tabs.iter().enumerate() {
                    let label = tab.page.document.title().unwrap_or("Untitled").to_string();
                    let tab_width = label.chars().count() as u16 + 4; // " N:title "
                    if col >= cursor && col < cursor + tab_width {
                        *active_tab = i;
                        return Ok(());
                    }
                    cursor += tab_width;
                }
                return Ok(());
            }

            // Header area is rows 1-3 (tab is row 1, header border is row
            // 2-3). Body area starts after that.
            if row < 4 {
                return Ok(());
            }

            // Body area: map row to rendered line + scroll_offset
            let mut tab = tabs[*active_tab].clone();
            let state = &mut tab.state;
            let page = &mut tab.page;

            // Row relative to body (top of body = row 4, minus 1 for top border)
            let body_row = (row as usize)
                .saturating_sub(4)
                .saturating_add(state.scroll_offset as usize);

            // Find the item at or near this row in the rendered lines
            // Walk through page.items and find which one corresponds to body_row
            let mut line_idx = 0usize;
            let mut found_item: Option<usize> = None;
            for block in &page.document.blocks {
                let block_lines = block_line_count(block);
                if body_row >= line_idx && body_row < line_idx + block_lines {
                    // Found the block — find its item index
                    let mut item_offset = 0usize;
                    for b in &page.document.blocks {
                        if std::ptr::eq(b, block) {
                            break;
                        }
                        match b {
                            Block::Link(_) | Block::Input(_) | Block::Button(_) | Block::Image(_) => item_offset += 1,
                            _ => {}
                        }
                    }
                    found_item = Some(item_offset);
                    break;
                }
                line_idx += block_lines + 1; // +1 for blank line after most blocks
            }

            if let Some(item_idx) = found_item {
                state.selected = item_idx.min(page.items.len().saturating_sub(1));
                // Activate the selected item (link, button, input)
                activate_selected(state, page, script_runtime, bridge)?;
            }

            tabs[*active_tab] = tab;
        }
        _ => {}
    }
    Ok(())
}

/// Count the rendered lines a Block takes up in the TUI display.
fn block_line_count(block: &Block) -> usize {
    match block {
        Block::Heading { .. } => 2,       // heading line + blank
        Block::Paragraph(text) => text.lines().count().max(1) + 1, // content + blank
        Block::Link(_) => 1,
        Block::Input(_) => 1,
        Block::Button(_) => 1,
        Block::Image(_) => 1,
        Block::Quote(text) => text.lines().count() + 1, // lines + blank
        Block::List(items) => items.len() + 1, // items + blank
        Block::Rule => 2,                  // rule + blank
        Block::Table(table) => {
            // header + separator + rows
            1 + 1 + table.rows.len() + 1 // + blank
        }
        Block::Preformatted { code, .. } => code.lines().count() + 2 + 1, // top border + lines + bottom border + blank
        Block::Script { .. } => 0,        // not rendered
        Block::Auth { .. } => 2,          // auth line + blank
    }
}

/// Draw the full frame including tab bar, page content, footer, and overlays.
fn draw_frame(
    frame: &mut ratatui::Frame<'_>,
    tabs: &[Tab],
    active_tab: usize,
    elapsed: Duration,
) {
    use ratatui::layout::{Constraint, Direction, Layout};

    let area = frame.area();
    frame.render_widget(Clear, area);

    // Theme colours from config
    let cfg = &tabs[active_tab].state.config;
    let accent = parse_color(&cfg.theme.accent);
    let border_color = parse_color(&cfg.theme.border);

    // Tab bar takes 1 line at the top
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tab bar
            Constraint::Length(3), // header
            Constraint::Min(6),    // body
            Constraint::Length(3), // footer
        ])
        .split(area);

    // ── Tab bar ───────────────────────────────────────────────────
    draw_tab_bar(frame, tabs, active_tab, main_chunks[0]);

    let page = &tabs[active_tab].page;
    let state = &tabs[active_tab].state;

    // ── Header ────────────────────────────────────────────────────
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
            .border_style(Style::default().fg(accent)),
    );
    frame.render_widget(header, main_chunks[1]);

    // ── Body ───────────────────────────────────────────────────────
    let find_color = parse_color(&cfg.theme.find_highlight);
    let mut body_lines = render_lines(page, state.selected, &state.find_state, find_color, state.show_source);
    // Apply text selection highlight if active
    if state.text_select_active {
        apply_text_selection(&mut body_lines, state.text_select_start, state.text_select_end);
    }
    let body = Paragraph::new(body_lines)
        .block(
            TuiBlock::default()
                .borders(Borders::LEFT | Borders::RIGHT)
                .border_style(Style::default().fg(border_color)),
        )
        .scroll((state.scroll_offset, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(body, main_chunks[2]);

    // ── Footer ─────────────────────────────────────────────────────
    let spinner = spinner(elapsed);
    let mode_label = if state.show_source {
        "SRC"
    } else {
        match state.mode {
            BrowserMode::Selection => "SEL",
            BrowserMode::Scroll => "SCR",
        }
    };
    let mode_color = if state.show_source {
        Color::Cyan
    } else {
        match state.mode {
            BrowserMode::Selection => Color::Green,
            BrowserMode::Scroll => Color::Yellow,
        }
    };

    let line_count = render_lines(page, state.selected, &state.find_state, find_color_for(state), state.show_source).len();
    let viewport_height = main_chunks[2].height.saturating_sub(2);
    let pct = if line_count > viewport_height as usize {
        ((state.scroll_offset as f64)
            / (line_count.saturating_sub(viewport_height as usize) as f64)
            * 100.0) as u8
    } else {
        100
    };

    let help_text = if state.overlay == Some(jaringan_browser::Overlay::Find) {
        format!(
            "Find: {}  ({}/{})  Ctrl+n next • Ctrl+p prev • Esc close",
            state.find_state.query,
            if state.find_state.matches.is_empty() {
                0
            } else {
                state.find_state.match_idx + 1
            },
            state.find_state.matches.len(),
        )
    } else {
        "j/k ↓↑ • Enter ↵ • g goto • y copy url • H history • B bookmarks • Ctrl+f find • Ctrl+i info • Ctrl+t tab • ? help • q quit"
            .to_string()
    };

    // Build the footer stats line
            let stats = format!(
                "{} · {}B · {}ms",
                tabs[active_tab].page.content_type,
                tabs[active_tab].page.body_size,
                tabs[active_tab].page.load_time_ms
            );
            let footer = Paragraph::new(Line::from(vec![
                Span::styled(format!(" {spinner} "), Style::default().fg(Color::Magenta)),
                Span::styled(
                    format!(" {mode_label} "),
                    Style::default().fg(Color::Black).bg(mode_color).bold(),
                ),
                Span::raw(" "),
                {
                    // Auth indicator
                    let auth_span: Span<'static> = if let Some(ref svc) = state.auth_service_name {
                        if state.has_auth_token {
                            Span::styled(
                                format!(" 🔒{svc} "),
                                Style::default().fg(Color::Green),
                            )
                        } else {
                            Span::styled(
                                format!(" 🔓{svc} "),
                                Style::default().fg(Color::Red),
                            )
                        }
                    } else {
                        Span::raw("")
                    };
                    auth_span
                },
                Span::styled(&state.status, Style::default().fg(Color::Yellow)),
                Span::raw(" · "),
                Span::styled(format!("{pct}%"), Style::default().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::styled(
                    &help_text,
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw("  "),
                Span::styled(
                    &stats,
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw("  "),
                Span::styled(
                    state.current.display_url(),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                ),
            ]))
    .block(
        TuiBlock::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(accent)),
    );
    frame.render_widget(footer, main_chunks[3]);

    // ── Overlays ──────────────────────────────────────────────────
    match state.overlay {
        Some(jaringan_browser::Overlay::Help) => draw_help_overlay(frame, state),
        Some(jaringan_browser::Overlay::History) => draw_history_overlay(frame, state),
        Some(jaringan_browser::Overlay::Bookmarks) => draw_bookmarks_overlay(frame, state),
        Some(jaringan_browser::Overlay::Find) => draw_find_overlay(frame, state),
        Some(jaringan_browser::Overlay::PageInfo) => {
            draw_page_info_overlay(frame, state, page)
        }
        Some(jaringan_browser::Overlay::GoTo) => draw_goto_overlay(frame, state),
        None => {}
    }
}

/// Draw the tab bar at the top of the screen.
fn draw_tab_bar(frame: &mut ratatui::Frame<'_>, tabs: &[Tab], active_tab: usize, area: Rect) {
    let mut spans = Vec::new();
    for (i, tab) in tabs.iter().enumerate() {
        let title = tab.page.document.title().unwrap_or("Untitled");
        let is_watching = tab.state.config.live_reload
            && matches!(&tab.page.location, PageLocation::File(p) if p.is_file() || p.is_dir());
        let watch_mark = if is_watching { " ◉" } else { "" };
        let label = if tab.page.location == PageLocation::File(PathBuf::from("welcome")) {
            format!(" Welcome{watch_mark} ")
        } else if title.len() > 20 {
            // Char-boundary safe truncation (avoid UTF-8 panic)
            let trunc_len = title.floor_char_boundary(18);
            format!(" {}…{watch_mark} ", &title[..trunc_len])
        } else {
            format!(" {title}{watch_mark} ")
        };
        let style = if i == active_tab {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
        };
        let prefix = if i == active_tab { "▸" } else { " " };
        spans.push(Span::styled(
            format!("{prefix}{label}{prefix}"),
            style,
        ));
    }
    if spans.is_empty() {
        return;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn activate_selected(state: &mut BrowserState, page: &mut LoadedPage, script_runtime: &Option<WasmRuntime>, bridge: &Option<BridgeState>) -> anyhow::Result<()> {
    let Some(item) = page.items.get(state.selected).cloned() else {
        state.status = String::from("No selectable item");
        return Ok(());
    };

    match item {
        InteractiveItem::Link { label, target } => match resolve_target(&page.location, &target) {
            location @ (PageLocation::File(_) | PageLocation::Network(_) | PageLocation::Web(_)) => {
                state.status = format!("⠋ Loading {}", location.display_url());
                let loaded = load_location(&location, script_runtime, bridge)?;
                if !is_renderable_content_type(&loaded.content_type) {
                    // Non-renderable content — offer to download
                    let url = location.display_url();
                    let filename = filename_from_url(&url);
                    state.pending_download = Some(DownloadPrompt {
                        url,
                        filename,
                        content_type: loaded.content_type.clone(),
                    });
                    state.status = format!("Download {}? (y/N)", state.pending_download.as_ref().unwrap().filename);
                    return Ok(());
                }
                navigate_to(state, loaded.location.clone());
                state.record_current(loaded.document.title().unwrap_or("Untitled"));
                state.status = format!("Opened {label}");
                *page = loaded;
            }
            PageLocation::Unsupported(target) => {
                state.status = format!("Unsupported target for now: {target}");
            }
        },
        InteractiveItem::Button(action) => activate_button(state, page, action, script_runtime, bridge)?,
        InteractiveItem::Input(input) => {
            state.pending_confirmation = None;
            // Auto-submit: find the first Button on the page and activate it
            let button = page.items.iter().find_map(|item| {
                if let InteractiveItem::Button(action) = item {
                    Some(action.clone())
                } else {
                    None
                }
            });
            if let Some(action) = button {
                state.status = format!("Submitting form from input {0}...", input.name);
                return activate_button(state, page, action, script_runtime, bridge);
            }
            state.status = input_status(&input);
        }
        InteractiveItem::Image(ref image) => {
            state.pending_confirmation = None;
            if state.config.render_images {
                state.status = render_activated_image(&page.location, image);
            } else {
                state.status = image_status(&page.location, image);
            }
        }
    }

    Ok(())
}

fn activate_button(
    state: &mut BrowserState,
    page: &mut LoadedPage,
    action: ButtonAction,
    script_runtime: &Option<WasmRuntime>,
    bridge: &Option<BridgeState>,
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
            let response = if let Some(auth_service) = auth.as_deref() {
                // Look up stored token by service name; button works without one
                match lookup_stored_token(auth_service) {
                    Some(token) => post_tcp_with_action_token(&action_url, payload, &token)?,
                    None => post_tcp(&action_url, payload)?,
                }
            } else {
                post_tcp(&action_url, payload)?
            };
            let mut document = document_from_response(&response)?;
            run_page_scripts(script_runtime, &mut document, bridge);
            navigate_to(state, PageLocation::Network(action_url));
            state.status = format!("Submitted POST action: {target_with_payload}");
            *page = LoadedPage {
                location: state.current.clone(),
                items: collect_items(&document),
                document,
                signature_status: SignatureStatus::Unsigned,
                stream_rx: None,
                raw_body: None,
                content_type: "jrg".to_string(),
                body_size: 0,
                load_time_ms: 0,
            };
        }
        (PageLocation::File(current_file), ActionMethod::Post) => {
            let root = current_file.parent().unwrap_or_else(|| Path::new("."));
            let resolved_target = if target.starts_with("/") {
                format!("jrg://localhost{target}")
            } else {
                format!("jrg://localhost/{}", target.trim_start_matches("./"))
            };
            let action_url = JaringanUrl::parse(&resolved_target)
                .with_context(|| format!("bad action target `{target}`"))?;
            let resolver = LocalFileResolver::new(root);
            let mut request = Request::post(action_url, payload);
            if let Some(auth_service) = auth.as_deref() {
                // Look up stored token by service name; button works without one
                if let Some(token) = lookup_stored_token(auth_service) {
                    request = request.with_action_token(&token);
                }
            }
            let response = resolver.fetch(&request)?;
            let mut document = document_from_response(&response)?;
            run_page_scripts(script_runtime, &mut document, bridge);
            state.status = format!("Submitted local POST action: {target_with_payload}");
            *page = LoadedPage {
                location: page.location.clone(),
                items: collect_items(&document),
                document,
                signature_status: SignatureStatus::Unsigned,
                stream_rx: None,
                raw_body: None,
                content_type: "jrg".to_string(),
                body_size: 0,
                load_time_ms: 0,
            };
        }
        (PageLocation::File(current_file), ActionMethod::Get) if target == "/search" => {
            let root = current_file.parent().unwrap_or_else(|| Path::new("."));
            let query = payload_value(&payload, "q").unwrap_or_default();
            let index = build_local_search_index(root)?;
            let mut document = local_search_results_document(&index, &query);
            run_page_scripts(script_runtime, &mut document, bridge);
            state.status = format!("Searched local index for: {query}");
            *page = LoadedPage {
                location: page.location.clone(),
                items: collect_items(&document),
                document,
                signature_status: SignatureStatus::Unsigned,
                stream_rx: None,
                raw_body: None,
                content_type: "jrg".to_string(),
                body_size: 0,
                load_time_ms: 0,
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

fn input_payload_from_blocks(blocks: &[Block]) -> String {
    blocks
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

/// Show an image using the Kitty terminal protocol.
/// Requires a kitty-compatible terminal (kitty, WezTerm, ghostty, etc.).
fn render_image_kitty(path: &Path) -> String {
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(e) => return format!("Failed to read image: {e}"),
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
    // Kitty protocol: transmit and place at cursor, auto-detect format
    // a=T = transmit, f=100 = auto-detect format from header, d=1 = keep in memory
    print!("\x1b_Ga=T,f=100,d=1;{}\x1b\\", b64);
    use std::io::Write;
    let _ = std::io::stdout().flush();
    format!("🖼 Displayed image ({}) via kitty protocol", path.display())
}

/// Render an image when the user activates it with render_images enabled.
/// Downloads remote images first, then displays via kitty protocol.
fn render_activated_image(page_location: &PageLocation, image: &Image) -> String {
    let local_path = if image.source.starts_with("http://") || image.source.starts_with("https://") {
        let cache_dir = PathBuf::from(
            std::env::var_os("HOME")
                .unwrap_or_else(|| std::ffi::OsString::from("/tmp")),
        )
        .join(".cache/jaringan/images");
        if let Err(e) = fs::create_dir_all(&cache_dir) {
            return format!("Could not create image cache: {e}");
        }
        let output_path = cache_dir.join(cache_filename_for_url(&image.source));
        let response = match reqwest::blocking::get(&image.source) {
            Ok(r) => r,
            Err(e) => return format!("Failed to download image: {e}"),
        };
        let bytes = match response.bytes() {
            Ok(b) => b,
            Err(e) => return format!("Failed to read image data: {e}"),
        };
        let size = bytes.len();
        if let Err(e) = fs::write(&output_path, &bytes) {
            return format!("Failed to save image: {e}");
        }
        eprintln!("Downloaded image: {} ({} bytes)", output_path.display(), size);
        output_path
    } else {
        let PageLocation::File(page_path) = page_location else {
            return image_status(page_location, image);
        };
        page_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(&image.source)
    };

    if local_path.exists() {
        render_image_kitty(&local_path)
    } else {
        format!("Image missing: {}", local_path.display())
    }
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
    match reqwest::blocking::get(url) {
        Ok(response) => {
            match response.bytes() {
                Ok(bytes) => {
                    let size = bytes.len();
                    match fs::write(&output_path, &bytes) {
                        Ok(()) => format!("Downloaded image: {} ({} bytes)", output_path.display(), size),
                        Err(e) => format!("Failed to save image: {e}"),
                    }
                }
                Err(e) => format!("Failed to read image data: {e}"),
            }
        }
        Err(e) => format!("Failed to download image: {e}"),
    }
}

fn run_page_scripts(runtime: &Option<WasmRuntime>, doc: &mut Document, bridge: &Option<BridgeState>) {
    if let Some(rt) = runtime.as_ref()
        && doc.blocks.iter().any(|b| matches!(b, Block::Script { .. })) {
            match execute_document_scripts(rt, doc, bridge.as_ref()) {
                Ok(blocks) => {
                    let meta = doc.metadata.take();
                    *doc = Document::with_metadata(blocks, meta);
                }
                Err(e) => {
                    eprintln!("[jaringan] WASM script error: {e}");
                }
            }
        }
}

fn load_location(
    location: &PageLocation,
    script_runtime: &Option<WasmRuntime>,
    bridge: &Option<BridgeState>,
) -> anyhow::Result<LoadedPage> {
    let keyring = default_keyring();
    let mut page = match location {
        PageLocation::File(path) => {
            if path.is_dir() {
                load_directory_page(path, &keyring)?
            } else {
                load_file_page_with_keyring(path, &keyring)?
            }
        }
        PageLocation::Network(url) => load_network_page_with_keyring(url, &keyring)?,
        PageLocation::Web(url) => load_web_page(url, &keyring)?,
        PageLocation::Unsupported(target) => bail!("unsupported target: {target}"),
    };
    run_page_scripts(script_runtime, &mut page.document, bridge);
    page.items = collect_items(&page.document);
    Ok(page)
}

fn load_web_page(url: &str, keyring: &PublicKeyring) -> anyhow::Result<LoadedPage> {
    let resolver = jaringan_gateway::JrgToHttpResolver::new(
        jaringan_gateway::JrgToHttpResolverConfig::default(),
    );
    let jrg_url = jaringan_browser::web_to_jrg_url(url);
    let parsed = JaringanUrl::parse(&jrg_url)?;
    let request = Request::new(parsed);
    let response = resolver.fetch(&request).map_err(|e| {
        anyhow::anyhow!("failed to fetch web page {url}: {e}")
    })?;
    let document = document_from_response(&response)?;
    let items = collect_items(&document);
    Ok(LoadedPage {
        location: PageLocation::Web(url.to_owned()),
        document,
        items,
        signature_status: verify_source_signature(&response.body, keyring),
        stream_rx: None,
        raw_body: Some(response.body.clone()),
        content_type: "web".to_string(),
        body_size: response.body.len(),
        load_time_ms: 0,
    })
}

/// Convert a minimal subset of markdown to JRG blocks.
/// Handles: #/##/### headings, paragraphs, and - / * lists.
fn md_to_jrg_blocks(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para_lines = Vec::new();
    let mut list_items = Vec::new();
    let mut in_list = false;

    fn flush_para(blocks: &mut Vec<Block>, lines: &mut Vec<String>) {
        if !lines.is_empty() {
            blocks.push(Block::Paragraph(lines.join(" ")));
            lines.clear();
        }
    }

    fn flush_list(blocks: &mut Vec<Block>, items: &mut Vec<String>) {
        if !items.is_empty() {
            blocks.push(Block::List(items.clone()));
            items.clear();
        }
    }

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if in_list {
                flush_list(&mut blocks, &mut list_items);
                in_list = false;
            } else {
                flush_para(&mut blocks, &mut para_lines);
            }
            continue;
        }

        // Heading
        if let Some(rest) = trimmed.strip_prefix("# ") {
            flush_para(&mut blocks, &mut para_lines);
            flush_list(&mut blocks, &mut list_items);
            in_list = false;
            blocks.push(Block::Heading { level: 1, text: rest.to_string() });
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            flush_para(&mut blocks, &mut para_lines);
            flush_list(&mut blocks, &mut list_items);
            in_list = false;
            blocks.push(Block::Heading { level: 2, text: rest.to_string() });
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("### ") {
            flush_para(&mut blocks, &mut para_lines);
            flush_list(&mut blocks, &mut list_items);
            in_list = false;
            blocks.push(Block::Heading { level: 3, text: rest.to_string() });
            continue;
        }

        // List items
        let list_trimmed = trimmed.trim_start();
        if list_trimmed.starts_with("- ") || list_trimmed.starts_with("* ") || list_trimmed.starts_with("+ ") {
            flush_para(&mut blocks, &mut para_lines);
            if !in_list {
                flush_list(&mut blocks, &mut list_items);
            }
            in_list = true;
            list_items.push(list_trimmed[2..].to_string());
            continue;
        }

        // Paragraph
        if in_list {
            flush_list(&mut blocks, &mut list_items);
            in_list = false;
        }
        para_lines.push(trimmed.to_string());
    }

    // Flush remaining
    if in_list {
        flush_list(&mut blocks, &mut list_items);
    } else {
        flush_para(&mut blocks, &mut para_lines);
    }

    blocks
}

fn load_file_page_with_keyring(path: &Path, keyring: &PublicKeyring) -> anyhow::Result<LoadedPage> {
    let path = canonicalish(path);

    if path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext == "jrg") {
        // .jrg files: parse as JRG document
        let source =
            fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let document =
            parse_document(&source).with_context(|| format!("failed to parse {}", path.display()))?;
        let items = collect_items(&document);
        let signature_status = verify_source_signature(&source, keyring);
        return Ok(LoadedPage {
            location: PageLocation::File(path),
            document,
            items,
            signature_status,
            stream_rx: None,
            raw_body: Some(source.clone()),
            content_type: "jrg".to_string(),
            body_size: source.len(),
            load_time_ms: 0,
        });
    }

    // Non-.jrg files
    let source = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let raw_body = source.clone();
    let body_size = raw_body.len();

    // .md files: render as JRG blocks via basic markdown conversion
    if path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext == "md") {
        let document = Document::new(md_to_jrg_blocks(&source));
        let items = collect_items(&document);
        return Ok(LoadedPage {
            location: PageLocation::File(path),
            document,
            items,
            signature_status: SignatureStatus::Unsigned,
            stream_rx: None,
            raw_body: Some(raw_body),
            content_type: "markdown".to_string(),
            body_size,
            load_time_ms: 0,
        });
    }

    // All other non-.jrg files: load as plain text
    let document = Document::new(vec![Block::Preformatted { code: source, language: None }]);
    let items = collect_items(&document);
    Ok(LoadedPage {
        location: PageLocation::File(path),
        document,
        items,
        signature_status: SignatureStatus::Unsigned,
        stream_rx: None,
        raw_body: Some(raw_body),
        content_type: "text".to_string(),
        body_size,
        load_time_ms: 0,
    })
}

fn start_stream_reader(url: &JaringanUrl) -> (Option<Response>, Arc<Mutex<mpsc::Receiver<String>>>, String) {
    let (tx, rx) = mpsc::channel();
    let rx = Arc::new(Mutex::new(rx));
    let url_copy = url.clone();

    let (response, initial_body) = match fetch_tcp_stream(&url_copy) {
        Ok(mut conn) => {
            let resp = conn.response.clone();
            let initial = conn.read_block().ok().flatten().unwrap_or_default();
            std::thread::spawn(move || {
                while let Ok(Some(block)) = conn.read_block() {
                    if tx.send(block).is_err() {
                        break;
                    }
                }
            });
            (Some(resp), initial)
        }
        Err(_) => (None, String::new()),
    };

    (response, rx, initial_body)
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

        // Check if this is a streaming response — open a streaming connection
        let stream_rx = if response.tags.contains(&ResponseTag::Stream)
            || response.content_type == ContentType::JrgStream
        {
            Some(start_stream_reader(&current).1)
        } else {
            None
        };

        return Ok(LoadedPage {
            location: PageLocation::Network(current),
            document,
            items,
            signature_status: verify_source_signature(&response.body, keyring),
            stream_rx,
            raw_body: Some(response.body.clone()),
            content_type: "jrg".to_string(),
            body_size: response.body.len(),
            load_time_ms: 0,
        });
    }

    unreachable!("redirect loop exits by returning once MAX_REDIRECTS is reached")
}

fn redirect_target(response: &Response) -> Option<&str> {
    response.tags.iter().find_map(|tag| match tag {
        ResponseTag::Redirect { target } => Some(target.as_str()),
        ResponseTag::Stream
        | ResponseTag::Key { .. }
        | ResponseTag::ContentType { .. }
        | ResponseTag::Token { .. } => None,
    })
}

fn network_error_page(location: JaringanUrl, message: String) -> LoadedPage {
    let document = Document::new(vec![
        Block::Heading {
            level: 1,
            text: "Network error".to_owned(),
        },
        Block::Preformatted { code: message, language: None },
    ]);

    LoadedPage {
        location: PageLocation::Network(location),
        document,
        items: Vec::new(),
        signature_status: SignatureStatus::Unsigned,
        stream_rx: None,
        raw_body: None,
        content_type: "error".to_string(),
        body_size: 0,
        load_time_ms: 0,
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

    Ok(Document::new(vec![Block::Preformatted { code: text, language: None }]))
}

fn parse_start_location(target: &str) -> anyhow::Result<PageLocation> {
    if target.starts_with("jrg://") {
        return Ok(PageLocation::Network(JaringanUrl::parse(target)?));
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        return Ok(PageLocation::Web(target.to_owned()));
    }
    Ok(PageLocation::File(canonicalish(Path::new(target))))
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

/// Look up an ed25519 public key from the keyring file by key id.
fn lookup_keyring_key(key_id: &str) -> anyhow::Result<String> {
    let path = default_keyring_path();
    if !path.exists() {
        bail!("keyring not found at {}. Use `jaringan-browser generate-key` first.", path.display());
    }
    let source = fs::read_to_string(&path)
        .with_context(|| format!("failed to read keyring {}", path.display()))?;
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 2 && parts[0] == key_id {
            if let Some(key_base64) = parts[1].strip_prefix("ed25519:") {
                return Ok(key_base64.to_owned());
            }
            bail!("key `{key_id}` has unknown format in keyring");
        }
    }
    bail!("key `{key_id}` not found in keyring at {}", path.display());
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

/// A span within a paragraph — either plain text or an inline link.
enum InlineSpan {
    Text(String),
    Link { label: String, target: String },
}

/// Split paragraph text into inline spans, extracting `[label](target)` links.
fn split_inline_spans(text: &str) -> Vec<InlineSpan> {
    let mut spans = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut last_end = 0;

    while i < chars.len() {
        if chars[i] == '[' {
            // Try to find a full [label](target) pattern
            if let Some(end_label) = chars[i + 1..].iter().position(|&c| c == ']') {
                let label_end = i + 1 + end_label;
                if label_end + 1 < chars.len() && chars[label_end + 1] == '('
                    && let Some(end_url) =
                        chars[label_end + 2..].iter().position(|&c| c == ')')
                    {
                        let url_end = label_end + 2 + end_url;
                        let label: String = chars[i + 1..label_end].iter().collect();
                        let target: String = chars[label_end + 2..url_end].iter().collect();
                        if !label.is_empty() && !target.is_empty() {
                            // Push any preceding text
                            if last_end < i {
                                spans.push(InlineSpan::Text(
                                    chars[last_end..i].iter().collect(),
                                ));
                            }
                            spans.push(InlineSpan::Link { label, target });
                            i = url_end + 1;
                            last_end = i;
                            continue;
                        }
                    }
            }
        }
        i += 1;
    }

    // Push remaining text
    if last_end < chars.len() {
        spans.push(InlineSpan::Text(chars[last_end..].iter().collect()));
    }

    spans
}

/// Extract `[label](target)` patterns from paragraph text.
fn extract_inline_links(text: &str) -> Vec<(String, String)> {
    split_inline_spans(text)
        .into_iter()
        .filter_map(|span| match span {
            InlineSpan::Link { label, target } => Some((label, target)),
            _ => None,
        })
        .collect()
}

fn collect_items(document: &Document) -> Vec<InteractiveItem> {
    document
        .blocks
        .iter()
        .flat_map(|block| match block {
            Block::Link(link) => vec![InteractiveItem::Link {
                label: link.label.clone(),
                target: link.target.clone(),
            }],
            Block::Input(input) => vec![InteractiveItem::Input(input.clone())],
            Block::Button(button) => vec![InteractiveItem::Button(ButtonAction {
                id: button.id.clone(),
                label: button.label.clone(),
                target: button.target.clone(),
                method: button.method,
                confirm: button.confirm.clone(),
                auth: button.auth.clone(),
            })],
            Block::Image(image) => vec![InteractiveItem::Image(image.clone())],
            Block::Paragraph(text) => extract_inline_links(text)
                .into_iter()
                .map(|(label, target)| InteractiveItem::Link { label, target })
                .collect(),
            _ => vec![],
        })
        .collect()
}

/// Draw the help overlay.
fn draw_help_overlay(frame: &mut ratatui::Frame<'_>, _state: &BrowserState) {
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
                .title(" ⌨ Help ")
                .title_alignment(ratatui::layout::Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan).bold()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, overlay_area[1]);
    frame.render_widget(help_block, overlay_area[1]);
}

/// Draw the history overlay.
fn draw_history_overlay(frame: &mut ratatui::Frame<'_>, state: &BrowserState) {
    let area = frame.area();
    let overlay_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Max(area.height.saturating_sub(4)),
        ])
        .split(area);

    let mut text = String::new();
    if state.history.is_empty() {
        text.push_str("  No history yet. Browse some pages!\n");
    } else {
        for (i, entry) in state.history.iter().rev().enumerate() {
            let marker = if i == state.overlay_selected { "❯ " } else { "  " };
            text.push_str(&format!("{marker}{}\n  {}\n", entry.title, entry.url));
        }
    }

    let block = Paragraph::new(text)
        .block(
            TuiBlock::default()
                .title(" 📜 History ")
                .title_alignment(ratatui::layout::Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue).bold()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, overlay_area[1]);
    frame.render_widget(block, overlay_area[1]);
}

/// Draw the bookmarks overlay.
fn draw_bookmarks_overlay(frame: &mut ratatui::Frame<'_>, state: &BrowserState) {
    let area = frame.area();
    let overlay_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Max(area.height.saturating_sub(4)),
        ])
        .split(area);

    let mut text = String::from("  Ctrl+d  Bookmark/unbookmark current page\n\n");
    if state.bookmarks.is_empty() {
        text.push_str("  No bookmarks yet. Press Ctrl+d on a page to bookmark it!\n");
    } else {
        for (i, bm) in state.bookmarks.iter().enumerate() {
            let marker = if i == state.overlay_selected { "❯ " } else { "  " };
            text.push_str(&format!("{marker}◆ {}\n  {}\n", bm.name, bm.url));
        }
    }

    let block = Paragraph::new(text)
        .block(
            TuiBlock::default()
                .title(" ★ Bookmarks ")
                .title_alignment(ratatui::layout::Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta).bold()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, overlay_area[1]);
    frame.render_widget(block, overlay_area[1]);
}

/// Draw the find-in-page overlay — shows the search query and match count.
fn draw_find_overlay(frame: &mut ratatui::Frame<'_>, state: &BrowserState) {
    let area = frame.area();
    let overlay_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let match_info = if state.find_state.matches.is_empty() {
        if state.find_state.query.is_empty() {
            String::from("  Type to search...")
        } else {
            String::from("  No matches")
        }
    } else {
        format!(
            "  {} of {} matches  (Ctrl+n next, Ctrl+p prev)",
            state.find_state.match_idx + 1,
            state.find_state.matches.len(),
        )
    };

    let text = format!(
        "Find: {}{match_info}",
        state.find_state.query,
    );

    let block = Paragraph::new(text)
        .block(
            TuiBlock::default()
                .title(" 🔍 Find ")
                .title_alignment(ratatui::layout::Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green).bold()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, overlay_area[1]);
    frame.render_widget(block, overlay_area[1]);
}

/// Draw the "go to" overlay — shows a URL/path input bar.
fn draw_goto_overlay(frame: &mut ratatui::Frame<'_>, state: &BrowserState) {
    let area = frame.area();
    let overlay_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let cursor_hint = if state.goto_buffer.is_empty() {
        String::from("  Type a URL or file path...")
    } else {
        String::from("")
    };

    let text = format!(
        "Go to: {}{}",
        state.goto_buffer, cursor_hint,
    );

    let block = Paragraph::new(text)
        .block(
            TuiBlock::default()
                .title(" 🌐 Go To ")
                .title_alignment(ratatui::layout::Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan).bold()),
        )
        .wrap(Wrap { trim: false });

    let completions = if !state.goto_buffer.is_empty() {
        let q = state.goto_buffer.to_lowercase();
        let mut matches: Vec<&str> = state.history.iter()
            .rev()
            .filter(|e| e.url.to_lowercase().contains(&q) || e.title.to_lowercase().contains(&q))
            .take(5)
            .map(|e| e.url.as_str())
            .collect();
        // Also include goto_history
        for entry in state.goto_history.iter().rev() {
            if entry.to_lowercase().contains(&q) && !matches.contains(&entry.as_str()) && matches.len() < 5 {
                matches.push(entry.as_str());
            }
        }
        matches
    } else {
        Vec::new()
    };

    frame.render_widget(Clear, overlay_area[1]);
    frame.render_widget(block, overlay_area[1]);

    // Draw completion dropdown below the Goto input
    if !completions.is_empty() {
        let completion_text = completions
            .iter()
            .enumerate()
            .map(|(i, url)| format!("  {}. {url}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");

        let completion_block = Paragraph::new(completion_text)
            .block(
                TuiBlock::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .wrap(Wrap { trim: false });

        let completion_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(completions.len() as u16 + 2)])
            .split(overlay_area[1]);

        frame.render_widget(completion_block, completion_area[1]);
    }
}

/// Scan rendered line text for a query and return the line indices that match.
fn compute_find_matches(page: &LoadedPage, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let q = query.to_lowercase();
    // Use a dummy find state to get unhighlighted lines
    let dummy = FindState {
        query: String::new(),
        matches: Vec::new(),
        match_idx: 0,
    };
    let lines = render_lines(page, 0, &dummy, Color::Reset, false);
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            text.to_lowercase().contains(&q)
        })
        .map(|(idx, _)| idx)
        .collect()
}

/// Draw the page info overlay — metadata about the current page.
fn draw_page_info_overlay(
    frame: &mut ratatui::Frame<'_>,
    state: &BrowserState,
    page: &LoadedPage,
) {
    let area = frame.area();
    let overlay_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Max(area.height.saturating_sub(4)),
        ])
        .split(area);

    let title = page.document.title().unwrap_or("Untitled");
    let url = state.current.display_url();
    let sig = security_label(&page.signature_status);
    let block_count = page.document.blocks.len();
    let item_count = page.items.len();
    let dummy = FindState {
        query: String::new(),
        matches: Vec::new(),
        match_idx: 0,
    };
    let line_count = render_lines(page, state.selected, &dummy, Color::Reset, false).len();

    let text = format!(
        "  Title: {title}\n  \
         URL: {url}\n  \n  \
         Content: Jaringan Page\n  \
         Blocks: {block_count}  |  Items: {item_count}  |  Lines: {line_count}\n  \
         Security: {sig}\n  \n  \
         Mode: {mode}\n  \
         Scroll: {scroll}/{max}\n  \n  \
         Press Esc/i to close",
        mode = match state.mode {
            BrowserMode::Selection => "Selection",
            BrowserMode::Scroll => "Scroll",
        },
        scroll = state.scroll_offset,
        max = line_count.saturating_sub(1),
    );

    let block = Paragraph::new(text)
        .block(
            TuiBlock::default()
                .title(" ℹ Page Info ")
                .title_alignment(ratatui::layout::Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue).bold()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, overlay_area[1]);
    frame.render_widget(block, overlay_area[1]);
}

fn render_lines(page: &LoadedPage, selected: usize, find_state: &FindState, find_color: Color, show_source: bool) -> Vec<Line<'static>> {
    if show_source
        && let Some(ref raw) = page.raw_body {
            let mut lines = vec![
                Line::from(Span::styled(
                    "── Source view ──",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )),
                Line::raw(""),
            ];
            for line_text in raw.lines() {
                lines.push(Line::from(Span::styled(
                    line_text.to_string(),
                    Style::default().fg(Color::Gray),
                )));
            }
            return lines;
        }

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
                let spans = split_inline_spans(text);
                if spans.is_empty() {
                    // Empty paragraph
                    lines.push(Line::raw(""));
                    lines.push(Line::raw(""));
                } else if spans.iter().all(|s| matches!(s, InlineSpan::Text(_))) {
                    // No inline links — render as before
                    lines.push(Line::from(Span::styled(
                        text.clone(),
                        Style::default().fg(Color::Gray),
                    )));
                    lines.push(Line::raw(""));
                } else {
                    // Has inline links — build a line with mixed spans
                    let mut line_spans: Vec<Span<'static>> = Vec::new();
                    for span in spans {
                        match span {
                            InlineSpan::Text(t) => {
                                line_spans.push(Span::styled(
                                    t,
                                    Style::default().fg(Color::Gray),
                                ));
                            }
                            InlineSpan::Link { label, target: _ } => {
                                let style = if selected == item_index {
                                    Style::default()
                                        .fg(Color::Black)
                                        .bg(Color::Green)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Color::Green)
                                };
                                line_spans.push(Span::styled(label, style));
                                item_index += 1;
                            }
                        }
                    }
                    lines.push(Line::from(line_spans));
                    lines.push(Line::raw(""));
                }
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
            Block::Preformatted { code, .. } => {
                lines.push(Line::from(Span::styled(
                    "╭─",
                    Style::default().fg(Color::DarkGray),
                )));
                for line in code.lines() {
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
            Block::Script { .. } => {} // Script blocks are not rendered visually
            Block::Auth { service, scope, ttl } => {
                let mut text = format!("[auth] {}", service);
                if let Some(scope) = scope {
                    text.push_str(&format!(" scope=\"{}\"", scope));
                }
                if let Some(ttl) = ttl {
                    text.push_str(&format!(" ttl=\"{}\"", ttl));
                }
                lines.push(Line::from(Span::styled(
                    text,
                    Style::default().fg(Color::LightBlue),
                )));
                lines.push(Line::raw(""));
            }
        }
    }

    apply_find_highlights(&mut lines, find_state, find_color);
    lines
}

/// Apply find-match highlighting to rendered lines.
/// Lines that match the query get a background color; the current match
/// gets a more intense (bold + high-contrast) style.
fn apply_find_highlights(
    lines: &mut [Line<'static>],
    find_state: &FindState,
    find_color: Color,
) {
    if find_state.query.is_empty() || find_state.matches.is_empty() {
        return;
    }
    for (line_idx, line) in lines.iter_mut().enumerate() {
        let is_match = find_state.matches.contains(&line_idx);
        let is_current = find_state
            .matches
            .get(find_state.match_idx)
            .is_some_and(|&m| m == line_idx);

        if !is_match {
            continue;
        }

        if is_current {
            // Current match: bold with high-contrast
            for span in &mut line.spans {
                let style = span.style;
                span.style = style
                    .bg(Color::Yellow)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD);
            }
        } else {
            // Other matches: subtle background highlight
            for span in &mut line.spans {
                let style = span.style;
                // Only apply background if the span doesn't already have one
                if !has_bg_color(&style) {
                    span.style = style.bg(find_color);
                }
            }
        }
    }
}

/// Check if a ratatui Style has any background color set.
fn has_bg_color(style: &Style) -> bool {
    style.bg.is_some()
}

/// Apply text selection highlight to a set of rendered lines.
/// Highlights the region between start and end with a blue background.
fn apply_text_selection(lines: &mut [Line<'static>], start: TextSelectPos, end: TextSelectPos) {
    let row_lo = start.row.min(end.row) as usize;
    let row_hi = start.row.max(end.row) as usize;
    let col_start = if start.row < end.row { start.col } else { end.col } as usize;
    let col_end = if start.row < end.row { end.col } else { start.col } as usize;

    for (row_idx, line) in lines.iter_mut().enumerate() {
        if row_idx < row_lo || row_idx > row_hi {
            continue;
        }
        if row_idx == row_lo && row_idx == row_hi {
            // Single line selection
            apply_col_range_highlight(line, col_start, col_end);
        } else if row_idx == row_lo {
            // First line: from col_start to end
            apply_col_range_highlight(line, col_start, usize::MAX);
        } else if row_idx == row_hi {
            // Last line: from 0 to col_end
            apply_col_range_highlight(line, 0, col_end);
        } else {
            // Middle: full line
            apply_col_range_highlight(line, 0, usize::MAX);
        }
    }
}

/// Apply a blue background highlight to a range of columns within a single Line.
fn apply_col_range_highlight(line: &mut Line<'static>, col_start: usize, col_end: usize) {
    let mut char_offset = 0usize;
    for span in &mut line.spans {
        let span_len = span.content.chars().count();
        let span_start = char_offset;
        let span_end = char_offset + span_len;

        // Check if this span overlaps the selection range
        if span_end > col_start && span_start < col_end {
            let style = span.style;
            span.style = style.bg(Color::Blue).fg(Color::White);
        }

        char_offset += span_len;
    }
}

fn render_browser_table(table: &Table) -> Vec<Line<'static>> {
    const MAX_COL_WIDTH: usize = 40;

    let columns = table
        .headers
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return Vec::new();
    }

    // Compute column widths (clamped to MAX_COL_WIDTH)
    let widths: Vec<usize> = (0..columns)
        .map(|col| {
            let header_w = table.headers.get(col).map(|h| h.chars().count()).unwrap_or(0);
            let max_row_w = table
                .rows
                .iter()
                .filter_map(|row| {
                    row.get(col).map(|cell| {
                        cell.lines().map(|l| l.chars().count()).max().unwrap_or(0)
                    })
                })
                .max()
                .unwrap_or(0);
            header_w.max(max_row_w).min(MAX_COL_WIDTH)
        })
        .collect();

    let mut lines = Vec::new();

    // ── Header row ──
    lines.push(browser_table_row(&table.headers, &widths, true, None, false, &table.alignments));
    // ── Header/body separator ──
    lines.push(Line::from(Span::styled(
        browser_table_separator(&widths),
        Style::default().fg(Color::DarkGray),
    )));

    // ── Data rows ──
    for (row_idx, row) in table.rows.iter().enumerate() {
        let is_even = row_idx % 2 == 0;
        // Check if any cell in this row spans multiple lines
        let max_lines = row
            .iter()
            .map(|cell| cell.lines().count().max(1))
            .max()
            .unwrap_or(1);

        if max_lines > 1 {
            // Multiline: render each sub-row
            for sub_idx in 0..max_lines {
                let sub_row: Vec<String> = (0..columns)
                    .map(|col| {
                        row.get(col)
                            .map(|cell| {
                                cell.lines().nth(sub_idx).unwrap_or("").to_string()
                            })
                            .unwrap_or_default()
                    })
                    .collect();
                lines.push(browser_table_row(&sub_row, &widths, false, Some(row_idx), is_even, &table.alignments));
                if sub_idx + 1 < max_lines {
                    // Thin separator between multi-line rows
                    lines.push(Line::from(Span::styled(
                        browser_table_row_separator(&widths),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
        } else {
            lines.push(browser_table_row(row, &widths, false, Some(row_idx), is_even, &table.alignments));
        }
    }

    lines
}

fn browser_table_row(
    row: &[String],
    widths: &[usize],
    header: bool,
    _row_idx: Option<usize>,
    is_even: bool,
    alignments: &[jaringan_core::Alignment],
) -> Line<'static> {
    let mut spans = vec![Span::styled("  │", Style::default().fg(Color::DarkGray))];
    for (index, width) in widths.iter().enumerate() {
        let cell = row.get(index).map(String::as_str).unwrap_or("");
        // Truncate cell to column width
        let display_cell = if cell.chars().count() > *width {
            cell.chars().take(width.saturating_sub(1)).collect::<String>() + "…"
        } else {
            cell.to_owned()
        };

        let base_style = if header {
            Style::default().fg(Color::Cyan).bold()
        } else if is_even {
            Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 40))
        } else {
            Style::default().fg(Color::White)
        };

        // Apply alignment
        let alignment = alignments.get(index).copied().unwrap_or(jaringan_core::Alignment::None);
        let formatted = match alignment {
            jaringan_core::Alignment::Left => format!(" {display_cell:<width$} "),
            jaringan_core::Alignment::Right => format!(" {display_cell:>width$} "),
            jaringan_core::Alignment::Center => format!(" {:^width$} ", display_cell),
            jaringan_core::Alignment::None => format!(" {display_cell:<width$} "),
        };

        spans.push(Span::styled(formatted, base_style));
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

fn browser_table_row_separator(widths: &[usize]) -> String {
    let cells = widths
        .iter()
        .map(|width| "┄".repeat(width + 2))
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

/// Extract the find highlight color from the browser state's config.
fn find_color_for(state: &BrowserState) -> Color {
    parse_color(&state.config.theme.find_highlight)
}

fn spinner(elapsed: Duration) -> &'static str {
    const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let index = (elapsed.as_millis() / 120) as usize % FRAMES.len();
    FRAMES[index]
}

fn canonicalish(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Append `:7070` to an address string if it doesn't already have a port.
fn ensure_port(addr: &str) -> String {
    // IPv6: "[::1]" or "[::1]:7070" — strip brackets to check for port
    if addr.starts_with('[') {
        let after_bracket = addr.find(']').map(|i| &addr[i + 1..]).unwrap_or("");
        if after_bracket.starts_with(':') {
            addr.to_owned() // already has port
        } else {
            format!("{}:7070", addr.trim_end_matches('/'))
        }
    } else if addr.contains(':') {
        addr.to_owned() // already has port
    } else {
        format!("{}:7070", addr.trim_end_matches('/'))
    }
}

fn help_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Navigation",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  j/k ↓/↑", Style::default().fg(Color::Yellow)),
            Span::raw("     Move selection / scroll"),
        ]),
        Line::from(vec![
            Span::styled("  Enter", Style::default().fg(Color::Yellow)),
            Span::raw("       Open link / Press button / Edit input"),
        ]),
        Line::from(vec![
            Span::styled("  b / f", Style::default().fg(Color::Yellow)),
            Span::raw("       Back / Forward"),
        ]),
        Line::from(vec![
            Span::styled("  r", Style::default().fg(Color::Yellow)),
            Span::raw("          Reload page"),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "Scrolling",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  PgDn/Space / PgUp", Style::default().fg(Color::Yellow)),
            Span::raw("  Page down / up"),
        ]),
        Line::from(vec![
            Span::styled("  g", Style::default().fg(Color::Yellow)),
            Span::raw("         Go to URL / path"),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "History & Bookmarks",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  H", Style::default().fg(Color::Yellow)),
            Span::raw("          Open history panel"),
        ]),
        Line::from(vec![
            Span::styled("  B", Style::default().fg(Color::Yellow)),
            Span::raw("          Open bookmarks panel"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+d", Style::default().fg(Color::Yellow)),
            Span::raw("    Bookmark/unbookmark current page"),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "Modes",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Tab / s / v", Style::default().fg(Color::Yellow)),
            Span::raw("  Toggle / Scroll / Selection mode"),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Home / End", Style::default().fg(Color::Yellow)),
            Span::raw("  First / Last item"),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "Text Selection",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  V", Style::default().fg(Color::Yellow)),
            Span::raw("          Enter text selection mode"),
        ]),
        Line::from(vec![
            Span::styled("  h/j/k/l", Style::default().fg(Color::Yellow)),
            Span::raw("  Move cursor in text selection"),
        ]),
        Line::from(vec![
            Span::styled("  y / Enter", Style::default().fg(Color::Yellow)),
            Span::raw("  Copy selected text"),
        ]),
        Line::from(vec![
            Span::styled("  Esc / V", Style::default().fg(Color::Yellow)),
            Span::raw("    Exit text selection"),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "General",
            Style::default().fg(Color::Cyan).bold(),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  ? / h", Style::default().fg(Color::Yellow)),
            Span::raw("      Toggle help"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+i", Style::default().fg(Color::Yellow)),
            Span::raw("   Page info"),
        ]),
        Line::from(vec![
            Span::styled("  q / Esc", Style::default().fg(Color::Yellow)),
            Span::raw("      Quit"),
        ]),
    ]
}

// ── Script build ─────────────────────────────────────────────────────────

/// Compile a Rust/WAT crate to WASM for Jaringan pages.
///
/// Runs `cargo build --target wasm32-unknown-unknown --release` in the given
/// directory, copies the output .wasm if requested, and optionally generates a
/// .jrg page template with the WASM embedded as base64.
fn cmd_script_build(
    path: &Path,
    output: Option<&Path>,
    template: Option<&Path>,
    title: &str,
    label: &str,
) -> anyhow::Result<()> {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    // 1. Verify the path exists and has a Cargo.toml
    let cargo_toml = path.join("Cargo.toml");
    if !cargo_toml.exists() {
        anyhow::bail!(
            "no Cargo.toml found in {} — this command builds Rust crates targeting wasm32-unknown-unknown.\n\
             For WAT files, use `wat2wasm` from the wabt toolkit, then embed the .wasm manually.",
            path.display()
        );
    }

    // 2. Rust crate — ensure wasm32-unknown-unknown target is installed
    println!("Building WASM script in {}...", path.display());
    let target_check = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .context("failed to check installed targets (is rustup installed?)")?;

    if !String::from_utf8_lossy(&target_check.stdout).contains("wasm32-unknown-unknown") {
        println!("Installing wasm32-unknown-unknown target...");
        let install = Command::new("rustup")
            .args(["target", "add", "wasm32-unknown-unknown"])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .context("failed to install wasm32-unknown-unknown target")?;
        if !install.success() {
            anyhow::bail!("rustup target add wasm32-unknown-unknown failed");
        }
    }

    // 3. Run cargo build
    let mut child = Command::new("cargo")
        .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
        .current_dir(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn cargo build")?;

    // Print build output in real-time
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        #[allow(clippy::lines_filter_map_ok)]
        for l in reader.lines().filter_map(Result::ok) {
            println!("{l}");
        }
    }
    if let Some(stderr) = child.stderr.take() {
        let reader = BufReader::new(stderr);
        #[allow(clippy::lines_filter_map_ok)]
        for l in reader.lines().filter_map(Result::ok) {
            eprintln!("{l}");
        }
    }

    let status = child.wait().context("cargo build failed to complete")?;
    if !status.success() {
        anyhow::bail!("cargo build failed with exit code: {:?}", status.code());
    }

    // 4. Find the output .wasm file
    let target_dir = path.join("target/wasm32-unknown-unknown/release");
    let wasm_files: Vec<_> = std::fs::read_dir(&target_dir)
        .with_context(|| format!("failed to read target directory {}", target_dir.display()))?
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "wasm").then_some(p)
        })
        .collect();

    if wasm_files.is_empty() {
        anyhow::bail!(
            "no .wasm files found in {} after build",
            target_dir.display()
        );
    }

    // Pick the first .wasm file whose stem doesn't start with a dot
    let wasm_path = wasm_files
        .iter()
        .find(|p| !p.file_stem().unwrap_or_default().to_string_lossy().starts_with('.'))
        .or_else(|| wasm_files.first())
        .context("no .wasm files found")?;

    let wasm_bytes = std::fs::read(wasm_path)
        .with_context(|| format!("failed to read {}", wasm_path.display()))?;
    let wasm_size = wasm_bytes.len();

    println!(
        "Built {} ({} bytes)",
        wasm_path.file_name().unwrap_or_default().to_string_lossy(),
        wasm_size
    );

    // 5. Copy to output if requested
    if let Some(out) = output {
        std::fs::copy(wasm_path, out)
            .with_context(|| format!("failed to copy to {}", out.display()))?;
        println!("Copied to {}", out.display());
    }

    // 6. Generate .jrg template if requested
    if let Some(tpl_path) = template {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&wasm_bytes);
        let jrg = format!("# {title}\n\n~> {label}\n{b64}\n~<\n");
        std::fs::write(tpl_path, &jrg)
            .with_context(|| format!("failed to write template to {}", tpl_path.display()))?;
        println!("Generated template: {}", tpl_path.display());
    }

    Ok(())
}

// ── init ──────────────────────────────────────────────────────────────

/// Scaffold a new Jaringan site with example pages.
fn init_jrg_site(path: &Path) -> anyhow::Result<()> {
    let path = if path == Path::new(".") {
        std::env::current_dir().context("failed to get current directory")?
    } else {
        path.to_path_buf()
    };

    if !path.exists() {
        fs::create_dir_all(&path)
            .with_context(|| format!("failed to create directory {}", path.display()))?;
    }

    let index_path = path.join("index.jrg");
    if index_path.exists() {
        bail!("{} already exists -- refusing to overwrite", index_path.display());
    }
    fs::write(
        &index_path,
        concat!(
            "# My Jaringan Site\n",
            "\n",
            "Welcome to your new Jaringan site!\n",
            "\n",
            "## Navigation\n",
            "\n",
            "=> pages/getting-started.jrg Getting Started\n",
            "\n",
            "Edit index.jrg to build your own pages.\n",
            "\n",
            "~~~~~\n",
            "title: Home\n",
            "tags: index, home\n",
        ),
    )
    .with_context(|| format!("failed to write {}", index_path.display()))?;

    let pages_dir = path.join("pages");
    fs::create_dir_all(&pages_dir)
        .with_context(|| "failed to create pages directory".to_string())?;
    fs::write(
        pages_dir.join("getting-started.jrg"),
        concat!(
            "# Getting Started\n",
            "\n",
            "Jaringan pages use a lightweight markup format.\n",
            "\n",
            "## Headings\n",
            "\n",
            "# Title (level 1)\n",
            "## Subtitle (level 2)\n",
            "\n",
            "## Links\n",
            "\n",
            "=> target.jrg Link Label\n",
            "\n",
            "## Buttons\n",
            "\n",
            "! action-id label=\"Do\" method=\"POST\" target=\"/action\"\n",
            "\n",
            "~~~~~\n",
            "title: Getting Started\n",
            "tags: tutorial, basics\n",
        ),
    )
    .with_context(|| "failed to write getting-started.jrg".to_string())?;

    println!("Scaffolded Jaringan site at {}", path.display());
    println!("  open:  jaringan-browser open {}", path.display());
    println!("  serve: jaringan-browser serve {} 127.0.0.1:7070", path.display());
    Ok(())
}

// ── Welcome page ──────────────────────────────────────────────────────

/// Built-in welcome page shown when no target is provided.
fn welcome_document() -> Document {
    Document::new(vec![
        Block::Heading {
            level: 1,
            text: "Welcome to Jaringan".to_owned(),
        },
        Block::Paragraph(
            "Jaringan is a terminal-native browser for Jaringan pages - \
             a lightweight, signed page format for local and network content."
                .to_owned(),
        ),
        Block::Heading {
            level: 2,
            text: "Getting Started".to_owned(),
        },
        Block::Link(jaringan_core::Link {
            target: ".".to_owned(),
            label: "Browse .jrg files in current directory".to_owned(),
        }),
        Block::Paragraph("Press Enter on the link above, or run:".to_owned()),
        Block::Preformatted { code: "  jaringan-browser open <path-or-jrg-url>".to_owned(), language: None },
        Block::Heading {
            level: 2,
            text: "Quick Commands".to_owned(),
        },
        Block::List(vec![
            "j/k down/up - Move selection".to_owned(),
            "Enter - Open link / press button".to_owned(),
            "b/f - Back / Forward".to_owned(),
            "H - History panel".to_owned(),
            "B - Bookmarks panel".to_owned(),
            "Ctrl+d - Bookmark/unbookmark page".to_owned(),
            "? - Toggle help".to_owned(),
        ]),
        Block::Heading {
            level: 2,
            text: "Scaffold a New Site".to_owned(),
        },
        Block::Paragraph("Create a new Jaringan site with:".to_owned()),
        Block::Preformatted { code: "  jaringan-browser init ./my-site".to_owned(), language: None },
        Block::Rule,
        Block::Paragraph("Press ? for full help, or q to quit.".to_owned()),
    ])
}

// ── Directory listing ─────────────────────────────────────────────────

/// Generate a directory-listing document for a directory with .jrg files.
fn load_directory_page(path: &Path, _keyring: &PublicKeyring) -> anyhow::Result<LoadedPage> {
    let dir = canonicalish(path);
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .with_context(|| format!("failed to read directory {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let is_jrg = e
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "jrg");
            let is_subdir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            is_jrg || is_subdir
        })
        .collect();
    entries.sort_by_key(|e| e.path());

    let jrg_count = entries
        .iter()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "jrg")
        })
        .count();

    let mut blocks = vec![
        Block::Heading {
            level: 1,
            text: format!("Directory: {}", dir.display()),
        },
        Block::Paragraph(format!("Found {} .jrg files", jrg_count)),
    ];

    if entries.is_empty() {
        blocks.push(Block::Paragraph(
            "No .jrg files found. Run `jaringan-browser init` \
             or create .jrg files manually."
                .to_owned(),
        ));
    } else {
        for entry in &entries {
            let entry_path = entry.path();
            let relative = entry_path
                .strip_prefix(&dir)
                .unwrap_or(&entry_path)
                .to_string_lossy()
                .to_string();
            if entry_path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "jrg")
            {
                let name = entry_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                blocks.push(Block::Link(jaringan_core::Link {
                    target: relative,
                    label: name.to_owned(),
                }));
            } else if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                blocks.push(Block::Link(jaringan_core::Link {
                    target: relative,
                    label: format!("Directory: {name}/"),
                }));
            }
        }
    }

    let document = Document::new(blocks);
    Ok(LoadedPage {
        location: PageLocation::File(dir),
        items: collect_items(&document),
        document,
        signature_status: SignatureStatus::Unsigned,
        stream_rx: None,
        raw_body: None,
        content_type: "dir".to_string(),
        body_size: 0,
        load_time_ms: 0,
    })
}

// ── Live reload ───────────────────────────────────────────────────────

/// Get the modification time of a file-backed page, if applicable.
/// For directories, returns a combined hash of entry names + mtimes so added/
/// removed/changed files are detected.
fn file_mtime_of(location: &PageLocation) -> Option<SystemTime> {
    match location {
        PageLocation::File(path) if path.is_file() => {
            fs::metadata(path).ok()?.modified().ok()
        }
        PageLocation::File(path) if path.is_dir() => {
            // For directories, use the dir's own mtime (changes on add/remove)
            // plus the mtime of index.jrg if it exists
            let dir_mtime = fs::metadata(path).ok()?.modified().ok()?;
            let index_path = path.join("index.jrg");
            if index_path.is_file() {
                let index_mtime = fs::metadata(&index_path).ok()?.modified().ok()?;
                Some(index_mtime.max(dir_mtime))
            } else {
                Some(dir_mtime)
            }
        }
        _ => None,
    }
}

/// Check if the current file has changed on disk and reload if so.
fn check_live_reload(
    page: &mut LoadedPage,
    state: &mut BrowserState,
    file_mtime: &mut Option<SystemTime>,
    script_runtime: &Option<WasmRuntime>,
    bridge: &Option<BridgeState>,
) {
    let Some(current_mtime) = file_mtime_of(&page.location) else {
        return;
    };
    let changed = file_mtime.is_none_or(|prev| current_mtime != prev);
    if !changed {
        return;
    }
    *file_mtime = Some(current_mtime);
    if let Ok(reloaded) = load_location(&page.location, script_runtime, bridge) {
        *page = reloaded;
        state.record_current(page.document.title().unwrap_or("Untitled"));
        state.status = String::from("⚡ Reloaded (file changed)");
    }
}

// ── Auth status ─────────────────────────────────────────────────────

/// Determine if a content type is renderable in the browser.
fn is_renderable_content_type(ct: &str) -> bool {
    ct.contains("jrg")
        || ct.contains("text/plain")
        || ct.contains("text/html")
        || ct.contains("json")
        || ct.starts_with("dir")
        || ct.is_empty()
        || ct.contains("error")
}

/// Suggest a filename from a URL path.
fn filename_from_url(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let name = path.split('/').last().unwrap_or("download");
    if name.is_empty() || name == "." || name == ".." {
        "download".to_owned()
    } else {
        name.to_owned()
    }
}

/// Download a file to ~/Downloads/ and return a status message.
fn perform_download(dl: &DownloadPrompt) -> String {
    let downloads_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Downloads");
    if let Err(e) = fs::create_dir_all(&downloads_dir) {
        return format!("Failed to create Downloads dir: {e}");
    }

    let dest = downloads_dir.join(&dl.filename);
    // Avoid overwriting — append a number if the file exists
    let dest = if dest.exists() {
        let stem = dest.file_stem().unwrap_or_default().to_string_lossy();
        let ext = dest.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let base = downloads_dir.join(format!("{stem}"));
        let mut counter = 1;
        loop {
            let candidate = base.with_file_name(format!("{stem} ({counter}){ext}"));
            if !candidate.exists() {
                break candidate;
            }
            counter += 1;
        }
    } else {
        dest
    };

    // Try fetching the URL as text and writing it
    // First try JRG TCP for jrg:// URLs
    if dl.url.starts_with("jrg://") {
        match JaringanUrl::parse(&dl.url) {
            Ok(url) => match fetch_tcp(&url) {
                Ok(resp) => {
                    if let Err(e) = fs::write(&dest, &resp.body) {
                        return format!("Failed to write {dest:?}: {e}");
                    }
                    let size = resp.body.len();
                    format!("Downloaded {size}B → {}", dest.display())
                }
                Err(e) => {
                    format!("Download failed: {e}")
                }
            },
            Err(e) => format!("Bad URL: {e}"),
        }
    } else if dl.url.starts_with("http://") || dl.url.starts_with("https://") {
        // Web URL: use reqwest
        match reqwest::blocking::get(&dl.url) {
            Ok(resp) => {
                match resp.bytes() {
                    Ok(bytes) => {
                        if let Err(e) = fs::write(&dest, &bytes) {
                            return format!("Failed to write {dest:?}: {e}");
                        }
                        format!("Downloaded {}B → {}", bytes.len(), dest.display())
                    }
                    Err(e) => format!("Download failed: {e}"),
                }
            }
            Err(e) => format!("Download failed: {e}"),
        }
    } else {
        format!("Cannot download from: {}", dl.url)
    }
}

/// Scan a document for Auth blocks, look up stored tokens, and update
/// the BrowserState fields accordingly.
fn update_auth_status(state: &mut BrowserState, doc: &Document) {
    let auth_block = doc.blocks.iter().find_map(|b| {
        if let Block::Auth { service, .. } = b {
            Some(service.clone())
        } else {
            None
        }
    });
    match auth_block {
        Some(service) => {
            let has_token = lookup_stored_token(&service).is_some();
            state.has_auth_token = has_token;
            state.auth_service_name = Some(service);
        }
        None => {
            state.has_auth_token = false;
            state.auth_service_name = None;
        }
    }
}

// ── Clipboard ────────────────────────────────────────────────────────

/// Extract text from rendered lines between two TextSelectPos positions.
/// start and end define a rectangular selection region: all lines between
/// min(start.row, end.row) and max(start.row, end.row), clipped to column
/// bounds on the first and last lines.
fn extract_selected_text(lines: &[Line<'static>], start: TextSelectPos, end: TextSelectPos) -> String {
    let row_lo = start.row.min(end.row) as usize;
    let row_hi = start.row.max(end.row) as usize;
    let col_start = if start.row < end.row { start.col } else { end.col } as usize;
    let col_end = if start.row < end.row { end.col } else { start.col } as usize;

    let mut result = String::new();
    for row in row_lo..=row_hi {
        if row >= lines.len() {
            continue;
        }
        let line_text = lines[row].to_string();
        let line_len = line_text.chars().count();
        if row == row_lo && row == row_hi {
            // Single line: slice between col_start and col_end
            let start_idx = col_start.min(line_len);
            let end_idx = col_end.min(line_len).max(start_idx);
            result.push_str(&line_text.chars().skip(start_idx).take(end_idx - start_idx).collect::<String>());
        } else if row == row_lo {
            // First line: from col_start to end
            let start_idx = col_start.min(line_len);
            result.push_str(&line_text.chars().skip(start_idx).collect::<String>());
        } else if row == row_hi {
            // Last line: from 0 to col_end
            let end_idx = col_end.min(line_len);
            result.push_str(&line_text.chars().take(end_idx).collect::<String>());
        } else {
            // Middle lines: full content
            result.push_str(&line_text);
        }
        if row < row_hi {
            result.push('\n');
        }
    }
    result
}

/// Copy text to the system clipboard. Tries wl-copy (Wayland) then xclip (X11).
/// Returns true if the copy succeeded, false if neither tool was available.
fn copy_to_clipboard(text: &str) -> bool {
    use std::process::{Command, Stdio};
    use std::io::Write;

    for cmd in &["wl-copy", "xclip"] {
        let args: &[&str] = if *cmd == "xclip" {
            &["-selection", "clipboard"]
        } else {
            &[]
        };
        if let Ok(mut child) = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return true;
            }
        }
    }
    false
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

        assert!(rendered.contains("Navigation"));
        assert!(rendered.contains("? / h"));
        assert!(rendered.contains("Back / Forward"));
        assert!(rendered.contains("Go to URL"));
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
            stream_rx: None,
            raw_body: None,
            content_type: String::new(),
            body_size: 0,
            load_time_ms: 0,
        };

        let rendered = render_lines(&page, 0, &FindState {
            query: String::new(),
            matches: Vec::new(),
            match_idx: 0,
        }, Color::Reset, false)
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

        // Store a token for the demo-search service so the button action works
        store_token("demo-search", "demo-search", None, None).unwrap();
        let document = parse_document(
            "# Tools\n\n? q label=\"Query\" value=\"laksa\"\n! search label=\"Search\" method=\"POST\" target=\"/actions/search\" confirm=\"Submit search?\" auth=\"demo-search\"\n",
        )
        .unwrap();
        let mut page = LoadedPage {
            location: PageLocation::File(root.join("tools.jrg")),
            items: collect_items(&document),
            document,
            signature_status: SignatureStatus::Unsigned,
            stream_rx: None,
            raw_body: None,
            content_type: String::new(),
            body_size: 0,
            load_time_ms: 0,
        };
        let mut state = BrowserState::new(page.location.clone(), Default::default());
        state.selected = 1;

        activate_selected(&mut state, &mut page, &None, &None).unwrap();
        assert_eq!(state.status, "Submit search? Press Enter again to confirm.");
        assert_eq!(
            state
                .pending_confirmation
                .as_ref()
                .map(|action| action.id.as_str()),
            Some("search")
        );

        activate_selected(&mut state, &mut page, &None, &None).unwrap();
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
            stream_rx: None,
            raw_body: None,
            content_type: String::new(),
            body_size: 0,
            load_time_ms: 0,
        };
        let mut state = BrowserState::new(page.location.clone(), Default::default());
        state.selected = 1;

        activate_selected(&mut state, &mut page, &None, &None).unwrap();

        assert_eq!(
            state.status,
            "Submitted local POST action: /actions/search?q=laksa"
        );
        assert!(matches!(
            page.document.blocks.first(),
            Some(Block::Preformatted { code: body, .. })
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
            stream_rx: None,
            raw_body: None,
            content_type: String::new(),
            body_size: 0,
            load_time_ms: 0,
        };
        let mut state = BrowserState::new(page.location.clone(), Default::default());

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
            stream_rx: None,
            raw_body: None,
            content_type: String::new(),
            body_size: 0,
            load_time_ms: 0,
        };
        let mut state = BrowserState::new(page.location.clone(), Default::default());
        state.selected = 1;

        activate_selected(&mut state, &mut page, &None, &None).unwrap();

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

    #[test]
    fn non_jrg_files_are_loaded_as_plain_text() {
        let root = std::env::temp_dir().join(format!(
            "jaringan-ext-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        // .txt file loads as preformatted text
        fs::write(root.join("readme.txt"), "Hello world\n\nThis is plain text.").unwrap();
        let result_txt = load_file_page_with_keyring(
            &root.join("readme.txt"),
            &PublicKeyring::default(),
        );
        assert!(result_txt.is_ok(), ".txt files should load as plain text: {:?}", result_txt);
        let page = result_txt.unwrap();
        assert!(matches!(page.document.blocks.first(), Some(Block::Preformatted { .. })));
        assert_eq!(page.signature_status, SignatureStatus::Unsigned);

        // No extension — loads as plain text too
        fs::write(root.join("readme"), "No extension content").unwrap();
        let result_noext = load_file_page_with_keyring(
            &root.join("readme"),
            &PublicKeyring::default(),
        );
        assert!(result_noext.is_ok(), "files without extension should load as plain text: {:?}", result_noext);
        let page = result_noext.unwrap();
        assert!(matches!(page.document.blocks.first(), Some(Block::Preformatted { .. })));

        // .jrg still parses as JRG
        fs::write(root.join("readme.jrg"), "# Hello").unwrap();
        let result_jrg = load_file_page_with_keyring(
            &root.join("readme.jrg"),
            &PublicKeyring::default(),
        );
        assert!(result_jrg.is_ok(), ".jrg files should load fine");
        let page = result_jrg.unwrap();
        assert!(matches!(page.document.blocks.first(), Some(Block::Heading { .. })));

        fs::remove_dir_all(root).unwrap();
    }
}

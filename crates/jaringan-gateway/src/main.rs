//! JRG Gateway — binary entrypoint
//!
//! Usage:
//!   cargo run -p jaringan-gateway -- serve-http --http-listen 127.0.0.1:8080 --jrg-host 127.0.0.1:7070
//!   cargo run -p jaringan-gateway -- jrg-to-http --jrg-listen 127.0.0.1:7071

use clap::{Parser, Subcommand};

use jaringan_gateway::{HttpToJrgGateway, HttpToJrgGatewayConfig, JrgToHttpResolver, JrgToHttpResolverConfig};

#[derive(Debug, Parser)]
#[command(name = "jaringan-gateway")]
#[command(about = "HTTP↔JRG two-way gateway for the Jaringan protocol")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
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
        #[arg(long, default_value = "Jaringan/0.1 (+https://github.com/thesimonharms/jaringan)")]
        user_agent: String,

        /// Request timeout in seconds
        #[arg(long, default_value_t = 15)]
        timeout: u64,

        /// Maximum response body size in bytes
        #[arg(long, default_value_t = 1_048_576)]
        max_response_size: usize,

        /// Follow redirects
        #[arg(long, default_value_t = true)]
        follow_redirects: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::ServeHttp {
            http_listen,
            jrg_host,
            enable_http_bridge,
            timeout,
        } => {
            // Tokio runtime is needed only for the axum HTTP server
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            rt.block_on(async {
                let config = HttpToJrgGatewayConfig {
                    listen_addr: http_listen,
                    jrg_host,
                    enable_http_bridge,
                    timeout_secs: timeout,
                    ..Default::default()
                };

                let gateway = HttpToJrgGateway::new(config);
                eprintln!("Starting HTTP→JRG gateway...");
                if let Err(e) = gateway.serve().await {
                    eprintln!("Gateway error: {e}");
                    std::process::exit(1);
                }
            });
        }

        Command::JrgToHttp {
            jrg_listen,
            user_agent,
            timeout,
            max_response_size,
            follow_redirects,
        } => {
            let resolver = JrgToHttpResolver::new(JrgToHttpResolverConfig {
                user_agent,
                timeout_secs: timeout,
                max_response_size,
                follow_redirects,
                ..Default::default()
            });

            let listener = match std::net::TcpListener::bind(&jrg_listen) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("Failed to bind JRG TCP listener on {jrg_listen}: {e}");
                    std::process::exit(1);
                }
            };

            eprintln!("JRG→HTTP gateway listening on tcp://{jrg_listen}");
            eprintln!("  Usage: jrg://http/<domain>/<path> for HTTP, jrg://https.<domain>/<path> for HTTPS");

            if let Err(e) = jaringan_protocol::serve(listener, resolver) {
                eprintln!("Server error: {e}");
                std::process::exit(1);
            }
        }
    }
}

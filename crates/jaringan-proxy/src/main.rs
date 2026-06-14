use std::time::Duration;

use clap::Parser;
use jaringan_proxy::{parse_routes, serve};

#[derive(Parser, Debug)]
#[command(name = "jaringan-proxy", about = "JRG vhost reverse proxy")]
struct Cli {
    /// Address to listen on
    #[arg(long, default_value = "0.0.0.0:7070")]
    bind: String,

    /// Virtual host -> backend mappings (comma-separated `host=addr` pairs)
    #[arg(long, value_delimiter = ',', default_value = "")]
    routes: Vec<String>,

    /// Default backend for unmatched hosts (only used with --allow-unknown)
    #[arg(long, default_value = "127.0.0.1:7072")]
    default: String,

    /// Allow unknown hosts to fall through to the default backend
    #[arg(long, default_value_t = false)]
    allow_unknown: bool,

    /// Per-connection timeout in seconds
    #[arg(long, default_value_t = 30)]
    timeout: u64,

    /// Maximum request head size in bytes (default 64 KB)
    #[arg(long, default_value_t = 65536)]
    max_head_size: usize,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let routes = parse_routes(&cli.routes);

    if routes.is_empty() && !cli.allow_unknown {
        eprintln!("⚠️  No --routes specified and --allow-unknown is false. All requests will be rejected.");
    } else if routes.is_empty() {
        eprintln!("⚠️  No --routes specified. All traffic will go to --default {}", cli.default);
    } else {
        eprintln!("📋 VHost routes:");
        for (host, backend) in &routes {
            eprintln!("   {host} -> {backend}");
        }
    }
    if cli.allow_unknown {
        eprintln!("📋 Default backend: {} (unknown hosts allowed)", cli.default);
    } else {
        eprintln!("📋 Unknown hosts: rejected");
    }
    eprintln!("📋 Max head size: {} bytes", cli.max_head_size);
    eprintln!();

    eprintln!("🔀 Jaringan vhost proxy listening on {}", cli.bind);
    serve(
        &cli.bind,
        routes,
        cli.allow_unknown,
        &cli.default,
        Duration::from_secs(cli.timeout),
        cli.max_head_size,
    )?;
    Ok(())
}
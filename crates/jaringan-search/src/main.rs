mod engine;
mod pages;

use std::sync::Arc;

use clap::{Parser, Subcommand};
use engine::SearchEngine;

/// JRG Search Engine — JRG-native search with DNS-verified submissions
#[derive(Parser)]
#[command(name = "jaringan-search")]
#[command(about = "JRG search node — DNS-based domain submission, JRG indexing, and search")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the search engine server
    Serve {
        /// Address to bind (e.g. 127.0.0.1:7080)
        #[arg(long, default_value = "127.0.0.1:7080")]
        bind: String,

        /// Path to store index and submission data
        #[arg(long, default_value = "/tmp/jaringan-search")]
        data_dir: String,

        /// Domain for the search engine itself
        #[arg(long, default_value = "search.localhost")]
        domain: String,

        /// Interval in hours for periodic re-indexing of verified domains (0 = disabled)
        #[arg(long, default_value = "6")]
        reindex_hours: u64,
    },
    /// Show version
    Version,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            bind,
            data_dir,
            domain,
            reindex_hours,
        } => {
            let port = extract_port(&bind).unwrap_or(7080);
            let engine = Arc::new(SearchEngine::new(data_dir, domain, port));

            // Start periodic re-indexing if enabled
            if reindex_hours > 0 {
                engine::start_periodic_reindex(engine.clone());
            }

            let listener = std::net::TcpListener::bind(&bind)
                .map_err(|e| anyhow::anyhow!("failed to bind {bind}: {e}"))?;

            eprintln!("🔍 JRG Search Engine");
            eprintln!("   Listen:  jrg://{bind}");
            eprintln!("   Domain:  {}", engine.domain);
            eprintln!("   Data:    {}", engine.data_dir.display());
            eprintln!("   Pages:   {}", engine.index.lock().unwrap().entries().len());
            if reindex_hours > 0 {
                eprintln!("   Re-index: every {reindex_hours}h");
            }

            jaringan_protocol::serve(listener, engine.clone())?;

            Ok(())
        }
        Commands::Version => {
            println!("jaringan-search {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn extract_port(bind: &str) -> Option<u16> {
    bind.rsplit(':').next()?.parse().ok()
}

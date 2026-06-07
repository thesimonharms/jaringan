use std::{fs, path::PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use jaringan_core::parse_document;
use jaringan_render::render_plain;

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
    }

    Ok(())
}

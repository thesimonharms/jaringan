use std::path::PathBuf;

use clap::{Parser, Subcommand};

use jaringan_formatter::{FormatOptions, JrgFormatter, LintIssue, LintLevel};

#[derive(Parser)]
#[command(name = "jaringan-format", about = "Format and lint .jrg files")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Format .jrg files in-place
    Format {
        /// File paths to format
        #[arg(required = true)]
        paths: Vec<PathBuf>,

        /// Only check, don't write (exit 1 if unformatted)
        #[arg(long)]
        check: bool,

        /// Indent size (default 2)
        #[arg(long, default_value_t = 2)]
        indent: usize,

        /// Max line width (default 80)
        #[arg(long, default_value_t = 80)]
        width: usize,
    },
    /// Lint .jrg files
    Lint {
        /// File paths to lint
        #[arg(required = true)]
        paths: Vec<PathBuf>,

        /// Output JSON report
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Format {
            paths,
            check,
            indent,
            width,
        } => {
            let options = FormatOptions {
                indent_size: indent,
                max_line_width: width,
                ..Default::default()
            };
            let formatter = JrgFormatter::new(options);

            let mut any_unformatted = false;

            for path in paths {
                let source = match std::fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Error reading {}: {e}", path.display());
                        any_unformatted = true;
                        continue;
                    }
                };

                let formatted = match formatter.format_source(&source) {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("Error formatting {}: {e}", path.display());
                        any_unformatted = true;
                        continue;
                    }
                };

                if formatted == source {
                    println!("{}: OK", path.display());
                    continue;
                }

                if check {
                    println!("{}: would reformat", path.display());
                    any_unformatted = true;
                } else {
                    match std::fs::write(&path, &formatted) {
                        Ok(_) => println!("{}: formatted", path.display()),
                        Err(e) => {
                            eprintln!("Error writing {}: {e}", path.display());
                            any_unformatted = true;
                        }
                    }
                }
            }

            if any_unformatted {
                std::process::exit(1);
            }
        }
        Command::Lint { paths, json } => {
            let options = FormatOptions::default();
            let formatter = JrgFormatter::new(options);

            let mut all_issues: Vec<(PathBuf, Vec<LintIssue>)> = Vec::new();
            let mut has_errors = false;

            for path in paths {
                let source = match std::fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Error reading {}: {e}", path.display());
                        has_errors = true;
                        continue;
                    }
                };

                let doc = match jaringan_core::parse_document(&source) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("Error parsing {}: {e}", path.display());
                        has_errors = true;
                        continue;
                    }
                };

                let issues = formatter.lint_document(&doc, &source);

                let file_has_error = issues.iter().any(|i| i.level == LintLevel::Error);
                if file_has_error {
                    has_errors = true;
                }

                if json {
                    all_issues.push((path, issues));
                } else {
                    for issue in &issues {
                        let level = match issue.level {
                            LintLevel::Error => "error",
                            LintLevel::Warning => "warning",
                            LintLevel::Info => "info",
                        };
                        println!(
                            "{}:{}:{}: {}: {} ({})",
                            path.display(),
                            issue.line,
                            level,
                            issue.rule,
                            issue.message,
                            level,
                        );
                    }
                    if issues.is_empty() {
                        println!("{}: no issues", path.display());
                    }
                }
            }

            if json {
                let json_reports: Vec<serde_json::Value> = all_issues
                    .into_iter()
                    .map(|(path, issues)| {
                        serde_json::json!({
                            "file": path.to_string_lossy(),
                            "issues": issues.iter().map(|i| serde_json::json!({
                                "level": i.level.to_string(),
                                "rule": i.rule,
                                "message": i.message,
                                "line": i.line,
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&json_reports).unwrap());
            }

            if has_errors {
                std::process::exit(1);
            }
        }
    }
}

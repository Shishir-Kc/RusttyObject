mod brand;
mod cli;
mod object;
mod server;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "rusttyobject", version, about = "GitHub-native object storage")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the Rust API used by the desktop console.
    Server {
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: String,
    },
    /// Create config.rustyobject and index every file below the current folder.
    #[command(alias = "create-object")]
    Init {
        #[arg(long, short)]
        repo: String,
        #[arg(long, default_value = "main")]
        branch: String,
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Rebuild the local config.rustyobject index.
    Index {
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Re-index the workspace and push its files to the configured GitHub repository.
    Push {
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let args = Args::parse();
    match args.command {
        Some(Command::Server { bind }) => server::run(&bind).await?,
        Some(Command::Init { repo, branch, path }) => cli::init(&repo, &branch, &path)?,
        Some(Command::Index { path }) => cli::index(&path)?,
        Some(Command::Push { path }) => cli::push(&path).await?,
        None => {
            brand::brand();
            cli::show_options();
            cli::run_cli()?;
        }
    }
    Ok(())
}

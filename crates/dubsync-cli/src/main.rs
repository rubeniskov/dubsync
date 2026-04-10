use clap::{Parser, Subcommand};
use dubsync_cli::commands;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "dubsync-cli", about = "DubSync CLI utilities")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Probe an audio/video file and print metadata
    Probe {
        /// Path to the file to probe
        path: PathBuf,
    },
    /// Extract audio from a video file
    Extract {
        /// Path to the video file
        input: PathBuf,
        /// Path to the output audio file
        #[arg(short = 'o')]
        output: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Probe { path } => {
            commands::probe::run(path)?;
        }
        Commands::Extract { input, output } => {
            commands::extract::run(input, output)?;
        }
    }
    Ok(())
}

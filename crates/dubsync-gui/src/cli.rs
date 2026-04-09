use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Path to a project JSON file to load (default command)
    #[arg(short, long)]
    pub project: Option<PathBuf>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Commands {
    /// Take a rendering snapshot of the app given a state JSON
    Snapshot {
        /// Path to a state JSON snapshot to load
        #[arg(short, long)]
        state: PathBuf,

        /// Path to save the rendering snapshot (PNG)
        #[arg(short, long)]
        output: PathBuf,
    },
}

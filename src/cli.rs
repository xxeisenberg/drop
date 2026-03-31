use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, ValueEnum, PartialEq)]
pub enum Method {
    Send,
    Receive,
}

#[derive(Parser)]
pub struct Cli {
    /// Port number (1-65535)
    #[arg(short, long, default_value_t = 1844)]
    pub port: u16,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Send {
        file_path: PathBuf,
    },
    Receive,
    Join {
        /// Optional: The file to upload if the host is in 'Receive' mode
        file_path: Option<PathBuf>,
    },
}

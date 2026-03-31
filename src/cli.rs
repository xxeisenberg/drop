use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// A peer-to-peer file transfer CLI tool.
///
/// Host a session to send or receive files, or join an existing session.
#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Port number to host or connect on (1024-65535)
    #[arg(short, long, default_value_t = 1844, value_parser = clap::value_parser!(u16).range(1024..))]
    pub port: u16,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Host a session and send a file to a joining peer
    Send {
        /// Path to the file to send
        file_path: PathBuf,
    },

    /// Host a session and wait to receive a file from a joining peer
    Receive,

    /// Join an existing session hosted by another peer
    Join {
        /// Path to the file to upload (required when the host is in 'receive' mode, omit when the host is in 'send' mode)
        file_path: Option<PathBuf>,
    },
}

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// A peer-to-peer file transfer CLI tool.
///
/// Host a session to send or receive files, or join an existing session.
#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Host a session and send a file to a joining peer
    Send {
        /// Path to the file to send
        file_path: PathBuf,

        /// Port number to host on (1024-65535)
        #[arg(short, long, default_value_t = 1844, value_parser = clap::value_parser!(u16).range(1024..))]
        port: u16,

        /// Enable end-to-end encryption for CLI-to-CLI transfers
        #[arg(long, default_value_t = false)]
        encrypt: bool,

        /// Disable token protection on generated QR code and browser links
        #[arg(long, default_value_t = false)]
        no_link_token: bool,
    },

    /// Host a session and wait to receive a file from a joining peer
    Receive {
        /// Port number to host on (1024-65535)
        #[arg(short, long, default_value_t = 1844, value_parser = clap::value_parser!(u16).range(1024..))]
        port: u16,

        /// Maximum upload file size in megabytes (default: no limit)
        #[arg(long)]
        max_size: Option<usize>,

        /// Enable end-to-end encryption for CLI-to-CLI transfers
        #[arg(long, default_value_t = false)]
        encrypt: bool,

        /// Disable token protection on generated QR code and browser links
        #[arg(long, default_value_t = false)]
        no_link_token: bool,
    },

    /// Join an existing session hosted by another peer
    Join {
        /// Path to the file to upload (required when the host is in 'receive' mode, omit when the host is in 'send' mode)
        file_path: Option<PathBuf>,
    },
}

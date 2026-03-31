mod cli;
mod client;
mod discovery;
mod server;
mod utils;

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Extension, Router,
};
use clap::Parser;
use cli::{Cli, Commands};
use local_ip_address::local_ip;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use anyhow::{Result,Context};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let token = CancellationToken::new();
    let shutdown_token = token.clone();

    match cli.command {
        Commands::Receive => {
            let local_ip = local_ip().context("Failed to obtain local IP.")?;
            let ip_with_port = format!("{local_ip}:{}", cli.port);

            let app = Router::new()
                .route("/", get(server::get_upload))
                .route("/upload", post(server::post_upload))
                .layer(DefaultBodyLimit::disable())
                .layer(Extension(token.clone()));

            let link = format!("http://{ip_with_port}");
            println!();

            qr2term::print_qr(&link).context(format!("Failed to print QR code."))?;
            println!("\nScan the QR or go to {}", &link);

            discovery::spawn_mdns_advertiser(cli.port, "receive", token);

            let listener = tokio::net::TcpListener::bind(&ip_with_port).await.context(format!("Failed to bind to port {}", cli.port))?;

            axum::serve(listener, app)
                .with_graceful_shutdown(utils::shutdown_signal(shutdown_token))
                .await
                .context(format!("Failed to serve web server at {}.",ip_with_port))?;
        }

        Commands::Send { file_path } => {
            if let Err(e) = std::fs::File::open(&file_path) {
                eprintln!(
                    "[ ERROR ] : Error reading file {}. {}",
                    &file_path.display(),
                    e
                );
                std::process::exit(1);
            }

            let local_ip = local_ip().context("Failed to obtain local IP.")?;
            let ip_with_port = format!("{local_ip}:{}", cli.port);

            let app = Router::new()
                .route("/download", get(server::download))
                .layer(Extension(Arc::new(file_path)))
                .layer(Extension(token.clone()));

            let link = format!("http://{ip_with_port}/download");

            qr2term::print_qr(&link).context(format!("Failed to print QR code."))?;
            println!("\nScan the QR or go to {}", &link);

            discovery::spawn_mdns_advertiser(cli.port, "send", token);

            let listener = tokio::net::TcpListener::bind(&ip_with_port).await.context(format!("Failed to bind to port {}", cli.port))?;
            axum::serve(listener, app)
                .with_graceful_shutdown(utils::shutdown_signal(shutdown_token))
                .await
                .context(format!("Failed to serve web server at {}.",ip_with_port))?;
        }

        Commands::Join { file_path } => {
            client::join_network(file_path).await?;
        }
    }

    Ok(())
}

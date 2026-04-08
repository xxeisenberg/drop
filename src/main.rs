mod cli;
mod client;
mod crypto;
mod discovery;
mod server;
mod tls;
mod utils;

use anyhow::{Context, Result};
use axum::{
    Extension, Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use clap::Parser;
use cli::{Cli, Commands};
use local_ip_address::local_ip;
use rand::RngExt;
use rand::distr::Alphanumeric;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let token = CancellationToken::new();
    let shutdown_token = token.clone();

    match cli.command {
        Commands::Receive {
            port,
            max_size,
            encrypt,
            https,
            tls_cert,
            tls_key,
            no_link_token,
        } => {
            let auth_token: Option<Arc<String>> = (!no_link_token).then(|| {
                Arc::new(
                    rand::rng()
                        .sample_iter(Alphanumeric)
                        .take(32)
                        .map(char::from)
                        .collect(),
                )
            });

            let enc_key: Option<Arc<[u8; 32]>> = if encrypt {
                let key = crypto::generate_key();
                println!("[ CRYPTO ] : End-to-end encryption enabled (AES-256-GCM)");
                Some(Arc::new(key))
            } else {
                None
            };
            let enc_key_encoded: Option<String> = enc_key.as_ref().map(|k| crypto::encode_key(k));

            let local_ip = local_ip().context("Failed to obtain local IP")?;
            let ip_with_port = format!("{local_ip}:{}", port);
            let (_, mdns_host_name) = discovery::get_mdns_names("receive");
            let https_config = if https {
                Some(tls::HttpsConfig::load_or_generate(
                    local_ip,
                    Some(&mdns_host_name),
                    tls_cert.as_deref(),
                    tls_key.as_deref(),
                )?)
            } else {
                None
            };
            let scheme = if https { "https" } else { "http" };

            let current_dir = std::env::current_dir().context("Failed to get current directory")?;

            let body_limit = match max_size {
                Some(mb) => DefaultBodyLimit::max(mb * 1024 * 1024),
                None => DefaultBodyLimit::disable(),
            };

            let app = Router::new()
                .route("/", get(server::get_upload))
                .route("/upload", post(server::post_upload))
                .layer(axum::middleware::from_fn(server::validate_token))
                .layer(body_limit)
                .layer(Extension(token.clone()))
                .layer(Extension(Arc::new(current_dir)))
                .layer(Extension(auth_token.clone()))
                .layer(Extension(enc_key.clone()));

            let link = utils::with_optional_token(
                &utils::build_base_url(scheme, &ip_with_port, None),
                auth_token.as_deref().map(String::as_str),
            );
            println!();

            qr2term::print_qr(&link).context("Failed to print QR code")?;
            println!("\nScan the QR or go to {}", &link);
            if let Some(config) = https_config.as_ref()
                && config.is_generated()
            {
                println!(
                    "[ INFO ] : Using a self-signed HTTPS certificate; browsers may show a trust warning"
                );
            }
            if no_link_token {
                println!("[ INFO ] : Link token disabled for browser access");
            }

            discovery::spawn_mdns_advertiser(
                port,
                "receive",
                scheme,
                auth_token.as_deref().cloned(),
                enc_key_encoded.clone(),
                https_config
                    .as_ref()
                    .map(|config| config.fingerprint().to_string()),
                token,
            );

            if let Some(config) = https_config {
                let addr = ip_with_port.parse()?;
                let shutdown = shutdown_token.clone();
                let serve = tls::serve_https(addr, app, config.rustls_server_config()?, shutdown);
                let (serve_result, _) = tokio::join!(serve, utils::shutdown_signal(shutdown_token));
                serve_result
                    .context(format!("Failed to serve HTTPS server at {}", ip_with_port))?;
            } else {
                let listener = tokio::net::TcpListener::bind(&ip_with_port)
                    .await
                    .context(format!("Failed to bind to port {}", port))?;

                axum::serve(listener, app)
                    .with_graceful_shutdown(utils::shutdown_signal(shutdown_token))
                    .await
                    .context(format!("Failed to serve web server at {}", ip_with_port))?;
            }
        }

        Commands::Send {
            file_path,
            port,
            encrypt,
            https,
            tls_cert,
            tls_key,
            no_link_token,
        } => {
            if let Err(e) = std::fs::File::open(&file_path) {
                eprintln!(
                    "[ ERROR ] : Error reading file {}. {}",
                    &file_path.display(),
                    e
                );
                std::process::exit(1);
            }

            let auth_token: Option<Arc<String>> = (!no_link_token).then(|| {
                Arc::new(
                    rand::rng()
                        .sample_iter(Alphanumeric)
                        .take(32)
                        .map(char::from)
                        .collect(),
                )
            });

            let enc_key: Option<Arc<[u8; 32]>> = if encrypt {
                let key = crypto::generate_key();
                println!("[ CRYPTO ] : End-to-end encryption enabled (AES-256-GCM)");
                Some(Arc::new(key))
            } else {
                None
            };
            let enc_key_encoded: Option<String> = enc_key.as_ref().map(|k| crypto::encode_key(k));

            let local_ip = local_ip().context("Failed to obtain local IP")?;
            let ip_with_port = format!("{local_ip}:{}", port);
            let (_, mdns_host_name) = discovery::get_mdns_names("send");
            let https_config = if https {
                Some(tls::HttpsConfig::load_or_generate(
                    local_ip,
                    Some(&mdns_host_name),
                    tls_cert.as_deref(),
                    tls_key.as_deref(),
                )?)
            } else {
                None
            };
            let scheme = if https { "https" } else { "http" };

            let app = Router::new()
                .route("/download", get(server::download))
                .layer(axum::middleware::from_fn(server::validate_token))
                .layer(Extension(Arc::new(file_path)))
                .layer(Extension(token.clone()))
                .layer(Extension(auth_token.clone()))
                .layer(Extension(enc_key.clone()));

            let link = utils::with_optional_token(
                &utils::build_base_url(scheme, &ip_with_port, Some("/download")),
                auth_token.as_deref().map(String::as_str),
            );

            qr2term::print_qr(&link).context("Failed to print QR code")?;
            println!("\nScan the QR or go to {}", &link);
            if let Some(config) = https_config.as_ref()
                && config.is_generated()
            {
                println!(
                    "[ INFO ] : Using a self-signed HTTPS certificate; browsers may show a trust warning"
                );
            }
            if no_link_token {
                println!("[ INFO ] : Link token disabled for browser access");
            }

            discovery::spawn_mdns_advertiser(
                port,
                "send",
                scheme,
                auth_token.as_deref().cloned(),
                enc_key_encoded.clone(),
                https_config
                    .as_ref()
                    .map(|config| config.fingerprint().to_string()),
                token,
            );

            if let Some(config) = https_config {
                let addr = ip_with_port.parse()?;
                let shutdown = shutdown_token.clone();
                let serve = tls::serve_https(addr, app, config.rustls_server_config()?, shutdown);
                let (serve_result, _) = tokio::join!(serve, utils::shutdown_signal(shutdown_token));
                serve_result
                    .context(format!("Failed to serve HTTPS server at {}", ip_with_port))?;
            } else {
                let listener = tokio::net::TcpListener::bind(&ip_with_port)
                    .await
                    .context(format!("Failed to bind to port {}", port))?;
                axum::serve(listener, app)
                    .with_graceful_shutdown(utils::shutdown_signal(shutdown_token))
                    .await
                    .context(format!("Failed to serve web server at {}", ip_with_port))?;
            }
        }

        Commands::Join { file_path } => {
            client::join_network(file_path).await?;
        }
    }

    Ok(())
}

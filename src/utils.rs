use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

pub fn get_unique_filename(dir: &Path, file_name: &str) -> (String, PathBuf) {
    let mut counter = 1;
    let mut current_name = file_name.to_string();
    let (base_name, ext) = match file_name.rsplit_once('.') {
        Some((n, e)) => (n, format!(".{}", e)),
        None => (file_name, String::new()),
    };

    let mut target_path = dir.join(&current_name);

    while target_path.exists() {
        current_name = format!("{}({}){}", base_name, counter, ext);
        target_path = dir.join(&current_name);
        counter += 1;
    }

    (current_name, target_path)
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1000;
    const MB: u64 = 1000 * KB;
    const GB: u64 = 1000 * MB;

    if bytes >= GB {
        format!("{:.2} Gigabytes", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} Megabytes", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} Kilobytes", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

pub fn with_optional_token(url: &str, token: Option<&str>) -> String {
    match token {
        Some(token) if !token.is_empty() => {
            let separator = if url.contains('?') { '&' } else { '?' };
            format!("{url}{separator}token={token}")
        }
        _ => url.to_string(),
    }
}

pub fn build_base_url(scheme: &str, host: &str, path: Option<&str>) -> String {
    match path {
        Some(path) => format!("{scheme}://{host}{path}"),
        None => format!("{scheme}://{host}"),
    }
}

pub async fn shutdown_signal(token: CancellationToken) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            println!("\n[ INFO ] : Ctrl+C received, shutting down...");
            token.cancel();
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        },
        _ = terminate => {
            println!("\n[ INFO ] : Terminate signal received, shutting down...");
            token.cancel();
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        },
        _ = token.cancelled() => {
            println!("\n[ INFO ] : Transfer complete, shutting down server...");
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }
}

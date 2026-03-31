use crate::utils::format_size;
use anyhow::{Context, Result};
use indicatif::ProgressBar;
use std::path::PathBuf;
use tokio::{fs, io::AsyncWriteExt};

pub async fn join_network(file_path: Option<PathBuf>) -> Result<()> {
    println!("[ INFO ] : Searching for drop on the local network...");

    let mdns = mdns_sd::ServiceDaemon::new()
        .context("Failed to start mDNS service daemon")?;
    let service_type = "_dropshare._tcp.local.";
    let receiver = mdns.browse(service_type)
        .context("Failed to browse for mDNS services")?;

    while let Ok(event) = receiver.recv_async().await {
        if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
            let ip = info
                .get_addresses()
                .iter()
                .find(|addr| addr.is_ipv4())
                .context("Found a host, but it does not have a valid IPv4 address")?;

            let port = info.get_port();
            let properties = info.get_properties();
            let mode = properties.get_property_val_str("mode").unwrap_or("unknown");

            println!(
                "\n[ SUCCESS ] : Found a host in '{}' mode at {}",
                mode,
                info.get_fullname()
            );

            let client = reqwest::Client::new();

            if mode == "send" {
                let url = format!("http://{}:{}/download", ip, port);
                println!("[ INFO ] : Automatically downloading from {}...", url);

                let mut res = client.get(&url).send().await
                    .context(format!("Failed to connect to host at {}", url))?;

                let total_size = res.content_length().unwrap_or(0);

                let mut file_name = String::from("downloaded_file");
                if let Some(cd) = res.headers().get(reqwest::header::CONTENT_DISPOSITION) {
                    let cd_str = cd.to_str().unwrap_or("");
                    if let Some(idx) = cd_str.find("filename=\"") {
                        let start = idx + 10;
                        if let Some(end) = cd_str[start..].find("\"") {
                            file_name = cd_str[start..start + end].to_string();
                        }
                    }
                }

                let pb = ProgressBar::new(total_size);
                println!("[ INFO ] : Incoming file size: {}", format_size(total_size));
                pb.set_message(format!("Downloading \"{}\"...", file_name));

                let mut file = fs::File::create(&file_name).await
                    .context(format!("Failed to create local file: {}", file_name))?;

                while let Some(chunk) = res.chunk().await.context("Failed to read chunk from network")? {
                    file.write_all(&chunk).await
                        .context("Failed to write downloaded chunk to disk")?;
                    pb.inc(chunk.len() as u64);
                }
                pb.finish_with_message(format!("[ SUCCESS ] : Saved as {}", file_name));

            } else if mode == "receive" {
                let url = format!("http://{}:{}/upload", ip, port);

                if let Some(path) = file_path {
                    println!(
                        "[ INFO ] : Automatically uploading {} to {}...",
                        path.display(),
                        url
                    );

                    let file_name = path.file_name()
                        .context("The provided path does not have a valid file name")?
                        .to_str()
                        .context("The file name contains invalid UTF-8 characters")?
                        .to_string();

                    let file = fs::File::open(&path).await
                        .context(format!("Failed to open file for reading: {}", path.display()))?;

                    let part = reqwest::multipart::Part::stream(file).file_name(file_name);
                    let form = reqwest::multipart::Form::new().part("uploadedFile", part);

                    let res = client.post(&url).multipart(form).send().await
                        .context("Failed to send file upload request to the host")?;

                    if res.status().is_success() {
                        println!("[ SUCCESS ] : Upload complete!");
                    } else {
                        println!(
                            "[ ERROR ] : Upload failed. Server responded with: {}",
                            res.status()
                        );
                    }
                } else {
                    println!(
                        "[ ERROR ] : The host is waiting to receive a file, but you didn't provide one."
                    );
                    println!(
                        "Run the command again like this: cargo run -- join ./your_file.txt"
                    );
                }
            }

            break;
        }
    }

    Ok(())
}

use crate::utils::format_size;
use indicatif::ProgressBar;
use std::path::PathBuf;
use tokio::{fs, io::AsyncWriteExt};

pub async fn join_network(file_path: Option<PathBuf>) {
    println!("[ INFO ] : Searching for RustShare on the local network...");

    let mdns = mdns_sd::ServiceDaemon::new().unwrap();
    let service_type = "_dropshare._tcp.local.";
    let receiver = mdns.browse(service_type).unwrap();

    while let Ok(event) = receiver.recv_async().await {
        if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
            let ip = info
                .get_addresses()
                .iter()
                .find(|addr| addr.is_ipv4())
                .unwrap();
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

                let mut res = client.get(&url).send().await.unwrap();
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

                let mut file = fs::File::create(&file_name).await.unwrap();

                while let Some(chunk) = res.chunk().await.unwrap() {
                    file.write_all(&chunk).await.unwrap();
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

                    let file_name = path.file_name().unwrap().to_str().unwrap().to_string();

                    let file = fs::File::open(&path).await.unwrap();
                    let part = reqwest::multipart::Part::stream(file).file_name(file_name);
                    let form = reqwest::multipart::Form::new().part("uploadedFile", part);

                    let res = client.post(&url).multipart(form).send().await.unwrap();

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
}

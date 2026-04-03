use crate::crypto;
use crate::utils::format_size;
use anyhow::{Context, Result};
use dialoguer::{Select, theme::ColorfulTheme};
use indicatif::ProgressBar;
use std::path::PathBuf;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
};

pub async fn join_network(file_path: Option<PathBuf>) -> Result<()> {
    println!("[ INFO ] : Searching for drop on the local network...");

    let mdns = mdns_sd::ServiceDaemon::new().context("Failed to start mDNS service daemon")?;
    let service_type = "_dropshare._tcp.local.";
    let receiver = mdns
        .browse(service_type)
        .context("Failed to browse for mDNS services")?;

    let mut hosts = Vec::new();

    let timeout_duration = std::time::Duration::from_secs(1);
    let _ = tokio::time::timeout(timeout_duration, async {
        while let Ok(event) = receiver.recv_async().await {
            if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
                hosts.push(info);
            }
        }
    })
    .await;

    if hosts.is_empty() {
        println!(
            "\n[ ERROR ] : No hosts found on the local network. Make sure the sender is running."
        );
        return Ok(());
    }

    let selected_host = if hosts.len() == 1 {
        &hosts[0]
    } else {
        println!();
        let host_names: Vec<String> = hosts
            .iter()
            .map(|h| {
                let ip = h
                    .get_addresses()
                    .iter()
                    .find(|addr| addr.is_ipv4())
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| "Unknown IP".to_string());

                let clean_name = h.get_fullname().replace("._dropshare._tcp.local.", "");

                format!("{} [{}]", clean_name, ip)
            })
            .collect();

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Multiple hosts found! Use arrow keys to select one")
            .default(0)
            .items(&host_names)
            .interact()
            .context("Failed to render terminal menu")?;

        &hosts[selection]
    };

    let ip = selected_host
        .get_addresses()
        .iter()
        .find(|addr| addr.is_ipv4())
        .context("Selected host does not have a valid IPv4 address")?;
    let port = selected_host.get_port();
    let properties = selected_host.get_properties();
    let mode = properties.get_property_val_str("mode").unwrap_or("unknown");
    let auth_token = properties.get_property_val_str("token").unwrap_or("");

    let enc_key: Option<[u8; 32]> = properties
        .get_property_val_str("enc_key")
        .and_then(|encoded| crypto::decode_key(encoded).ok());

    if enc_key.is_some() {
        println!("[ CRYPTO ] : End-to-end encryption enabled (AES-256-GCM)");
    }

    println!(
        "\n[ SUCCESS ] : Connecting to host in '{}' mode at {}",
        mode,
        selected_host.get_fullname()
    );

    let client = reqwest::Client::new();

    if mode == "send" {
        let url = format!("http://{}:{}/download?token={}", ip, port, auth_token);
        println!("[ INFO ] : Downloading from host...");

        let mut req = client.get(&url);
        if enc_key.is_some() {
            req = req.header("X-Drop-Encrypted", "true");
        }
        let mut res = req
            .send()
            .await
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

        let (actual_file_name, _) = crate::utils::get_unique_filename(
            std::path::Path::new(""),
            &file_name,
        );

        file_name = actual_file_name;

        let is_encrypted = enc_key.is_some() && res.headers().get("X-Drop-Encrypted").is_some();

        let pb = ProgressBar::new(total_size);
        pb.set_style(
            indicatif::ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta}) \"{msg}\"")
                .context("Failed to set progress bar template")?
                .progress_chars("#>-"),
        );
        println!("[ INFO ] : Incoming file size: {}", format_size(total_size));
        pb.set_message(format!("Downloading \"{}\"...", file_name));

        if is_encrypted {
            let key = enc_key.unwrap();
            let nonce_size = crypto::StreamDecryptor::nonce_size();
            let enc_chunk_size = crypto::StreamDecryptor::encrypted_chunk_size();

            let mut nonce_buf = Vec::new();
            let mut data_buf = Vec::new();
            let mut nonce_read = false;
            let mut decryptor: Option<crypto::StreamDecryptor> = None;

            let mut file = BufWriter::new(
                fs::File::create(&file_name)
                    .await
                    .context(format!("Failed to create local file: {}", file_name))?,
            );

            while let Some(chunk) = res
                .chunk()
                .await
                .context("Failed to read chunk from network")?
            {
                pb.inc(chunk.len() as u64);

                if !nonce_read {
                    nonce_buf.extend_from_slice(&chunk);
                    if nonce_buf.len() >= nonce_size {
                        let nonce: [u8; 7] = nonce_buf[..nonce_size].try_into().unwrap();
                        data_buf.extend_from_slice(&nonce_buf[nonce_size..]);
                        nonce_buf.clear();
                        nonce_read = true;
                        decryptor = Some(crypto::StreamDecryptor::new(&key, &nonce));
                    }
                    continue;
                }

                data_buf.extend_from_slice(&chunk);

                // Decrypt complete chunks
                let dec = decryptor.as_mut().unwrap();
                while data_buf.len() >= enc_chunk_size {
                    let enc_chunk: Vec<u8> = data_buf.drain(..enc_chunk_size).collect();
                    let plaintext = dec.decrypt_next(&enc_chunk).context("Decryption failed")?;
                    file.write_all(&plaintext)
                        .await
                        .context("Failed to write decrypted data to disk")?;
                }
            }

            // Decrypt the last chunk
            if let Some(dec) = decryptor.take() {
                if !data_buf.is_empty() {
                    let plaintext = dec
                        .decrypt_last(&data_buf)
                        .context("Final decryption failed")?;
                    file.write_all(&plaintext)
                        .await
                        .context("Failed to write final decrypted chunk")?;
                }
            }

            file.flush().await.context("Failed to flush file buffer")?;
            pb.finish_with_message(format!(
                "[ SUCCESS ] : Decrypted and saved as {}",
                file_name
            ));
        } else {
            let mut file = BufWriter::new(
                fs::File::create(&file_name)
                    .await
                    .context(format!("Failed to create local file: {}", file_name))?,
            );

            while let Some(chunk) = res
                .chunk()
                .await
                .context("Failed to read chunk from network")?
            {
                file.write_all(&chunk)
                    .await
                    .context("Failed to write downloaded chunk to disk")?;
                pb.inc(chunk.len() as u64);
            }
            file.flush().await.context("Failed to flush file buffer")?;
            pb.finish_with_message(format!("[ SUCCESS ] : Saved as {}", file_name));
        }
    } else if mode == "receive" {
        let url = format!("http://{}:{}/upload?token={}", ip, port, auth_token);

        if let Some(path) = file_path {
            println!("[ INFO ] : Uploading {} to host...", path.display(),);

            let original_file_name = path
                .file_name()
                .context("The provided path does not have a valid file name")?
                .to_str()
                .context("The file name contains invalid UTF-8 characters")?
                .to_string();

            if let Some(key) = enc_key {
                println!("[ CRYPTO ] : Encrypting file before upload...");

                let mut file = fs::File::open(&path).await.context(format!(
                    "Failed to open file for reading: {}",
                    path.display()
                ))?;

                let file_size = file
                    .metadata()
                    .await
                    .context("Failed to read file metadata")?
                    .len();

                let chunk_size = crypto::StreamEncryptor::chunk_size();
                let mut encryptor = crypto::StreamEncryptor::new(&key);
                let nonce = encryptor.nonce_bytes().to_vec();

                let pb = ProgressBar::new(file_size);
                pb.set_style(
                    indicatif::ProgressStyle::default_bar()
                        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta}) \"{msg}\"")
                        .context("Failed to set progress bar template")?
                        .progress_chars("#>-"),
                );
                pb.set_message("Encrypting...");

                let mut encrypted_body = nonce;
                let mut buf = vec![0u8; chunk_size];

                loop {
                    let mut bytes_in_chunk = 0;
                    while bytes_in_chunk < chunk_size {
                        match file.read(&mut buf[bytes_in_chunk..]).await {
                            Ok(0) => break,
                            Ok(n) => {
                                bytes_in_chunk += n;
                                pb.inc(n as u64);
                            }
                            Err(e) => return Err(e.into()),
                        }
                    }

                    if bytes_in_chunk == 0 {
                        let ct = encryptor
                            .encrypt_last(b"")
                            .context("Empty final encryption failed")?;
                        encrypted_body.extend_from_slice(&ct);
                        break;
                    }

                    let is_eof = bytes_in_chunk < chunk_size;
                    let plaintext = &buf[..bytes_in_chunk];

                    if is_eof {
                        let ct = encryptor
                            .encrypt_last(plaintext)
                            .context("Final encryption failed")?;
                        encrypted_body.extend_from_slice(&ct);
                        break;
                    } else {
                        let ct = encryptor
                            .encrypt_next(plaintext)
                            .context("Encryption failed")?;
                        encrypted_body.extend_from_slice(&ct);
                    }
                }
                pb.finish_with_message("Encryption complete");

                let enc_file_name = format!("{}.enc", original_file_name);
                let part = reqwest::multipart::Part::bytes(encrypted_body).file_name(enc_file_name);
                let form = reqwest::multipart::Form::new().part("uploadedFile", part);

                let res = client
                    .post(&url)
                    .multipart(form)
                    .send()
                    .await
                    .context("Failed to send encrypted file upload request")?;

                if res.status().is_success() {
                    println!("[ SUCCESS ] : Encrypted upload complete!");
                } else {
                    println!(
                        "[ ERROR ] : Upload failed. Server responded with: {}",
                        res.status()
                    );
                }
            } else {
                let file = fs::File::open(&path).await.context(format!(
                    "Failed to open file for reading: {}",
                    path.display()
                ))?;

                let part = reqwest::multipart::Part::stream(file).file_name(original_file_name);
                let form = reqwest::multipart::Form::new().part("uploadedFile", part);

                let res = client
                    .post(&url)
                    .multipart(form)
                    .send()
                    .await
                    .context("Failed to send file upload request to the host")?;

                if res.status().is_success() {
                    println!("[ SUCCESS ] : Upload complete!");
                } else {
                    println!(
                        "[ ERROR ] : Upload failed. Server responded with: {}",
                        res.status()
                    );
                }
            }
        } else {
            println!(
                "[ ERROR ] : The host is waiting to receive a file, but you didn't provide one."
            );
            println!("Run the command again like this: cargo run -- join ./your_file.txt");
        }
    }

    Ok(())
}

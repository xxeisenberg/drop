use crate::crypto;
use crate::utils::format_size;
use anyhow::Context;
use axum::{
    Extension,
    body::Body,
    extract::{Multipart, Request},
    http::{Response, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response as AxumResponse},
};
use indicatif::ProgressBar;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
};
use tokio_util::{io::ReaderStream, sync::CancellationToken};

pub struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> AxumResponse {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Error: {:#}", self.0),
        )
            .into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

pub async fn validate_token(
    Extension(expected_token): Extension<Option<Arc<String>>>,
    request: Request,
    next: Next,
) -> AxumResponse {
    let Some(expected_token) = expected_token else {
        return next.run(request).await;
    };

    let query = request.uri().query().unwrap_or("");
    let is_valid = query.split('&').any(|pair| {
        pair.split_once('=')
            .map(|(k, v)| k == "token" && v == expected_token.as_str())
            .unwrap_or(false)
    });

    if !is_valid {
        return (
            StatusCode::UNAUTHORIZED,
            "Unauthorized: invalid or missing token",
        )
            .into_response();
    }

    next.run(request).await
}

pub async fn download(
    Extension(file_path): Extension<Arc<PathBuf>>,
    Extension(token): Extension<CancellationToken>,
    Extension(enc_key): Extension<Option<Arc<[u8; 32]>>>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let use_encryption = enc_key.is_some() && headers.get("X-Drop-Encrypted").is_some();
    let key = if use_encryption { enc_key } else { None };
    let res = stream_file(file_path, key).await?;
    token.cancel();
    Ok(res)
}

pub async fn get_upload(
    Extension(auth_token): Extension<Option<Arc<String>>>,
) -> Result<impl IntoResponse, AppError> {
    let html = "<html lang=\"en\">
    <head>
        <meta charset=\"UTF-8\">
        <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">
        <title>File Upload</title>
        <style>
            body { font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif; background-color: #f3f4f6; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; }
            .upload-container { background-color: #ffffff; padding: 40px; border-radius: 12px; box-shadow: 0 8px 20px rgba(0, 0, 0, 0.08); text-align: center; width: 100%; max-width: 350px; }
            .upload-container h2 { margin-top: 0; color: #1f2937; font-size: 1.5rem; margin-bottom: 24px; }
            input[type=\"file\"] { display: block; width: 100%; margin-bottom: 24px; padding: 16px 10px; border: 2px dashed #d1d5db; border-radius: 8px; background-color: #f9fafb; color: #4b5563; box-sizing: border-box; cursor: pointer; transition: border-color 0.3s ease; }
            input[type=\"file\"]:hover { border-color: #3b82f6; }
            input[type=\"file\"]::file-selector-button { background-color: #e5e7eb; border: none; padding: 8px 16px; border-radius: 6px; color: #374151; cursor: pointer; margin-right: 12px; font-weight: 500; transition: background-color 0.3s ease; }
            input[type=\"file\"]::file-selector-button:hover { background-color: #d1d5db; }
            button[type=\"submit\"] { background-color: #3b82f6; color: white; border: none; padding: 12px 24px; font-size: 16px; font-weight: 600; border-radius: 8px; cursor: pointer; width: 100%; transition: background-color 0.3s ease, transform 0.1s ease; }
            button[type=\"submit\"]:hover { background-color: #2563eb; }
            button[type=\"submit\"]:active { transform: scale(0.98); }
        </style>
    </head>
    <body>
        <div class=\"upload-container\">
            <h2>Upload Files</h2>
            <form action=\"/upload\" method=\"post\" enctype=\"multipart/form-data\">
                <input type=\"file\" name=\"uploadedFile\" required multiple>
                <button type=\"submit\">Upload</button>
            </form>
        </div>
    </body>
    </html>";

    let action =
        crate::utils::with_optional_token("/upload", auth_token.as_deref().map(String::as_str));
    let html = html.replace("action=\"/upload\"", &format!("action=\"{action}\""));

    let response = Response::builder()
        .header(header::CONTENT_TYPE, "text/html")
        .body(Body::from(html))
        .context("Failed to build HTML response for get_upload")?;

    Ok(response)
}

pub fn sanitize_filename(name: &str) -> String {
    std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty() && *n != "." && *n != "..")
        .unwrap_or("file")
        .to_string()
}

pub async fn post_upload(
    Extension(token): Extension<CancellationToken>,
    Extension(upload_dir): Extension<Arc<PathBuf>>,
    Extension(enc_key): Extension<Option<Arc<[u8; 32]>>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    loop {
        let mut field = match multipart
            .next_field()
            .await
            .context("Failed to read multipart field")?
        {
            Some(f) => f,
            None => break,
        };

        // Sanitize filename: extract only the basename to prevent path traversal
        let original_file_name = field.file_name().unwrap_or("file");
        let original_file_name = sanitize_filename(original_file_name);

        let (actual_file_name, is_encrypted) =
            if original_file_name.ends_with(".enc") && enc_key.is_some() {
                (
                    original_file_name.strip_suffix(".enc").unwrap().to_string(),
                    true,
                )
            } else {
                (original_file_name.clone(), false)
            };

        let mut file_name = actual_file_name.clone();

        let field_content_length = field
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        let bar = match field_content_length {
            Some(len) => ProgressBar::new(len),
            None => ProgressBar::new_spinner(),
        };

        bar.set_style(
            indicatif::ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta}) \"{msg}\"",
                )
                .context("Failed to set progress bar template")?
                .progress_chars("#>-"),
        );
        bar.set_message(file_name.clone());
        bar.enable_steady_tick(std::time::Duration::from_millis(100));

        let (actual_file_name, target_path) =
            crate::utils::get_unique_filename(&upload_dir, &actual_file_name);
        file_name = actual_file_name;

        let mut file = match fs::File::create(&target_path).await {
            Ok(f) => BufWriter::new(f),
            Err(e) => {
                bar.abandon_with_message(format!(
                    "[ ERROR ] : Could not create file on disk: {}",
                    e
                ));
                return Err(anyhow::anyhow!("Disk error: {}", e).into());
            }
        };

        let mut upload_successful = true;

        if is_encrypted {
            let key = enc_key.as_ref().unwrap();
            let mut nonce_buf = Vec::new();
            let mut data_buf = Vec::new();
            let nonce_size = crypto::StreamDecryptor::nonce_size();
            let enc_chunk_size = crypto::StreamDecryptor::encrypted_chunk_size();
            let mut nonce_read = false;
            let mut decryptor: Option<crypto::StreamDecryptor> = None;

            loop {
                match field.chunk().await {
                    Ok(Some(chunk)) => {
                        bar.inc(chunk.len() as u64);

                        if !nonce_read {
                            nonce_buf.extend_from_slice(&chunk);
                            if nonce_buf.len() >= nonce_size {
                                let nonce: [u8; 7] = nonce_buf[..nonce_size].try_into().unwrap();
                                data_buf.extend_from_slice(&nonce_buf[nonce_size..]);
                                nonce_buf.clear();
                                nonce_read = true;
                                decryptor = Some(crypto::StreamDecryptor::new(key, &nonce));
                            }
                            continue;
                        }

                        data_buf.extend_from_slice(&chunk);

                        // Process complete encrypted chunks
                        let dec = decryptor.as_mut().unwrap();
                        while data_buf.len() >= enc_chunk_size {
                            let enc_chunk: Vec<u8> = data_buf.drain(..enc_chunk_size).collect();
                            match dec.decrypt_next(&enc_chunk) {
                                Ok(plaintext) => {
                                    if let Err(e) = file.write_all(&plaintext).await {
                                        bar.abandon_with_message(format!(
                                            "[ ERROR ] : Failed to write to disk: {}",
                                            e
                                        ));
                                        upload_successful = false;
                                        break;
                                    }
                                }
                                Err(e) => {
                                    bar.abandon_with_message(format!(
                                        "[ ERROR ] : Decryption failed: {}",
                                        e
                                    ));
                                    upload_successful = false;
                                    break;
                                }
                            }
                        }
                        if !upload_successful {
                            break;
                        }
                    }
                    Ok(None) => {
                        // Decrypt the last chunk
                        if let Some(dec) = decryptor.take() {
                            if !data_buf.is_empty() {
                                match dec.decrypt_last(&data_buf) {
                                    Ok(plaintext) => {
                                        if let Err(e) = file.write_all(&plaintext).await {
                                            bar.abandon_with_message(format!(
                                                "[ ERROR ] : Failed to write to disk: {}",
                                                e
                                            ));
                                            upload_successful = false;
                                        }
                                    }
                                    Err(e) => {
                                        bar.abandon_with_message(format!(
                                            "[ ERROR ] : Final decryption failed: {}",
                                            e
                                        ));
                                        upload_successful = false;
                                    }
                                }
                            }
                        }
                        break;
                    }
                    Err(e) => {
                        bar.abandon_with_message(format!(
                            "[ ERROR ] : Network connection lost: {}",
                            e
                        ));
                        upload_successful = false;
                        break;
                    }
                }
            }
        } else {
            loop {
                match field.chunk().await {
                    Ok(Some(chunk)) => {
                        if let Err(e) = file.write_all(&chunk).await {
                            bar.abandon_with_message(format!(
                                "[ ERROR ] : Failed to write to disk: {}",
                                e
                            ));
                            upload_successful = false;
                            break;
                        }

                        bar.inc(chunk.len() as u64);
                    }
                    Ok(None) => {
                        break;
                    }
                    Err(e) => {
                        bar.abandon_with_message(format!(
                            "[ ERROR ] : Network connection lost: {}",
                            e
                        ));
                        upload_successful = false;
                        break;
                    }
                }
            }
        }

        file.flush().await.ok();

        if !upload_successful {
            drop(file);
            let _ = fs::remove_file(&target_path).await;
            return Err(anyhow::anyhow!("Upload interrupted").into());
        }

        if is_encrypted {
            bar.finish_with_message(format!(
                "\"{}\" received and decrypted successfully",
                file_name
            ));
        } else {
            bar.finish_with_message(format!("\"{}\" received successfully", file_name));
        }
    }

    token.cancel();

    let html = "<html lang=\"en\">
    <head>
      <meta charset=\"UTF-8\" />
      <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />
      <title>Upload Status</title>
      <style>body { font-family: sans-serif; margin: 3rem; }</style>
    </head>
    <body>
      <h1>upload complete</h1>
    </body>
    </html>";

    let response = Response::builder()
        .header(header::CONTENT_TYPE, "text/html")
        .body(Body::from(html))
        .context("Failed to build HTML response for post_upload")?;

    Ok(response)
}

async fn stream_file(
    path: Arc<PathBuf>,
    enc_key: Option<Arc<[u8; 32]>>,
) -> Result<AxumResponse, AppError> {
    let content_length = match tokio::fs::metadata(&*path).await {
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((StatusCode::NOT_FOUND, "File not found").into_response());
        }
        Err(e) => return Err(anyhow::anyhow!("Failed to read file metadata: {}", e).into()),
    };

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");

    if let Some(key) = enc_key {
        let enc_size = crypto::encrypted_size(content_length);
        println!(
            "[ INFO ] : Serving file of size {} (Encrypted size: {})",
            format_size(content_length),
            format_size(enc_size)
        );
        println!("[ CRYPTO ] : Encrypting file stream with AES-256-GCM");

        let mut file = File::open(&*path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("File not found")
            } else {
                anyhow::anyhow!("Failed to open file: {}", e)
            }
        })?;

        let chunk_size = crypto::StreamEncryptor::chunk_size();
        let mut encryptor = crypto::StreamEncryptor::new(&key);
        let nonce_bytes = encryptor.nonce_bytes().to_vec();

        let pb = ProgressBar::new(content_length);
        pb.set_style(
            indicatif::ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta}) \"{msg}\"")
                .unwrap()
                .progress_chars("#>-"),
        );

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(4);

        tokio::spawn(async move {
            if tx.send(Ok(nonce_bytes)).await.is_err() {
                return;
            }

            let mut buf = vec![0u8; chunk_size];
            let mut total_read = 0u64;

            loop {
                let mut bytes_in_chunk = 0;
                while bytes_in_chunk < chunk_size {
                    match file.read(&mut buf[bytes_in_chunk..]).await {
                        Ok(0) => break,
                        Ok(n) => {
                            bytes_in_chunk += n;
                            total_read += n as u64;
                            pb.set_position(total_read);
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e)).await;
                            return;
                        }
                    }
                }

                if bytes_in_chunk == 0 {
                    match encryptor.encrypt_last(b"") {
                        Ok(ct) => {
                            let _ = tx.send(Ok(ct)).await;
                        }
                        Err(_) => {
                            let _ = tx
                                .send(Err(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    "encryption failed",
                                )))
                                .await;
                        }
                    }
                    break;
                }

                let is_eof = bytes_in_chunk < chunk_size;
                let plaintext = &buf[..bytes_in_chunk];

                if is_eof {
                    // Last chunk
                    match encryptor.encrypt_last(plaintext) {
                        Ok(ct) => {
                            let _ = tx.send(Ok(ct)).await;
                        }
                        Err(_) => {
                            let _ = tx
                                .send(Err(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    "encryption failed",
                                )))
                                .await;
                        }
                    }
                    break;
                } else {
                    // Intermediate chunk
                    match encryptor.encrypt_next(plaintext) {
                        Ok(ct) => {
                            if tx.send(Ok(ct)).await.is_err() {
                                return;
                            }
                        }
                        Err(_) => {
                            let _ = tx
                                .send(Err(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    "encryption failed",
                                )))
                                .await;
                            return;
                        }
                    }
                }
            }
            pb.finish();
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let body = Body::from_stream(stream);

        let content_disposition = format!("attachment; filename=\"{}\"", file_name);

        let enc_size = crypto::encrypted_size(content_length);
        let final_response = Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, enc_size)
            .header(header::CONTENT_DISPOSITION, content_disposition)
            .header("X-Drop-Encrypted", "true")
            .body(body)
            .context("Failed to construct encrypted response body")?
            .into_response();

        Ok(final_response)
    } else {
        let file = match File::open(&*path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((StatusCode::NOT_FOUND, "File not found").into_response());
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to open file: {}", e).into());
            }
        };

        let pb = ProgressBar::new(content_length);
        pb.set_style(
            indicatif::ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta}) \"{msg}\"")
                .unwrap()
                .progress_chars("#>-"),
        );
        let wrapped_file = pb.wrap_async_read(file);

        let content_disposition = format!("attachment; filename=\"{}\"", file_name);
        let stream = ReaderStream::with_capacity(wrapped_file, 64 * 1024);
        let body = Body::from_stream(stream);

        let mut response = Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_DISPOSITION, content_disposition);

        if content_length > 0 {
            response = response.header(header::CONTENT_LENGTH, content_length);
        }

        let final_response = response
            .body(body)
            .context("Failed to construct response body")?
            .into_response();

        Ok(final_response)
    }
}

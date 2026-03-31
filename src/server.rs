use crate::utils::format_size;
use anyhow::Context;
use axum::{
    body::Body,
    extract::Multipart,
    http::{header, Response, StatusCode},
    response::{IntoResponse, Response as AxumResponse},
    Extension,
};
use indicatif::ProgressBar;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    fs::{self, File},
    io::AsyncWriteExt,
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

pub async fn download(
    Extension(file_path): Extension<Arc<PathBuf>>,
    Extension(token): Extension<CancellationToken>,
) -> Result<impl IntoResponse, AppError> {
    let res = stream_file(file_path).await?;
    token.cancel();
    Ok(res)
}

pub async fn get_upload() -> Result<impl IntoResponse, AppError> {
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

    let response = Response::builder()
        .header(header::CONTENT_TYPE, "text/html")
        .body(Body::from(html))
        .context("Failed to build HTML response for get_upload")?;

    Ok(response)
}

pub async fn post_upload(
    Extension(token): Extension<CancellationToken>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    loop {
        let mut field = match multipart.next_field().await.context("Failed to read multipart field")? {
            Some(f) => f,
            None => break,
        };

        let original_file_name = field.file_name().unwrap_or("file").to_string();
        let mut file_name = original_file_name.clone();

        let bar = ProgressBar::new_spinner();
        bar.set_style(
            indicatif::ProgressStyle::default_spinner()
                .template(
                    "{spinner:.green} [{elapsed_precise}] Receiving \"{msg}\" — {bytes} ({bytes_per_sec})",
                )
                .context("Failed to set progress bar template")?,
        );
        bar.set_message(file_name.clone());
        bar.enable_steady_tick(std::time::Duration::from_millis(100));

        let mut counter = 1;
        let (base_name, ext) = match original_file_name.rsplit_once('.') {
            Some((n, e)) => (n, format!(".{}", e)),
            None => (original_file_name.as_str(), String::new()),
        };

        while Path::new(&file_name).exists() {
            file_name = format!("{}({}){}", base_name, counter, ext);
            counter += 1;
        }

        let mut file = match fs::File::create(&file_name).await {
            Ok(f) => f,
            Err(e) => {
                bar.abandon_with_message(format!(
                    "[ ERROR ] : Could not create file on disk: {}",
                    e
                ));
                return Err(anyhow::anyhow!("Disk error: {}", e).into());
            }
        };

        let mut upload_successful = true;

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
                    bar.abandon_with_message(format!("[ ERROR ] : Network connection lost: {}", e));
                    upload_successful = false;
                    break;
                }
            }
        }

        if !upload_successful {
            drop(file);
            let _ = fs::remove_file(&file_name).await;
            return Err(anyhow::anyhow!("Upload interrupted").into());
        }

        bar.finish_with_message(format!("\"{}\" received successfully", file_name));
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

async fn stream_file(path: Arc<PathBuf>) -> Result<AxumResponse, AppError> {
    let file = File::open(&*path)
        .await
        .context(format!("File not found: {}", path.display()))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");

    let content_length = tokio::fs::metadata(&*path)
        .await
        .context("Failed to read file metadata")?
        .len();

    println!(
        "[ INFO ] : Serving file of size {}",
        format_size(content_length)
    );

    let pb = ProgressBar::new(content_length);
    let wrapped_file = pb.wrap_async_read(file);

    let content_disposition = format!("attachment; filename=\"{}\"", file_name);
    let stream = ReaderStream::new(wrapped_file);
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

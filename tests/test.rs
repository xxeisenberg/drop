use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Extension, Router,
};
use reqwest::{Client, StatusCode};
use std::{io::Write, path::PathBuf, sync::Arc};
use tempfile::NamedTempFile;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[allow(dead_code)]
#[path = "../src/server.rs"]
mod server;

#[allow(dead_code)]
#[path = "../src/utils.rs"]
mod utils;

async fn spawn_test_server(app: Router) -> String {
    // Binding to port 0 tells the OS to assign a random available port.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let address = format!("http://127.0.0.1:{}", port);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    address
}

// ----------------------------------------------------------------------
// 1. TEST: The Upload Web UI (GET /)
// ----------------------------------------------------------------------
#[tokio::test]
async fn test_server_get_upload_ui() {
    let token = CancellationToken::new();

    let app = Router::new()
        .route("/", get(server::get_upload))
        .layer(Extension(token));

    let base_url = spawn_test_server(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{}/", base_url))
        .send()
        .await
        .expect("Failed to execute request");

    assert!(res.status().is_success(), "Expected 200 OK");
    assert_eq!(
        res.headers().get("content-type").unwrap(),
        "text/html",
        "Expected HTML content type"
    );

    let body_text = res.text().await.unwrap();
    assert!(
        body_text.contains("<title>File Upload</title>"),
        "HTML should contain the correct title"
    );
    assert!(
        body_text.contains("<form action=\"/upload\""),
        "HTML should contain the upload form"
    );
}

// ----------------------------------------------------------------------
// 2. TEST: Successful File Download (GET /download)
// ----------------------------------------------------------------------
#[tokio::test]
async fn test_server_download_flow() {
    let mut temp_file = NamedTempFile::new().unwrap();
    let file_content = b"Hello, RustShare! This is a test download.";
    temp_file.write_all(file_content).unwrap();

    let file_path = Arc::new(PathBuf::from(temp_file.path()));
    let token = CancellationToken::new();

    let app = Router::new()
        .route("/download", get(server::download))
        .layer(Extension(file_path.clone()))
        .layer(Extension(token));

    let base_url = spawn_test_server(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{}/download", base_url))
        .send()
        .await
        .expect("Failed to execute request");

    assert!(res.status().is_success(), "Expected 200 OK");

    let downloaded_bytes = res.bytes().await.unwrap();
    assert_eq!(downloaded_bytes.as_ref(), file_content);
}

// ----------------------------------------------------------------------
// 3. TEST: Missing File Download (GET /download)
// ----------------------------------------------------------------------
#[tokio::test]
async fn test_server_download_not_found() {
    // Point the server to a file that definitely does not exist
    let missing_file_path = Arc::new(PathBuf::from("does_not_exist_12345.txt"));
    let token = CancellationToken::new();

    let app = Router::new()
        .route("/download", get(server::download))
        .layer(Extension(missing_file_path))
        .layer(Extension(token));

    let base_url = spawn_test_server(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{}/download", base_url))
        .send()
        .await
        .expect("Failed to execute request");

    assert_eq!(
        res.status(),
        StatusCode::NOT_FOUND,
        "Expected a 404 NOT FOUND for missing files"
    );
}

// ----------------------------------------------------------------------
// 4. TEST: Successful Multipart Upload (POST /upload)
// ----------------------------------------------------------------------
#[tokio::test]
async fn test_server_upload_flow() {
    let temp_dir = tempfile::tempdir().unwrap();

    let token = CancellationToken::new();
    let upload_dir = Arc::new(temp_dir.path().to_path_buf());

    let app = Router::new()
        .route("/upload", post(server::post_upload))
        .layer(DefaultBodyLimit::disable())
        .layer(Extension(token))
        .layer(Extension(upload_dir));

    let base_url = spawn_test_server(app).await;

    let file_content = "File content to be uploaded";
    let part = reqwest::multipart::Part::bytes(file_content.as_bytes().to_vec())
        .file_name("test_upload.txt");
    let form = reqwest::multipart::Form::new().part("uploadedFile", part);

    let client = Client::new();
    let res = client
        .post(format!("{}/upload", base_url))
        .multipart(form)
        .send()
        .await
        .expect("Failed to execute request");

    assert!(res.status().is_success(), "Expected upload to succeed");

    let saved_file_path = temp_dir.path().join("test_upload.txt");
    assert!(saved_file_path.exists(), "File was not saved to disk");

    let saved_content = std::fs::read_to_string(saved_file_path).unwrap();
    assert_eq!(saved_content, file_content);
}

// ----------------------------------------------------------------------
// 5. TEST: Utility Data Formatting
// ----------------------------------------------------------------------
#[test]
fn test_utils_format_size() {
    // Test standard bytes
    assert_eq!(utils::format_size(500), "500 bytes");

    // Test Kilobytes (1000 bytes)
    assert_eq!(utils::format_size(1_500), "1.50 Kilobytes");

    // Test Megabytes (1,000,000 bytes)
    assert_eq!(utils::format_size(2_750_000), "2.75 Megabytes");

    // Test Gigabytes (1,000,000,000 bytes)
    assert_eq!(utils::format_size(4_120_000_000), "4.12 Gigabytes");
}

// ----------------------------------------------------------------------
// 6. TEST: Duplicate File Naming Logic
// ----------------------------------------------------------------------
#[tokio::test]
async fn test_server_upload_duplicate_names() {
    let temp_dir = tempfile::tempdir().unwrap();
    let upload_dir = Arc::new(temp_dir.path().to_path_buf());

    let token = CancellationToken::new();
    let app = Router::new()
        .route("/upload", post(server::post_upload))
        .layer(DefaultBodyLimit::disable())
        .layer(Extension(token))
        .layer(Extension(upload_dir));

    let base_url = spawn_test_server(app).await;
    let client = Client::new();

    let file_content = "Some data";

    // Helper closure to build a fresh multipart form
    let create_form = || {
        let part = reqwest::multipart::Part::bytes(file_content.as_bytes().to_vec())
            .file_name("duplicate_test.txt");
        reqwest::multipart::Form::new().part("uploadedFile", part)
    };

    // Upload the file the FIRST time
    client
        .post(format!("{}/upload", base_url))
        .multipart(create_form())
        .send()
        .await
        .unwrap();

    // Upload the EXACT SAME file a SECOND time
    client
        .post(format!("{}/upload", base_url))
        .multipart(create_form())
        .send()
        .await
        .unwrap();

    // Verify BOTH files exist with the correct naming scheme
    let original_file = temp_dir.path().join("duplicate_test.txt");
    let duplicate_file = temp_dir.path().join("duplicate_test(1).txt");

    assert!(original_file.exists(), "Original file should exist");
    assert!(duplicate_file.exists(), "Duplicate file should have (1) appended to it");
}

// ----------------------------------------------------------------------
// 7. TEST: Invalid/Malformed Upload Data
// ----------------------------------------------------------------------
#[tokio::test]
async fn test_server_upload_invalid_data() {
    let token = CancellationToken::new();
    let temp_dir = tempfile::tempdir().unwrap();
    let upload_dir = Arc::new(temp_dir.path().to_path_buf());

    let app = Router::new()
        .route("/upload", post(server::post_upload))
        .layer(DefaultBodyLimit::disable())
        .layer(Extension(token))
        .layer(Extension(upload_dir));

    let base_url = spawn_test_server(app).await;
    let client = Client::new();

    // Send a standard plain-text POST request instead of multipart
    let res = client
        .post(format!("{}/upload", base_url))
        .body("This is just a plain string, not a multipart form")
        .header("Content-Type", "text/plain")
        .send()
        .await
        .expect("Failed to execute request");

    // Axum's `Multipart` extractor should automatically reject this.
    // It usually returns a 400 Bad Request or 422 Unprocessable Entity.
    assert!(
        res.status().is_client_error(),
        "Server should reject non-multipart requests with a 4xx error, got: {}",
        res.status()
    );
}

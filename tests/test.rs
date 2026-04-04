use axum::{
    Extension, Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
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

#[allow(dead_code)]
#[path = "../src/crypto.rs"]
mod crypto;

#[allow(dead_code)]
#[path = "../src/cli.rs"]
mod cli;

#[allow(dead_code)]
#[path = "../src/discovery.rs"]
mod discovery;

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
        .layer(Extension(Some(Arc::new("dummy_token".to_string()))))
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
        body_text.contains("<form action=\"/upload?token="),
        "HTML should contain the upload form"
    );
}

#[tokio::test]
async fn test_server_get_upload_ui_without_token() {
    let token = CancellationToken::new();

    let app = Router::new()
        .route("/", get(server::get_upload))
        .layer(Extension(None::<Arc<String>>))
        .layer(Extension(token));

    let base_url = spawn_test_server(app).await;
    let client = Client::new();

    let res = client.get(format!("{}/", base_url)).send().await.unwrap();

    assert!(res.status().is_success(), "Expected 200 OK");

    let body_text = res.text().await.unwrap();
    assert!(
        body_text.contains("<form action=\"/upload\""),
        "HTML should contain a plain upload form action when tokens are disabled"
    );
    assert!(
        !body_text.contains("token="),
        "HTML should not embed a token when tokens are disabled"
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
        .layer(Extension(token))
        .layer(Extension(None as Option<Arc<[u8; 32]>>));

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
        .layer(Extension(token))
        .layer(Extension(None as Option<Arc<[u8; 32]>>));

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
        .layer(Extension(upload_dir))
        .layer(Extension(None as Option<Arc<[u8; 32]>>));

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

#[test]
fn test_utils_with_optional_token() {
    assert_eq!(
        utils::with_optional_token("http://127.0.0.1:1844/download", Some("secret")),
        "http://127.0.0.1:1844/download?token=secret"
    );
    assert_eq!(
        utils::with_optional_token("http://127.0.0.1:1844/download", None),
        "http://127.0.0.1:1844/download"
    );
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
        .layer(Extension(upload_dir))
        .layer(Extension(None as Option<Arc<[u8; 32]>>));

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
    assert!(
        duplicate_file.exists(),
        "Duplicate file should have (1) appended to it"
    );
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
        .layer(Extension(upload_dir))
        .layer(Extension(None as Option<Arc<[u8; 32]>>));

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

// ----------------------------------------------------------------------
// 8. TEST: Crypto
// ----------------------------------------------------------------------
#[test]
fn test_crypto_roundtrip_single_chunk() {
    let key = crypto::generate_key();
    let data = b"Hello, world! This is a test of streaming encryption.";

    let enc = crypto::StreamEncryptor::new(&key);
    let nonce = *enc.nonce_bytes();
    let ciphertext = enc.encrypt_last(data).unwrap();

    let dec = crypto::StreamDecryptor::new(&key, &nonce);
    let plaintext = dec.decrypt_last(&ciphertext).unwrap();

    assert_eq!(data.as_slice(), plaintext.as_slice());
}

#[test]
fn test_crypto_roundtrip_multi_chunk() {
    let key = crypto::generate_key();
    let chunk_size = crypto::StreamEncryptor::chunk_size();
    // Create data larger than chunk_size
    let data: Vec<u8> = (0..chunk_size * 3 + 1234)
        .map(|i| (i % 256) as u8)
        .collect();

    let mut enc = crypto::StreamEncryptor::new(&key);
    let nonce = *enc.nonce_bytes();
    let mut ciphertext_chunks = Vec::new();

    let chunks: Vec<&[u8]> = data.chunks(chunk_size).collect();
    let last_idx = chunks.len() - 1;

    // Encrypt intermediate chunks
    for chunk in &chunks[..last_idx] {
        ciphertext_chunks.push(enc.encrypt_next(chunk).unwrap());
    }
    // Encrypt the final chunk (consumes enc)
    ciphertext_chunks.push(enc.encrypt_last(chunks[last_idx]).unwrap());

    let mut dec = crypto::StreamDecryptor::new(&key, &nonce);
    let mut decrypted = Vec::new();
    let last_ct_idx = ciphertext_chunks.len() - 1;

    // Decrypt intermediate chunks
    for chunk in &ciphertext_chunks[..last_ct_idx] {
        decrypted.extend_from_slice(&dec.decrypt_next(chunk).unwrap());
    }
    // Decrypt the final chunk (consumes dec)
    decrypted.extend_from_slice(&dec.decrypt_last(&ciphertext_chunks[last_ct_idx]).unwrap());

    assert_eq!(data, decrypted);
}

#[test]
fn test_crypto_wrong_key_fails() {
    let key1 = crypto::generate_key();
    let key2 = crypto::generate_key();
    let data = b"secret data";

    let enc = crypto::StreamEncryptor::new(&key1);
    let nonce = *enc.nonce_bytes();
    let ciphertext = enc.encrypt_last(data).unwrap();

    let dec = crypto::StreamDecryptor::new(&key2, &nonce);
    assert!(dec.decrypt_last(&ciphertext).is_err());
}

#[test]
fn test_crypto_tampered_data_fails() {
    let key = crypto::generate_key();
    let data = b"important data";

    let enc = crypto::StreamEncryptor::new(&key);
    let nonce = *enc.nonce_bytes();
    let mut ciphertext = enc.encrypt_last(data).unwrap();

    // Tamper with a byte
    if let Some(byte) = ciphertext.get_mut(5) {
        *byte ^= 0xFF;
    }

    let dec = crypto::StreamDecryptor::new(&key, &nonce);
    assert!(dec.decrypt_last(&ciphertext).is_err());
}

#[test]
fn test_crypto_key_encode_decode_roundtrip() {
    let key = crypto::generate_key();
    let encoded = crypto::encode_key(&key);
    let decoded = crypto::decode_key(&encoded).unwrap();
    assert_eq!(key, decoded);
}

// ----------------------------------------------------------------------
// 9. TEST: CLI Argument Parsing
// ----------------------------------------------------------------------
#[test]
fn test_cli_parsing() {
    use clap::Parser;

    // Test default port and mode
    let cli = cli::Cli::try_parse_from(&["drop", "receive"]).unwrap();
    match cli.command {
        cli::Commands::Receive { port, encrypt, .. } => {
            assert_eq!(port, 1844);
            assert!(!encrypt);
        }
        _ => panic!("Expected Receive subcommand"),
    }

    // Test specific port
    let cli = cli::Cli::try_parse_from(&["drop", "receive", "--port", "2000"]).unwrap();
    match cli.command {
        cli::Commands::Receive { port, .. } => assert_eq!(port, 2000),
        _ => panic!("Expected Receive subcommand"),
    }

    // Test encrypt flag
    let cli = cli::Cli::try_parse_from(&["drop", "receive", "--encrypt"]).unwrap();
    match cli.command {
        cli::Commands::Receive { encrypt, .. } => assert!(encrypt),
        _ => panic!("Expected Receive subcommand"),
    }

    let cli = cli::Cli::try_parse_from(&["drop", "receive", "--no-link-token"]).unwrap();
    match cli.command {
        cli::Commands::Receive { no_link_token, .. } => assert!(no_link_token),
        _ => panic!("Expected Receive subcommand"),
    }

    // Test send subcommand
    let cli = cli::Cli::try_parse_from(&["drop", "send", "my_file.txt"]).unwrap();
    match cli.command {
        cli::Commands::Send {
            file_path,
            port,
            encrypt,
            no_link_token,
        } => {
            assert_eq!(file_path.to_str().unwrap(), "my_file.txt");
            assert_eq!(port, 1844);
            assert!(!encrypt);
            assert!(!no_link_token);
        }
        _ => panic!("Expected Send subcommand"),
    }

    let cli =
        cli::Cli::try_parse_from(&["drop", "send", "my_file.txt", "--no-link-token"]).unwrap();
    match cli.command {
        cli::Commands::Send { no_link_token, .. } => assert!(no_link_token),
        _ => panic!("Expected Send subcommand"),
    }
}

// ----------------------------------------------------------------------
// 10. TEST: Discovery Name Logic
// ----------------------------------------------------------------------
#[test]
fn test_discovery_name_logic() {
    let (instance, host) = discovery::get_mdns_names("receive");

    // The instance name should contain the mode
    assert!(instance.contains("receive"));

    // Hostname should end with .local. and have no spaces
    assert!(host.ends_with(".local."));
    assert!(!host.contains(' '));

    // Test extreme length (placeholder name as whoami might return anything)
    // The code handles truncation internally.
}

// ----------------------------------------------------------------------
// 11. TEST: Server Filename Sanitization
// ----------------------------------------------------------------------
#[test]
fn test_server_filename_sanitization() {
    assert_eq!(server::sanitize_filename("normal.txt"), "normal.txt");
    assert_eq!(server::sanitize_filename("../../etc/passwd"), "passwd");
    assert_eq!(server::sanitize_filename("path/to/file.png"), "file.png");
    assert_eq!(server::sanitize_filename(".."), "file");
    assert_eq!(server::sanitize_filename("."), "file");
    assert_eq!(server::sanitize_filename(""), "file");
}

// ----------------------------------------------------------------------
// 12. TEST: Server Token Validation Middleware
// ----------------------------------------------------------------------
#[tokio::test]
async fn test_server_token_validation() {
    let expected_token = Arc::new("secret-token".to_string());
    let token = CancellationToken::new();

    let app = Router::new()
        .route("/protected", get(|| async { "Success" }))
        .layer(axum::middleware::from_fn(server::validate_token))
        .layer(Extension(Some(expected_token.clone())))
        .layer(Extension(token));

    let base_url = spawn_test_server(app).await;
    let client = Client::new();

    // 1. Missing token
    let res = client
        .get(format!("{}/protected", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 2. Wrong token
    let res = client
        .get(format!("{}/protected?token=wrong", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 3. Correct token
    let res = client
        .get(format!("{}/protected?token={}", base_url, expected_token))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.text().await.unwrap(), "Success");
}

#[tokio::test]
async fn test_server_token_validation_can_be_disabled() {
    let token = CancellationToken::new();

    let app = Router::new()
        .route("/protected", get(|| async { "Success" }))
        .layer(axum::middleware::from_fn(server::validate_token))
        .layer(Extension(None::<Arc<String>>))
        .layer(Extension(token));

    let base_url = spawn_test_server(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{}/protected", base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.text().await.unwrap(), "Success");
}

// ----------------------------------------------------------------------
// 13. TEST: Encrypted Server Download Roundtrip
// ----------------------------------------------------------------------
#[tokio::test]
async fn test_server_encrypted_download_roundtrip() {
    let mut temp_file = NamedTempFile::new().unwrap();
    let file_content = b"Encrypted data content";
    temp_file.write_all(file_content).unwrap();

    let file_path = Arc::new(PathBuf::from(temp_file.path()));
    let token = CancellationToken::new();
    let key = Arc::new(crypto::generate_key());

    let app = Router::new()
        .route("/download", get(server::download))
        .layer(Extension(file_path))
        .layer(Extension(token))
        .layer(Extension(Some(key.clone())))
        .layer(Extension(Arc::new("dummy-token".to_string())));

    let base_url = spawn_test_server(app).await;
    let client = Client::new();

    let res = client
        .get(format!("{}/download?token=dummy-token", base_url))
        .header("X-Drop-Encrypted", "true")
        .send()
        .await
        .unwrap();

    assert!(res.status().is_success());
    assert_eq!(res.headers().get("X-Drop-Encrypted").unwrap(), "true");

    let bytes = res.bytes().await.unwrap();
    let nonce_size = crypto::StreamDecryptor::nonce_size();
    let nonce = &bytes[..nonce_size];
    let ciphertext = &bytes[nonce_size..];

    let decryptor = crypto::StreamDecryptor::new(&key, nonce.try_into().unwrap());
    let decrypted = decryptor.decrypt_last(ciphertext).unwrap();

    assert_eq!(decrypted, file_content);
}

// ----------------------------------------------------------------------
// 14. TEST: Server Token Cancellation on Completion
// ----------------------------------------------------------------------
#[tokio::test]
async fn test_server_cancellation_on_download() {
    let mut temp_file = NamedTempFile::new().unwrap();
    temp_file.write_all(b"data").unwrap();

    let file_path = Arc::new(PathBuf::from(temp_file.path()));
    let token = CancellationToken::new();

    let app = Router::new()
        .route("/download", get(server::download))
        .layer(Extension(file_path))
        .layer(Extension(token.clone()))
        .layer(Extension(None as Option<Arc<[u8; 32]>>))
        .layer(Extension(Arc::new("token".to_string())));

    let base_url = spawn_test_server(app).await;
    let client = Client::new();

    assert!(!token.is_cancelled());

    let res = client
        .get(format!("{}/download?token=token", base_url))
        .send()
        .await
        .unwrap();

    assert!(res.status().is_success());
    // Consume the body to ensure the request finishes
    let _ = res.bytes().await.unwrap();

    // The handler calls token.cancel() after streaming
    assert!(token.is_cancelled());
}

use anyhow::{Context, Result, bail};
use axum::Router;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HyperBuilder;
use rcgen::{CertificateParams, DnType, KeyPair, SanType};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, ServerConfig, SignatureScheme,
};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct HttpsConfig {
    cert_chain: Vec<CertificateDer<'static>>,
    private_key: PrivateKeyDer<'static>,
    fingerprint: String,
    generated: bool,
}

impl HttpsConfig {
    pub fn load_or_generate(
        local_ip: IpAddr,
        mdns_host_name: Option<&str>,
        cert_path: Option<&Path>,
        key_path: Option<&Path>,
    ) -> Result<Self> {
        match (cert_path, key_path) {
            (Some(cert_path), Some(key_path)) => Self::from_pem_files(cert_path, key_path),
            _ => Self::generate(local_ip, mdns_host_name),
        }
    }

    fn from_pem_files(cert_path: &Path, key_path: &Path) -> Result<Self> {
        let cert_pem = std::fs::read(cert_path).with_context(|| {
            format!(
                "Failed to read TLS certificate from {}",
                cert_path.display()
            )
        })?;
        let key_pem = std::fs::read(key_path).with_context(|| {
            format!("Failed to read TLS private key from {}", key_path.display())
        })?;

        let cert_chain: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut &cert_pem[..])
            .collect::<std::result::Result<_, _>>()
            .context("Failed to parse TLS certificate PEM")?;

        if cert_chain.is_empty() {
            bail!("TLS certificate file did not contain any certificates");
        }

        let private_key = load_private_key(&key_pem)?;
        Ok(Self {
            fingerprint: fingerprint_hex(cert_chain[0].as_ref()),
            cert_chain,
            private_key,
            generated: false,
        })
    }

    fn generate(local_ip: IpAddr, mdns_host_name: Option<&str>) -> Result<Self> {
        let mut params = CertificateParams::new(vec!["localhost".to_string()])
            .context("Failed to initialize self-signed certificate parameters")?;
        params.subject_alt_names.push(SanType::IpAddress(local_ip));
        if let Some(host_name) = mdns_host_name.and_then(normalize_mdns_san_name) {
            params.subject_alt_names.push(SanType::DnsName(
                host_name
                    .try_into()
                    .context("Failed to encode mDNS hostname into certificate SAN")?,
            ));
        }
        params
            .distinguished_name
            .push(DnType::CommonName, "drop self-signed");

        let key_pair = KeyPair::generate().context("Failed to generate self-signed private key")?;
        let cert = params
            .self_signed(&key_pair)
            .context("Failed to generate self-signed certificate")?;
        let cert_der: CertificateDer<'static> = cert.der().clone();
        let private_key = PrivateKeyDer::Pkcs8(key_pair.serialize_der().into());

        Ok(Self {
            fingerprint: fingerprint_hex(cert_der.as_ref()),
            cert_chain: vec![cert_der],
            private_key,
            generated: true,
        })
    }

    pub fn rustls_server_config(&self) -> Result<Arc<ServerConfig>> {
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(self.cert_chain.clone(), self.private_key.clone_key())
            .context("Failed to build Rustls server config")?;
        Ok(Arc::new(config))
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn is_generated(&self) -> bool {
        self.generated
    }
}

fn load_private_key(key_pem: &[u8]) -> Result<PrivateKeyDer<'static>> {
    let mut reader = &key_pem[..];
    let mut pkcs8_keys = rustls_pemfile::pkcs8_private_keys(&mut reader);
    if let Some(key) = pkcs8_keys.next() {
        return Ok(PrivateKeyDer::Pkcs8(
            key.context("Failed to parse PKCS#8 private key")?,
        ));
    }

    let mut reader = &key_pem[..];
    let mut rsa_keys = rustls_pemfile::rsa_private_keys(&mut reader);
    if let Some(key) = rsa_keys.next() {
        return Ok(PrivateKeyDer::Pkcs1(
            key.context("Failed to parse RSA private key")?,
        ));
    }

    let mut reader = &key_pem[..];
    let mut sec1_keys = rustls_pemfile::ec_private_keys(&mut reader);
    if let Some(key) = sec1_keys.next() {
        return Ok(PrivateKeyDer::Sec1(
            key.context("Failed to parse SEC1 private key")?,
        ));
    }

    bail!("TLS private key file did not contain a supported key")
}

pub fn build_pinned_https_client(expected_fingerprint: &str) -> Result<reqwest::Client> {
    let verifier = Arc::new(FingerprintVerifier::new(expected_fingerprint)?);
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();

    reqwest::Client::builder()
        .use_preconfigured_tls(config)
        .build()
        .context("Failed to build HTTPS client")
}

pub async fn serve_https(
    addr: std::net::SocketAddr,
    app: Router,
    config: Arc<ServerConfig>,
    shutdown_token: CancellationToken,
) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind TLS listener on {}", addr))?;
    let acceptor = tokio_rustls::TlsAcceptor::from(config);

    loop {
        tokio::select! {
            biased;
            _ = shutdown_token.cancelled() => break,
            accept_result = listener.accept() => {
                let (stream, _) = accept_result.context("Failed to accept TLS connection")?;
                let acceptor = acceptor.clone();
                let app = app.clone();

                tokio::spawn(async move {
                    let tls_stream = match acceptor.accept(stream).await {
                        Ok(stream) => stream,
                        Err(_) => return,
                    };

                    let service = service_fn(move |request| {
                        let mut app = app.clone();
                        async move {
                            use tower_service::Service;
                            app.call(request).await
                        }
                    });

                    let io = TokioIo::new(tls_stream);
                    let builder = HyperBuilder::new(TokioExecutor::new());
                    let connection = builder.serve_connection_with_upgrades(io, service);
                    let _ = connection.await;
                });
            }
        }
    }

    Ok(())
}

pub fn fingerprint_hex(cert_der: &[u8]) -> String {
    hex::encode(Sha256::digest(cert_der))
}

pub fn normalize_mdns_san_name(host_name: &str) -> Option<String> {
    let trimmed = host_name.trim_end_matches('.');
    if trimmed.is_empty() {
        return None;
    }

    let valid = trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.');
    valid.then(|| trimmed.to_string())
}

#[derive(Debug)]
struct FingerprintVerifier {
    expected_fingerprint: String,
}

impl FingerprintVerifier {
    fn new(expected_fingerprint: &str) -> Result<Self> {
        if expected_fingerprint.is_empty() {
            bail!("Expected TLS fingerprint cannot be empty");
        }

        Ok(Self {
            expected_fingerprint: expected_fingerprint.to_ascii_lowercase(),
        })
    }

    fn verify_fingerprint(
        &self,
        end_entity: &CertificateDer<'_>,
    ) -> Result<ServerCertVerified, RustlsError> {
        let actual_fingerprint = fingerprint_hex(end_entity.as_ref());
        if actual_fingerprint == self.expected_fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(RustlsError::General(format!(
                "Server certificate fingerprint mismatch (expected {}, got {})",
                self.expected_fingerprint, actual_fingerprint
            )))
        }
    }
}

impl ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, RustlsError> {
        self.verify_fingerprint(end_entity)
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

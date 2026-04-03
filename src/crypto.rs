use aead::stream::{DecryptorBE32, EncryptorBE32, StreamBE32};
use aes_gcm::Aes256Gcm;
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngExt;

const CHUNK_SIZE: usize = 65536; // 64 KB plaintext chunks
const TAG_SIZE: usize = 16; // GCM auth tag overhead
pub const ENCRYPTED_CHUNK_SIZE: usize = CHUNK_SIZE + TAG_SIZE;
const STREAM_NONCE_SIZE: usize = 7; // AEAD STREAM nonce size

pub fn encrypted_size(content_length: u64) -> u64 {
    let num_chunks = (content_length / CHUNK_SIZE as u64) + 1;
    STREAM_NONCE_SIZE as u64 + content_length + (num_chunks * TAG_SIZE as u64)
}

pub fn generate_key() -> [u8; 32] {
    rand::rng().random()
}

pub fn encode_key(key: &[u8; 32]) -> String {
    URL_SAFE_NO_PAD.encode(key)
}

pub fn decode_key(encoded: &str) -> Result<[u8; 32]> {
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .context("Failed to base64-decode encryption key")?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("Expected 32-byte key, got {} bytes", v.len()))
}

// Streaming encryptor prevents reordering and truncation attacks
pub struct StreamEncryptor {
    inner: EncryptorBE32<Aes256Gcm>,
    nonce_bytes: [u8; STREAM_NONCE_SIZE],
}

impl StreamEncryptor {
    pub fn new(key: &[u8; 32]) -> Self {
        let nonce_bytes: [u8; STREAM_NONCE_SIZE] = rand::rng().random();
        let aes_key = aes_gcm::Key::<Aes256Gcm>::from_slice(key);
        let nonce =
            aead::stream::Nonce::<Aes256Gcm, StreamBE32<Aes256Gcm>>::from_slice(&nonce_bytes);
        let inner = EncryptorBE32::new(aes_key, nonce);
        Self { inner, nonce_bytes }
    }

    pub fn nonce_bytes(&self) -> &[u8; STREAM_NONCE_SIZE] {
        &self.nonce_bytes
    }

    pub fn encrypt_next(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        self.inner
            .encrypt_next(plaintext)
            .map_err(|_| anyhow::anyhow!("Encryption failed"))
    }

    pub fn encrypt_last(self, plaintext: &[u8]) -> Result<Vec<u8>> {
        self.inner
            .encrypt_last(plaintext)
            .map_err(|_| anyhow::anyhow!("Final encryption failed"))
    }

    pub fn chunk_size() -> usize {
        CHUNK_SIZE
    }
}

pub struct StreamDecryptor {
    inner: DecryptorBE32<Aes256Gcm>,
}

impl StreamDecryptor {
    pub fn new(key: &[u8; 32], nonce: &[u8; STREAM_NONCE_SIZE]) -> Self {
        let aes_key = aes_gcm::Key::<Aes256Gcm>::from_slice(key);
        let stream_nonce =
            aead::stream::Nonce::<Aes256Gcm, StreamBE32<Aes256Gcm>>::from_slice(nonce);
        let inner = DecryptorBE32::new(aes_key, stream_nonce);
        Self { inner }
    }

    pub fn decrypt_next(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        self.inner
            .decrypt_next(ciphertext)
            .map_err(|_| anyhow::anyhow!("Decryption failed — data may be corrupted or tampered"))
    }

    pub fn decrypt_last(self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        self.inner.decrypt_last(ciphertext).map_err(|_| {
            anyhow::anyhow!("Final decryption failed — data may be corrupted or tampered")
        })
    }

    pub fn encrypted_chunk_size() -> usize {
        ENCRYPTED_CHUNK_SIZE
    }

    pub fn nonce_size() -> usize {
        STREAM_NONCE_SIZE
    }
}

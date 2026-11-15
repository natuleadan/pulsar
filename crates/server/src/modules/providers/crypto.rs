//! Argon2id key derivation + AES-256-GCM encryption for stored provider API keys.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};

use crate::infra::error::ServerError;

const ENVELOPE_PREFIX: &str = "enc:v1:";

fn derive_aes_key(master_key: &str, salt: &[u8; 16]) -> Result<[u8; 32], ServerError> {
    let params = Params::new(19_456, 2, 1, Some(32))
        .map_err(|e| ServerError::Internal(format!("argon2 params failed: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon2
        .hash_password_into(master_key.as_bytes(), salt, &mut out)
        .map_err(|e| ServerError::Internal(format!("argon2id kdf failed: {e}")))?;
    Ok(out)
}

/// Encrypts a plaintext API key using an Argon2id-derived 256-bit key and AES-256-GCM.
pub fn encrypt_api_key(master_key: &str, plaintext: &str) -> Result<String, ServerError> {
    let rng = SystemRandom::new();
    let mut salt = [0u8; 16];
    let mut nonce_bytes = [0u8; 12];
    rng.fill(&mut salt)
        .map_err(|_| ServerError::Internal("csprng salt failed".to_owned()))?;
    rng.fill(&mut nonce_bytes)
        .map_err(|_| ServerError::Internal("csprng nonce failed".to_owned()))?;

    let derived = derive_aes_key(master_key, &salt)?;
    let unbound = UnboundKey::new(&AES_256_GCM, &derived)
        .map_err(|_| ServerError::Internal("invalid aes key".to_owned()))?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut in_out = plaintext.as_bytes().to_vec();
    key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| ServerError::Internal("aes-gcm seal failed".to_owned()))?;

    Ok(format!(
        "{ENVELOPE_PREFIX}{}:{}:{}",
        B64.encode(salt),
        B64.encode(nonce_bytes),
        B64.encode(in_out)
    ))
}

/// Decrypts an `enc:v1:<salt>:<nonce>:<ciphertext>` envelope back to the plaintext API key.
pub fn decrypt_api_key(master_key: &str, envelope: &str) -> Result<String, ServerError> {
    let body = envelope
        .strip_prefix(ENVELOPE_PREFIX)
        .ok_or_else(|| ServerError::Internal("invalid encrypted key envelope".to_owned()))?;
    let mut parts = body.splitn(3, ':');
    let salt_b64 = parts
        .next()
        .ok_or_else(|| ServerError::Internal("missing salt".to_owned()))?;
    let nonce_b64 = parts
        .next()
        .ok_or_else(|| ServerError::Internal("missing nonce".to_owned()))?;
    let ct_b64 = parts
        .next()
        .ok_or_else(|| ServerError::Internal("missing ciphertext".to_owned()))?;

    let salt_vec = B64
        .decode(salt_b64)
        .map_err(|_| ServerError::Internal("invalid salt encoding".to_owned()))?;
    let salt: [u8; 16] = salt_vec
        .try_into()
        .map_err(|_| ServerError::Internal("invalid salt length".to_owned()))?;

    let nonce_vec = B64
        .decode(nonce_b64)
        .map_err(|_| ServerError::Internal("invalid nonce encoding".to_owned()))?;
    let nonce_bytes: [u8; 12] = nonce_vec
        .try_into()
        .map_err(|_| ServerError::Internal("invalid nonce length".to_owned()))?;

    let mut ct = B64
        .decode(ct_b64)
        .map_err(|_| ServerError::Internal("invalid ciphertext encoding".to_owned()))?;

    let derived = derive_aes_key(master_key, &salt)?;
    let unbound = UnboundKey::new(&AES_256_GCM, &derived)
        .map_err(|_| ServerError::Internal("invalid aes key".to_owned()))?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let plain = key
        .open_in_place(nonce, Aad::empty(), &mut ct)
        .map_err(|_| ServerError::Internal("aes-gcm decryption failed".to_owned()))?;

    String::from_utf8(plain.to_vec())
        .map_err(|_| ServerError::Internal("decrypted api key is not utf-8".to_owned()))
}

/// Computes a non-reversible short SHA-256 fingerprint for UI/audit display.
#[must_use]
pub fn key_fingerprint(plaintext: &str) -> String {
    let digest = Sha256::digest(plaintext.as_bytes());
    format!("sha256:{}", &hex::encode(digest)[..12])
}

#[cfg(test)]
mod tests {
    use super::{decrypt_api_key, encrypt_api_key, key_fingerprint};

    #[test]
    fn roundtrip_encrypt_decrypt_api_key() {
        let master = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let secret = "sk-test-provider-key-12345";
        let env = encrypt_api_key(master, secret).expect("encrypt");
        assert!(env.starts_with("enc:v1:"));
        assert!(!env.contains(secret));
        let plain = decrypt_api_key(master, &env).expect("decrypt");
        assert_eq!(plain, secret);
        assert!(key_fingerprint(secret).starts_with("sha256:"));
    }
}

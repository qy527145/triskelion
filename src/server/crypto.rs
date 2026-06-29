//! 凭据加解密：AES-256-GCM，每条密钥独立随机 nonce。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{Result, anyhow};
use rand::RngCore;

/// 加密明文，返回 (nonce[12], ciphertext)。
pub fn encrypt(master_key: &[u8; 32], plaintext: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(master_key));
    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow!("加密失败: {e}"))?;
    Ok((nonce_bytes.to_vec(), ct))
}

/// 解密。
pub fn decrypt(master_key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(master_key));
    let nonce = Nonce::from_slice(nonce);
    let pt = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow!("解密失败: {e}"))?;
    String::from_utf8(pt).map_err(|e| anyhow!("密文非 UTF-8: {e}"))
}

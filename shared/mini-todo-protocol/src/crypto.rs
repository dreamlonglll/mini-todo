//! 可选的客户端端到端加密（E2EE），保护 WebDAV 上存放的 `sync-data.json.gz`。
//!
//! 格式（大端序）：
//! - `0..8`   : 魔数 `MTENC01\0`（用于和明文 gzip `1F 8B` 零歧义区分，保持 100% 向后兼容）
//! - `8..12`  : `u32` PBKDF2-HMAC-SHA256 迭代次数（默认 200,000）
//! - `12..28` : 16 字节随机盐
//! - `28..40` : 12 字节随机 Nonce
//! - `40..`   : ChaCha20-Poly1305 密文 + 16 字节认证标签（AAD = 前 40 字节完整头部）

use std::num::NonZeroU32;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use ring::pbkdf2::{self, PBKDF2_HMAC_SHA256};
use ring::rand::{SecureRandom, SystemRandom};

/// 加密封包魔数（8 字节，区别于 gzip 的 `0x1F 0x8B`）
pub const ENCRYPTED_MAGIC: &[u8; 8] = b"MTENC01\0";
/// 默认 PBKDF2-HMAC-SHA256 迭代次数
pub const DEFAULT_KDF_ITERATIONS: u32 = 200_000;
const MIN_KDF_ITERATIONS: u32 = 10_000;
const MAX_KDF_ITERATIONS: u32 = 5_000_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = ENCRYPTED_MAGIC.len() + 4 + SALT_LEN + NONCE_LEN;
const KEY_LEN: usize = 32;

/// 判断一段字节是否带有 Mini-Todo E2EE 封包头
pub fn is_encrypted(bytes: &[u8]) -> bool {
    bytes.len() >= ENCRYPTED_MAGIC.len() && &bytes[..ENCRYPTED_MAGIC.len()] == ENCRYPTED_MAGIC
}

/// 使用口令加密明文字节（PBKDF2-HMAC-SHA256 + ChaCha20-Poly1305）
pub fn encrypt(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>, String> {
    encrypt_with_iterations(plaintext, passphrase, DEFAULT_KDF_ITERATIONS)
}

fn encrypt_with_iterations(
    plaintext: &[u8],
    passphrase: &str,
    iterations: u32,
) -> Result<Vec<u8>, String> {
    if passphrase.is_empty() {
        return Err("同步加密口令不能为空".to_string());
    }
    let iter = NonZeroU32::new(iterations)
        .filter(|n| (MIN_KDF_ITERATIONS..=MAX_KDF_ITERATIONS).contains(&n.get()))
        .ok_or_else(|| "无效的密钥派生迭代次数".to_string())?;

    let rng = SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut salt)
        .map_err(|_| "生成加密随机盐失败".to_string())?;
    rng.fill(&mut nonce_bytes)
        .map_err(|_| "生成加密随机 Nonce 失败".to_string())?;

    let key = derive_key(passphrase.as_bytes(), &salt, iter)?;

    let mut header = [0u8; HEADER_LEN];
    header[..8].copy_from_slice(ENCRYPTED_MAGIC);
    header[8..12].copy_from_slice(&iterations.to_be_bytes());
    header[12..28].copy_from_slice(&salt);
    header[28..40].copy_from_slice(&nonce_bytes);

    let mut in_out = Vec::with_capacity(plaintext.len() + CHACHA20_POLY1305.tag_len());
    in_out.extend_from_slice(plaintext);

    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    key.seal_in_place_append_tag(nonce, Aad::from(&header), &mut in_out)
        .map_err(|_| "同步数据加密失败".to_string())?;

    let mut out = Vec::with_capacity(HEADER_LEN + in_out.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&in_out);
    Ok(out)
}

/// 使用口令解密密文字节；口令错误、头部损坏或密文被篡改均返回清晰错误
pub fn decrypt(ciphertext: &[u8], passphrase: &str) -> Result<Vec<u8>, String> {
    if passphrase.is_empty() {
        return Err("云端同步数据已加密，请先配置同步加密口令".to_string());
    }
    let min_len = HEADER_LEN + CHACHA20_POLY1305.tag_len();
    if ciphertext.len() < min_len || !is_encrypted(ciphertext) {
        return Err("加密同步数据格式损坏或版本不受支持".to_string());
    }

    let header = &ciphertext[..HEADER_LEN];
    let iterations = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
    let iter = NonZeroU32::new(iterations)
        .filter(|n| (MIN_KDF_ITERATIONS..=MAX_KDF_ITERATIONS).contains(&n.get()))
        .ok_or_else(|| "加密同步数据的密钥派生参数不受支持".to_string())?;

    let salt = &header[12..28];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(&header[28..40]);

    let key = derive_key(passphrase.as_bytes(), salt, iter)?;
    let mut buf = ciphertext[HEADER_LEN..].to_vec();
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let plain = key
        .open_in_place(nonce, Aad::from(header), &mut buf)
        .map_err(|_| "解密同步数据失败：加密口令不正确或远端数据已损坏".to_string())?;
    Ok(plain.to_vec())
}

fn derive_key(
    passphrase: &[u8],
    salt: &[u8],
    iterations: NonZeroU32,
) -> Result<LessSafeKey, String> {
    let mut raw = [0u8; KEY_LEN];
    pbkdf2::derive(PBKDF2_HMAC_SHA256, iterations, salt, passphrase, &mut raw);
    let unbound =
        UnboundKey::new(&CHACHA20_POLY1305, &raw).map_err(|_| "初始化加密密钥失败".to_string())?;
    Ok(LessSafeKey::new(unbound))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_random_ciphertext() {
        let plain = b"hello mini-todo e2ee \x1f\x8b\x08\x00";
        let c1 = encrypt_with_iterations(plain, "s3cr3t-中文", MIN_KDF_ITERATIONS).unwrap();
        let c2 = encrypt_with_iterations(plain, "s3cr3t-中文", MIN_KDF_ITERATIONS).unwrap();
        assert!(is_encrypted(&c1));
        assert!(!is_encrypted(plain));
        assert_ne!(c1, c2, "相同明文每次应生成不同盐与 Nonce");
        assert_eq!(decrypt(&c1, "s3cr3t-中文").unwrap(), plain);
        assert_eq!(decrypt(&c2, "s3cr3t-中文").unwrap(), plain);
    }

    #[test]
    fn rejects_wrong_passphrase_and_tampering() {
        let mut ct = encrypt_with_iterations(b"secret", "pass-a", MIN_KDF_ITERATIONS).unwrap();
        assert!(decrypt(&ct, "pass-b").is_err());
        assert!(decrypt(&ct, "").unwrap_err().contains("加密口令"));

        // 篡改密文末尾字节
        let last = ct.len() - 1;
        ct[last] ^= 0x01;
        assert!(decrypt(&ct, "pass-a").is_err());

        // 篡改头部迭代次数（受 AAD 保护）
        let mut ct2 = encrypt_with_iterations(b"secret", "pass-a", MIN_KDF_ITERATIONS).unwrap();
        ct2[11] ^= 0x01;
        assert!(decrypt(&ct2, "pass-a").is_err());
    }

    #[test]
    fn default_iterations_round_trip() {
        let ct = encrypt(b"payload", "strong-pass").unwrap();
        assert_eq!(decrypt(&ct, "strong-pass").unwrap(), b"payload");
    }
}

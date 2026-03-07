use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};

fn derive_key() -> Result<Key<Aes256Gcm>, String> {
    let machine_id =
        machine_uid::get().map_err(|e| format!("无法获取机器标识: {}", e))?;

    let mut hasher = Sha256::new();
    hasher.update(machine_id.as_bytes());
    hasher.update(b"mini-todo-agent-key-salt");
    let result = hasher.finalize();

    Ok(*Key::<Aes256Gcm>::from_slice(&result))
}

pub fn encrypt_api_key(plain_key: &str) -> Result<String, String> {
    if plain_key.is_empty() {
        return Ok(String::new());
    }

    let key = derive_key()?;
    let cipher = Aes256Gcm::new(&key);

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plain_key.as_bytes())
        .map_err(|e| format!("加密失败: {}", e))?;

    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    Ok(base64::engine::general_purpose::STANDARD.encode(&combined))
}

pub fn decrypt_api_key(encrypted: &str) -> Result<String, String> {
    if encrypted.is_empty() {
        return Ok(String::new());
    }

    let combined = base64::engine::general_purpose::STANDARD
        .decode(encrypted)
        .map_err(|e| format!("Base64 解码失败: {}", e))?;

    if combined.len() < 13 {
        return Err("加密数据格式无效".to_string());
    }

    let key = derive_key()?;
    let cipher = Aes256Gcm::new(&key);
    let nonce = Nonce::from_slice(&combined[..12]);

    let plaintext = cipher
        .decrypt(nonce, &combined[12..])
        .map_err(|e| format!("解密失败（可能是不同机器的数据）: {}", e))?;

    String::from_utf8(plaintext).map_err(|e| format!("UTF-8 解码失败: {}", e))
}

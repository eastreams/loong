use aes::Aes256;
use base64::Engine;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::CliResult;

pub(super) fn decrypt_payload_if_needed(
    payload: &Value,
    encrypt_key: Option<&str>,
) -> CliResult<Option<Value>> {
    let Some(encrypted_payload) = payload.get("encrypt").and_then(Value::as_str) else {
        return Ok(None);
    };

    let Some(encrypt_key) = encrypt_key.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(
            "unauthorized: feishu payload is encrypted but encrypt key is not configured"
                .to_owned(),
        );
    };

    decrypt_feishu_event_payload(encrypted_payload, encrypt_key).map(Some)
}

fn decrypt_feishu_event_payload(encrypted_payload: &str, encrypt_key: &str) -> CliResult<Value> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encrypted_payload.trim())
        .map_err(|error| format!("decode encrypted feishu payload failed: {error}"))?;
    if raw.len() < 16 {
        return Err("decode encrypted feishu payload failed: payload too short".to_owned());
    }

    let iv = &raw[..16];
    let mut cipher_text = raw[16..].to_vec();
    if cipher_text.is_empty() {
        return Err("decode encrypted feishu payload failed: ciphertext is empty".to_owned());
    }

    let key = Sha256::digest(encrypt_key.as_bytes());
    let decrypted = cbc::Decryptor::<Aes256>::new_from_slices(&key, iv)
        .map_err(|error| format!("initialize feishu decryptor failed: {error}"))?
        .decrypt_padded_mut::<Pkcs7>(&mut cipher_text)
        .map_err(|error| format!("decrypt feishu payload failed: {error}"))?;

    serde_json::from_slice::<Value>(decrypted)
        .map_err(|error| format!("parse decrypted feishu payload failed: {error}"))
}

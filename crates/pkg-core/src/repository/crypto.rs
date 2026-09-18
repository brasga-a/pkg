//! Shared cryptographic verification and bounded HTTP responses for repositories.

use crate::error::{Error, Result};
use futures::StreamExt;
use pgp::{
    cleartext::CleartextSignedMessage,
    composed::{
        Deserializable, SignedPublicKey, StandaloneSignature,
        signed_key::{PublicOrSecret, SignedPublicKeyParser, from_armor_many},
    },
    packet::PacketParser,
    types::Tag,
};
use std::fs;
use std::path::Path;

/// Reads HTTP response into bytes with an enforced size limit to prevent resource exhaustion.
pub async fn bounded_response(response: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
    let response = response
        .error_for_status()
        .map_err(|e| Error::Network(e.to_string()))?;
    if response.content_length().is_some_and(|n| n > limit) {
        return Err(Error::LimitsExceeded(
            "Repository response exceeds allowed size".into(),
        ));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| Error::Network(e.to_string()))?;
        if body.len() as u64 + chunk.len() as u64 > limit {
            return Err(Error::LimitsExceeded(
                "Repository response exceeds allowed size".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Loads OpenPGP public keys from a file (supporting both binary keyring format and ASCII armor).
pub fn load_public_keys(key_path: &Path) -> Result<Vec<SignedPublicKey>> {
    let key_bytes = fs::read(key_path).map_err(Error::Io)?;

    // 1. Try parsing as binary keyring, filtering out GPG Trust packets
    let p_parser = PacketParser::new(&key_bytes[..]);
    let filtered_packets = p_parser
        .filter(|p| {
            if let Ok(pkt) = p {
                pkt.tag() != Tag::Trust
            } else {
                true
            }
        })
        .peekable();

    let mut keys = Vec::new();
    let parser = SignedPublicKeyParser::from_packets(filtered_packets);
    for key in parser.flatten() {
        keys.push(key);
    }

    // 2. If no keys parsed as binary packets, try ASCII armor
    if keys.is_empty() {
        if let Ok((iter, _)) = from_armor_many(&key_bytes[..]) {
            for key_res in iter {
                if let Ok(PublicOrSecret::Public(key)) = key_res {
                    keys.push(key);
                }
            }
        }
    }

    if keys.is_empty() {
        return Err(Error::SecurityViolation(format!(
            "No valid OpenPGP public keys found in {}",
            key_path.display()
        )));
    }

    Ok(keys)
}

/// Verifies cleartext signed message (such as Debian InRelease).
pub fn verify_cleartext_signature(signed_text: &str, key_path: &Path) -> Result<String> {
    let keys = load_public_keys(key_path)?;
    let (msg, _) = CleartextSignedMessage::from_string(signed_text)
        .map_err(|e| Error::Parse(format!("Failed to parse cleartext signed message: {:?}", e)))?;

    for key in &keys {
        if msg.verify(key).is_ok() {
            return Ok(msg.text().to_string());
        }
        for subkey in &key.public_subkeys {
            if msg.verify(subkey).is_ok() {
                return Ok(msg.text().to_string());
            }
        }
    }

    Err(Error::SecurityViolation(
        "Cleartext signature verification failed against trusted keyring".into(),
    ))
}

/// Verifies a detached OpenPGP signature (such as repomd.xml.asc or .sig) against data bytes.
pub fn verify_detached_signature(data: &[u8], sig_bytes: &[u8], key_path: &Path) -> Result<()> {
    let keys = load_public_keys(key_path)?;

    // Try parsing signature as ASCII armored first, then raw binary
    let sig = if let Ok(sig_str) = std::str::from_utf8(sig_bytes) {
        if let Ok((s, _)) = StandaloneSignature::from_string(sig_str) {
            s
        } else {
            StandaloneSignature::from_bytes(sig_bytes).map_err(|e| {
                Error::Parse(format!("Failed to parse detached OpenPGP signature: {e}"))
            })?
        }
    } else {
        StandaloneSignature::from_bytes(sig_bytes)
            .map_err(|e| Error::Parse(format!("Failed to parse detached OpenPGP signature: {e}")))?
    };

    for key in &keys {
        if sig.verify(key, data).is_ok() {
            return Ok(());
        }
        for subkey in &key.public_subkeys {
            if sig.verify(subkey, data).is_ok() {
                return Ok(());
            }
        }
    }

    Err(Error::SecurityViolation(
        "Detached signature verification failed against trusted keyring".into(),
    ))
}

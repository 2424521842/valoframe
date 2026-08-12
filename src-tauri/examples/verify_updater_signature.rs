use std::{env, fs, process};

use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};

fn main() {
    if let Err(error) = verify_from_args() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn verify_from_args() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let artifact_path = args.next().ok_or_else(|| {
        "usage: verify_updater_signature <artifact> <signature> <public-key>".to_string()
    })?;
    let signature_path = args
        .next()
        .ok_or_else(|| "signature path is required".to_string())?;
    let public_key = args
        .next()
        .ok_or_else(|| "base64-encoded public key is required".to_string())?;
    if args.next().is_some() {
        return Err("unexpected extra arguments".to_string());
    }

    let artifact = fs::read(&artifact_path)
        .map_err(|error| format!("could not read updater artifact {artifact_path}: {error}"))?;
    let signature_text = fs::read_to_string(&signature_path)
        .map_err(|error| format!("could not read updater signature {signature_path}: {error}"))?;
    verify_signature(&artifact, signature_text.trim(), public_key.trim())
}

fn verify_signature(data: &[u8], release_signature: &str, public_key: &str) -> Result<(), String> {
    let public_key_text = decode_base64_text(public_key, "public key")?;
    let public_key = PublicKey::decode(&public_key_text)
        .map_err(|error| format!("public key is invalid: {error}"))?;
    let signature_text = decode_base64_text(release_signature, "signature")?;
    let signature = Signature::decode(&signature_text)
        .map_err(|error| format!("signature is invalid: {error}"))?;
    public_key
        .verify(data, &signature, true)
        .map_err(|error| format!("updater signature verification failed: {error}"))
}

fn decode_base64_text(value: &str, description: &str) -> Result<String, String> {
    let decoded = STANDARD
        .decode(value)
        .map_err(|error| format!("{description} is not valid base64: {error}"))?;
    String::from_utf8(decoded)
        .map_err(|error| format!("{description} does not contain UTF-8 minisign data: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLIC_KEY_TEXT: &str = "untrusted comment: minisign public key E7620F1842B4E81F\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    const SIGNATURE_TEXT: &str = "untrusted comment: signature from minisign secret key\nRWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\ntrusted comment: timestamp:1555779966\tfile:test\nQtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";

    fn release_values() -> (String, String) {
        (
            STANDARD.encode(SIGNATURE_TEXT.as_bytes()),
            STANDARD.encode(PUBLIC_KEY_TEXT.as_bytes()),
        )
    }

    #[test]
    fn accepts_a_valid_tauri_minisign_envelope() {
        let (signature, public_key) = release_values();
        verify_signature(b"test", &signature, &public_key).expect("fixture should verify");
    }

    #[test]
    fn rejects_tampered_artifacts_and_signatures() {
        let (signature, public_key) = release_values();
        assert!(verify_signature(b"Test", &signature, &public_key).is_err());

        let tampered_signature =
            STANDARD.encode(SIGNATURE_TEXT.replace("QtKMXWyY", "AtKMXWyY").as_bytes());
        assert!(verify_signature(b"test", &tampered_signature, &public_key).is_err());
    }
}

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Check Meta's `X-Hub-Signature-256` header ("sha256=<hex>") against the raw request body.
/// Without this anyone could POST fake "voice notes" to the webhook and write to the ledger.
/// The comparison is constant-time.
pub fn verify_signature(app_secret: &str, body: &[u8], header: Option<&str>) -> bool {
    let Some(header) = header else {
        return false;
    };
    let Some(hex_sig) = header.trim().strip_prefix("sha256=") else {
        return false;
    };
    let Ok(expected) = hex::decode(hex_sig) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(app_secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn accepts_valid_signature() {
        let body = br#"{"hello":"world"}"#;
        let header = sign("topsecret", body);
        assert!(verify_signature("topsecret", body, Some(&header)));
    }

    #[test]
    fn rejects_wrong_secret() {
        let body = br#"{"hello":"world"}"#;
        let header = sign("topsecret", body);
        assert!(!verify_signature("another", body, Some(&header)));
    }

    #[test]
    fn rejects_tampered_body() {
        let header = sign("topsecret", br#"{"amount":100}"#);
        assert!(!verify_signature("topsecret", br#"{"amount":999}"#, Some(&header)));
    }

    #[test]
    fn rejects_missing_or_malformed_header() {
        let body = b"x";
        assert!(!verify_signature("topsecret", body, None));
        assert!(!verify_signature("topsecret", body, Some("deadbeef")));
        assert!(!verify_signature("topsecret", body, Some("sha256=not-hex")));
    }
}
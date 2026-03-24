use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::crypto::timing_safe_eq;

type HmacSha256 = Hmac<Sha256>;

/// Computes the 32-byte HMAC-SHA256 tag for `message`.
///
/// The `Option` mirrors the `Mac::new_from_slice` constructor even though
/// HMAC-SHA256 is expected to accept arbitrary key lengths.
#[must_use]
pub fn compute_hmac_sha256(secret: &[u8], message: &[u8]) -> Option<[u8; 32]> {
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(message);
    Some(mac.finalize().into_bytes().into())
}

/// Verifies a hex-encoded HMAC-SHA256 signature with constant-time equality.
///
/// Malformed hex input returns `false`.
#[must_use]
pub fn verify_hmac_sha256(secret: &[u8], message: &[u8], expected_signature_hex: &str) -> bool {
    let Ok(expected_signature) = hex::decode(expected_signature_hex) else {
        return false;
    };
    let Some(actual_signature) = compute_hmac_sha256(secret, message) else {
        return false;
    };
    timing_safe_eq(&actual_signature, &expected_signature)
}

#[cfg(test)]
mod tests {
    use super::{compute_hmac_sha256, verify_hmac_sha256};

    #[test]
    fn verify_hmac_sha256_accepts_known_vector() {
        let key = [0x0b_u8; 20];
        let message = b"Hi There";
        let expected = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";

        assert!(verify_hmac_sha256(&key, message, expected));
    }

    #[test]
    fn verify_hmac_sha256_rejects_malformed_hex() {
        assert!(!verify_hmac_sha256(b"secret", b"payload", "not-hex"));
    }

    #[test]
    fn verify_hmac_sha256_rejects_well_formed_mismatch() {
        let key = [0x0b_u8; 20];
        let message = b"Hi There";
        let incorrect = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff6";

        assert!(!verify_hmac_sha256(&key, message, incorrect));
    }

    #[test]
    fn compute_hmac_sha256_returns_expected_bytes() {
        let key = [0x0b_u8; 20];
        let message = b"Hi There";
        let expected: [u8; 32] = [
            0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
            0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
            0x2e, 0x32, 0xcf, 0xf7,
        ];

        assert_eq!(compute_hmac_sha256(&key, message), Some(expected));
    }
}

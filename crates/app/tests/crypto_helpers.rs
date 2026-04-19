use loong_app::crypto::{timing_safe_eq, verify_hmac_sha256};

#[test]
fn timing_safe_eq_rejects_mismatched_lengths() {
    assert!(!timing_safe_eq(b"abcd", b"abc"));
}

#[test]
fn verify_hmac_sha256_accepts_rfc_vector_hex() {
    let key = [0x0b_u8; 20];
    let message = b"Hi There";
    let signature_hex = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";

    assert!(verify_hmac_sha256(&key, message, signature_hex));
}

#[test]
fn verify_hmac_sha256_rejects_malformed_hex() {
    assert!(!verify_hmac_sha256(
        b"secret",
        b"payload",
        "not-hex-signature"
    ));
}

#[test]
fn verify_hmac_sha256_rejects_well_formed_but_incorrect_hex() {
    let key = [0x0b_u8; 20];
    let message = b"Hi There";
    let incorrect_hex = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff6";

    assert!(!verify_hmac_sha256(&key, message, incorrect_hex));
}

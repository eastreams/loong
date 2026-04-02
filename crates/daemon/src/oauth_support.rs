use base64::Engine as _;
use rand::RngExt;
use sha2::Digest;
use sha2::Sha256;

pub(crate) fn generate_oauth_state() -> String {
    random_urlsafe_token(24)
}

pub(crate) fn build_pkce_pair() -> (String, String) {
    let verifier = random_urlsafe_token(32);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    (verifier, challenge)
}

fn random_urlsafe_token(bytes_len: usize) -> String {
    let normalized_len = bytes_len.max(16);
    let mut bytes = vec![0_u8; normalized_len];
    let mut rng = rand::rng();
    rng.fill(bytes.as_mut_slice());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

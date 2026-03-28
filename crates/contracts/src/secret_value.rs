use zeroize::Zeroize;

pub struct SecretValue {
    inner: String,
}

impl SecretValue {
    pub fn new(inner: String) -> Self {
        Self { inner }
    }

    pub fn expose(&self) -> &str {
        self.inner.as_str()
    }

    pub fn into_inner(mut self) -> String {
        std::mem::take(&mut self.inner)
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.inner.zeroize();
    }
}

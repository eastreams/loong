use std::ffi::OsStr;

/// Mutate process environment only during startup or under a serialized
/// test lock. Rust 2024 marks these APIs as unsafe because concurrent
/// mutation can race with readers in other threads.
#[inline]
pub(crate) fn set_var(key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var(key, value);
    }
}

/// Remove process environment variables under the same startup/test-only
/// constraints as [`set_var`].
#[inline]
pub(crate) fn remove_var(key: impl AsRef<OsStr>) {
    #[allow(unsafe_code)]
    unsafe {
        std::env::remove_var(key);
    }
}

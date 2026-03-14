use std::ffi::{OsStr, OsString};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) struct ScopedEnv {
    originals: Vec<(&'static str, Option<OsString>)>,
    _guard: MutexGuard<'static, ()>,
}

impl ScopedEnv {
    pub(crate) fn new() -> Self {
        let guard = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self {
            originals: Vec::new(),
            _guard: guard,
        }
    }

    #[allow(clippy::disallowed_methods)]
    pub(crate) fn set(&mut self, key: &'static str, value: impl AsRef<OsStr>) {
        self.capture_original(key);
        crate::process_env::set_var(key, value);
    }

    #[allow(dead_code, clippy::disallowed_methods)]
    pub(crate) fn remove(&mut self, key: &'static str) {
        self.capture_original(key);
        crate::process_env::remove_var(key);
    }

    fn capture_original(&mut self, key: &'static str) {
        if self.originals.iter().any(|(saved, _)| *saved == key) {
            return;
        }
        self.originals.push((key, std::env::var_os(key)));
    }
}

impl Drop for ScopedEnv {
    #[allow(clippy::disallowed_methods)]
    fn drop(&mut self) {
        for (key, original) in self.originals.iter().rev() {
            match original {
                Some(value) => crate::process_env::set_var(key, value),
                None => crate::process_env::remove_var(key),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScopedEnv;

    #[test]
    fn scoped_env_recovers_after_mutex_poison() {
        let panic_result = std::thread::spawn(|| {
            let _env = ScopedEnv::new();
            panic!("poison env lock for test");
        })
        .join();

        assert!(panic_result.is_err(), "setup thread should poison the lock");

        let recovery = std::panic::catch_unwind(ScopedEnv::new);
        assert!(
            recovery.is_ok(),
            "ScopedEnv::new should recover from a poisoned env lock"
        );
    }
}

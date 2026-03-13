use std::path::PathBuf;

/// Resolve the user's home directory.
/// Checks HOME first, then USERPROFILE (Windows fallback).
/// Returns an error if neither is set.
pub fn taida_home_dir() -> Result<PathBuf, String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| "Home directory not found: neither HOME nor USERPROFILE is set".to_string())
}

/// Shared lock for tests that modify environment variables.
///
/// All tests across modules that call `std::env::set_var` / `std::env::remove_var`
/// must acquire this lock to prevent data races.
#[cfg(test)]
pub fn env_test_lock() -> &'static std::sync::Mutex<()> {
    use std::sync::OnceLock;
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

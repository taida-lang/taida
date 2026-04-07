//! Prebuild addon binary fetcher (RC1.5 Phase 2).
//!
//! Downloads prebuild `.so`/`.dylib`/`.dll` binaries from a URL,
//! verifies SHA-256 integrity, and places them atomically into
//! the addon cache (`~/.taida/addon-cache/<org>/<name>/<version>/<target>/`).
//!
//! - HTTPS: uses `reqwest` blocking (production)
//! - `file://`: copies from local path (testing / dev)
//! - SHA-256: streaming verification during download
//! - Size limit: 100 MB
//! - Cache hit: re-verify SHA-256 before reuse

use std::fmt;
use std::path::{Path, PathBuf};

use crate::crypto::Sha256;

// ── Error ───────────────────────────────────────────────────────

/// Errors from fetching / verifying / caching a prebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FetchError {
    /// Target is not in the supported list.
    UnsupportedTarget {
        host: String,
        supported: Vec<String>,
    },
    /// Download / file copy failed.
    DownloadFailed { message: String },
    /// SHA-256 mismatch after download.
    IntegrityMismatch { expected: String, actual: String },
    /// Download exceeded 100 MB size limit.
    SizeLimitExceeded { max_bytes: u64, actual_bytes: u64 },
    /// Cache I/O error (directory creation, rename, write).
    CacheIoError { message: String },
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTarget { host, supported } => {
                writeln!(f, "addon is not available for your platform")?;
                writeln!(f, "  host target:    {host}")?;
                writeln!(f, "  supported targets:")?;
                for t in supported {
                    writeln!(f, "    - {t}")?;
                }
                write!(
                    f,
                    "  action: ask the addon author to upload a prebuild for {host}"
                )
            }
            Self::DownloadFailed { message } => {
                write!(f, "prebuild download failed: {message}")
            }
            Self::IntegrityMismatch { expected, actual } => {
                writeln!(f, "addon integrity check failed")?;
                writeln!(f, "  expected: {expected}")?;
                write!(f, "  actual:   {actual}")
            }
            Self::SizeLimitExceeded {
                max_bytes,
                actual_bytes,
            } => {
                write!(
                    f,
                    "prebuild binary exceeded the {} MB size limit (downloaded {actual_bytes} bytes)",
                    max_bytes / 1_048_576,
                )
            }
            Self::CacheIoError { message } => {
                write!(f, "cache I/O error: {message}")
            }
        }
    }
}

impl std::error::Error for FetchError {}

fn download_fail(message: impl Into<String>) -> FetchError {
    FetchError::DownloadFailed {
        message: message.into(),
    }
}

fn cache_io(message: impl Into<String>) -> FetchError {
    FetchError::CacheIoError {
        message: message.into(),
    }
}

// ── Constants ───────────────────────────────────────────────────

/// Maximum allowed prebuild binary size (100 MB).
const MAX_SIZE_BYTES: u64 = 100 * 1024 * 1024;

// ── Cache paths ────────────────────────────────────────────────

/// Returns the addon cache root (`~/.taida/addon-cache`).
fn cache_root() -> Result<PathBuf, FetchError> {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| cache_io("cannot determine home directory ($HOME not set)"))?;
    Ok(home.join(".taida/addon-cache"))
}

/// Cache path for a specific prebuild version+target.
///
/// `~/.taida/addon-cache/<org>/<name>/<version>/<target>/`
fn cache_dir_for(
    org: &str,
    name: &str,
    version: &str,
    target: &str,
) -> Result<PathBuf, FetchError> {
    let root = cache_root()?;
    Ok(root.join(org).join(name).join(version).join(target))
}

/// Sidecar file storing the expected SHA-256 for cheap lookup.
fn sha256_sidecar(cache_dir: &Path) -> PathBuf {
    cache_dir.join(".manifest-sha256")
}

// ── SHA-256 verification of an existing file ───────────────────

fn verify_sha256(path: &Path, expected: &str) -> Result<(), FetchError> {
    let data = std::fs::read(path)
        .map_err(|e| cache_io(format!("cannot read cached file {}: {}", path.display(), e)))?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let actual = hasher.finalize_hex();
    if actual == expected {
        Ok(())
    } else {
        Err(FetchError::IntegrityMismatch {
            expected: expected.to_string(),
            actual,
        })
    }
}

// ── Parse org/name from package id ─────────────────────────────

/// Splits `"taida-lang/terminal"` into `("taida-lang", "terminal")`.
fn split_package_id(package_id: &str) -> Option<(&str, &str)> {
    package_id.split_once('/')
}

// ── Fetcher ────────────────────────────────────────────────────

/// Fetch a prebuild addon binary.
///
/// 1. Checks if the file is already cached and verifies its SHA-256.
/// 2. If not, downloads/copies from the URL with streaming SHA-256.
/// 3. Places the file atomically (temp file -> rename).
/// 4. Writes a `.manifest-sha256` sidecar for cheap lookup.
///
/// `package_id`: e.g. `"taida-lang/terminal"` (used to build cache path)
/// `version`:    e.g. `"a.1"`
/// `target_triple`: canonical triple e.g. `"x86_64-unknown-linux-gnu"`
/// `lib_name`:    cdylib stem e.g. `"terminal"`
/// `ext`:         platform extension e.g. `"so"`
/// `url`:         full download URL or `file://` path
/// `expected_sha256`: `sha256:` prefix + 64 hex chars (the exact hex part)
pub fn fetch_prebuild(
    package_id: &str,
    version: &str,
    target_triple: &str,
    lib_name: &str,
    ext: &str,
    url: &str,
    expected_sha256: &str,
) -> Result<PathBuf, FetchError> {
    let (org, name) = split_package_id(package_id)
        .ok_or_else(|| download_fail(format!("invalid package id: {package_id}")))?;

    let cache_dir = cache_dir_for(org, name, version, target_triple)?;
    let lib_filename = format!("lib{lib_name}.{ext}");
    let dest_path = cache_dir.join(&lib_filename);

    // 1. Cache hit: verify SHA-256 and return.
    if dest_path.exists() {
        if let Err(e) = verify_sha256(&dest_path, expected_sha256) {
            // Corrupted cache, remove and re-download.
            let _ = std::fs::remove_file(&dest_path);
            let _ = std::fs::remove_file(sha256_sidecar(&cache_dir));
            return Err(e);
        }
        return Ok(dest_path);
    }

    // 2. Download / copy with streaming SHA-256.
    let data = download_from_url(url, expected_sha256)?;

    // 3. Atomic placement.
    std::fs::create_dir_all(&cache_dir).map_err(|e| {
        cache_io(format!(
            "cannot create cache dir {}: {}",
            cache_dir.display(),
            e
        ))
    })?;

    let temp_path = cache_dir.join(format!("{lib_filename}.tmp"));
    std::fs::write(&temp_path, &data).map_err(|e| {
        cache_io(format!(
            "cannot write temp file {}: {}",
            temp_path.display(),
            e
        ))
    })?;

    std::fs::rename(&temp_path, &dest_path).map_err(|e| {
        cache_io(format!(
            "cannot atomically place {}: {}",
            dest_path.display(),
            e
        ))
    })?;

    // 4. Write sidecar.
    std::fs::write(sha256_sidecar(&cache_dir), expected_sha256)
        .map_err(|e| cache_io(format!("cannot write sha256 sidecar: {}", e)))?;

    Ok(dest_path)
}

/// Download from URL with streaming SHA-256 verification.
fn download_from_url(url: &str, expected_sha256: &str) -> Result<Vec<u8>, FetchError> {
    if let Some(path) = url.strip_prefix("file://") {
        download_from_file(path, expected_sha256)
    } else if url.starts_with("https://") {
        download_from_https(url, expected_sha256)
    } else {
        Err(download_fail(format!(
            "unsupported URL scheme (only https:// and file:// are allowed): {url}"
        )))
    }
}

/// Copy a file from local path with SHA-256 verification.
fn download_from_file(file_path: &str, expected_sha256: &str) -> Result<Vec<u8>, FetchError> {
    let data = std::fs::read(file_path)
        .map_err(|e| download_fail(format!("cannot read file {}: {}", file_path, e)))?;

    if data.len() as u64 > MAX_SIZE_BYTES {
        return Err(FetchError::SizeLimitExceeded {
            max_bytes: MAX_SIZE_BYTES,
            actual_bytes: data.len() as u64,
        });
    }

    let mut hasher = Sha256::new();
    hasher.update(&data);
    let actual = hasher.finalize_hex();

    if actual != expected_sha256 {
        return Err(FetchError::IntegrityMismatch {
            expected: expected_sha256.to_string(),
            actual,
        });
    }

    Ok(data)
}

/// Download from HTTPS URL with streaming SHA-256 verification.
#[cfg(feature = "community")]
fn download_from_https(url: &str, expected_sha256: &str) -> Result<Vec<u8>, FetchError> {
    use reqwest::blocking::Client;
    use std::io::Read;

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| download_fail(format!("cannot create HTTP client: {e}")))?;

    let mut response = client
        .get(url)
        .send()
        .map_err(|e| download_fail(format!("request failed: {e}")))?;

    if !response.status().is_success() {
        return Err(download_fail(format!(
            "HTTP {} for {url}",
            response.status()
        )));
    }

    // Check Content-Length upper bound.
    if let Some(len) = response.content_length()
        && len > MAX_SIZE_BYTES
    {
        return Err(FetchError::SizeLimitExceeded {
            max_bytes: MAX_SIZE_BYTES,
            actual_bytes: len,
        });
    }

    let mut hasher = Sha256::new();
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];

    loop {
        let n = response
            .read(&mut buf)
            .map_err(|e| download_fail(format!("read failed: {e}")))?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        if data.len() as u64 + chunk.len() as u64 > MAX_SIZE_BYTES {
            return Err(FetchError::SizeLimitExceeded {
                max_bytes: MAX_SIZE_BYTES,
                actual_bytes: data.len() as u64 + chunk.len() as u64,
            });
        }
        hasher.update(chunk);
        data.extend_from_slice(chunk);
    }

    let actual = hasher.finalize_hex();
    if actual != expected_sha256 {
        return Err(FetchError::IntegrityMismatch {
            expected: expected_sha256.to_string(),
            actual,
        });
    }

    Ok(data)
}

/// HTTPS download for when `community` feature is not enabled (test mode).
#[cfg(not(feature = "community"))]
fn download_from_https(url: &str, _expected_sha256: &str) -> Result<Vec<u8>, FetchError> {
    Err(download_fail(format!(
        "HTTPS downloads require the 'community' feature: {url}"
    )))
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp_file(data: &[u8]) -> (PathBuf, String) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        use std::time::SystemTime;

        let mut hasher = Sha256::new();
        hasher.update(data);
        let sha = hasher.finalize_hex();

        // Unique filename with timestamp + hash to avoid collisions between tests
        let mut h = DefaultHasher::new();
        h.write_u64(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        );
        h.write(sha.as_bytes());
        let unique = h.finish();

        let dir = std::env::temp_dir();
        let path = dir.join(format!("taida_fetcher_test_{}_{}", unique, &sha[..16]));
        std::fs::write(&path, data).unwrap();

        (path, sha)
    }

    #[test]
    fn file_scheme_happy_path() {
        let data = b"test binary content";
        let (path, sha) = make_temp_file(data);

        // file:// URL with absolute path: file:///tmp/taida_test_xxx
        let url = format!("file://{}", path.display());
        let result = download_from_url(&url, &sha);
        if let Err(ref e) = result {
            panic!("download failed: {:?}\nurl: {}", e, url);
        }
        assert_eq!(result.unwrap(), *data);

        // Cleanup.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_scheme_integrity_mismatch() {
        let data = b"test binary content";
        let (path, _) = make_temp_file(data);

        let url = format!("file://{}", path.display());
        let wrong_sha = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let err = download_from_url(&url, wrong_sha).unwrap_err();
        assert!(matches!(err, FetchError::IntegrityMismatch { .. }));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unsupported_scheme_is_rejected() {
        let err = download_from_url("http://example.com/binary.so", "sha256:aa").unwrap_err();
        assert!(matches!(err, FetchError::DownloadFailed { .. }));
        assert!(err.to_string().contains("unsupported URL scheme"));
    }

    #[test]
    fn split_package_id_valid() {
        assert_eq!(
            split_package_id("taida-lang/terminal"),
            Some(("taida-lang", "terminal"))
        );
    }

    #[test]
    fn split_package_id_invalid() {
        assert_eq!(split_package_id("no-slash"), None);
    }

    #[test]
    fn fetch_error_display_unsupported_target() {
        let err = FetchError::UnsupportedTarget {
            host: "x86_64-unknown-freebsd".to_string(),
            supported: vec!["x86_64-unknown-linux-gnu".to_string()],
        };
        let msg = err.to_string();
        assert!(msg.contains("not available for your platform"));
        assert!(msg.contains("x86_64-unknown-freebsd"));
        assert!(msg.contains("supported targets"));
    }

    #[test]
    fn fetch_error_display_integrity_mismatch() {
        let err = FetchError::IntegrityMismatch {
            expected: "sha256:aaaa".to_string(),
            actual: "sha256:bbbb".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("integrity check failed"));
        assert!(msg.contains("expected"));
        assert!(msg.contains("actual"));
    }

    #[test]
    fn verify_sha256_happy() {
        let data = b"hello world";
        let (path, sha) = make_temp_file(data);
        assert!(verify_sha256(&path, &sha).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_sha256_mismatch() {
        let data = b"hello world";
        let (path, _) = make_temp_file(data);
        let wrong_sha = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert!(verify_sha256(&path, wrong_sha).is_err());
        let _ = std::fs::remove_file(&path);
    }
}

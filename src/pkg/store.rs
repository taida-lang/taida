/// Global package store for Taida Lang.
///
/// Manages `~/.taida/store/` where downloaded packages are cached.
///
/// Layout:
/// ```text
/// ~/.taida/store/{org}/{name}/{gen}.{num}[.{label}]/
///   main.td
///   mod.td
///   packages.tdm       <- used for transitive dependency resolution
///   ...
///   .taida_installed   <- download completion marker
///   _meta.toml         <- C17 sidecar: provenance + stale-check metadata
/// ```
///
/// ## C17 sidecar (`_meta.toml`)
///
/// C17 introduces a provenance sidecar written alongside `.taida_installed`.
/// The sidecar records the tarball SHA-256, the resolved commit SHA (filled
/// by the resolver in C17-2 / Phase 2), an RFC-3339 `fetched_at` timestamp,
/// and the source identifier (e.g. `github:taida-lang/terminal`).
///
/// The sidecar exists so `taida install` can detect stale store entries when
/// a tag is republished (retag / delete+recreate). See `.dev/C17_DESIGN.md`.
///
/// C17-1 only writes the sidecar. The stale-detection decision table is
/// implemented in C17-2 (Phase 2) and consumed by `taida install` there.
use std::path::{Path, PathBuf};

/// Base URL for GitHub archive downloads.
/// Override with `TAIDA_GITHUB_BASE_URL` for testing (e.g. local mock server).
pub(crate) fn github_base_url() -> String {
    std::env::var("TAIDA_GITHUB_BASE_URL").unwrap_or_else(|_| "https://github.com".to_string())
}

/// Base URL for GitHub API calls.
/// Override with `TAIDA_GITHUB_API_URL` for testing.
fn github_api_url() -> String {
    std::env::var("TAIDA_GITHUB_API_URL").unwrap_or_else(|_| "https://api.github.com".to_string())
}

/// Global package store at `~/.taida/store/`.
pub struct GlobalStore {
    root: PathBuf,
}

impl Default for GlobalStore {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalStore {
    /// Create a new GlobalStore using the default location (`~/.taida/store/`).
    pub fn new() -> Self {
        let home = crate::util::taida_home_dir().unwrap_or_else(|_| std::env::temp_dir());
        GlobalStore {
            root: home.join(".taida").join("store"),
        }
    }

    /// Create a GlobalStore with a custom root (for testing).
    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        GlobalStore { root }
    }

    /// Validate that a path component does not contain traversal sequences.
    /// Rejects `..`, `/`, `\`, and empty strings (RCB-307 / SEC-009).
    fn validate_path_component(component: &str, label: &str) -> Result<(), String> {
        if component.is_empty() {
            return Err(format!("{} must not be empty", label));
        }
        if component.contains("..") || component.contains('/') || component.contains('\\') {
            return Err(format!(
                "Invalid {}: '{}'. Path traversal characters ('..', '/', '\\') are not allowed.",
                label, component
            ));
        }
        Ok(())
    }

    /// Get the path for a specific package version in the store.
    pub fn package_path(&self, org: &str, name: &str, version: &str) -> PathBuf {
        self.root.join(org).join(name).join(version)
    }

    /// Check if a package version is already cached in the store.
    pub fn is_cached(&self, org: &str, name: &str, version: &str) -> bool {
        // RCB-307: Reject path traversal in components
        if Self::validate_path_component(org, "org").is_err()
            || Self::validate_path_component(name, "package name").is_err()
            || Self::validate_path_component(version, "version").is_err()
        {
            return false;
        }
        let pkg_dir = self.package_path(org, name, version);
        pkg_dir.join(".taida_installed").exists()
    }

    /// Fetch a package from GitHub and cache it in the store.
    ///
    /// Downloads the tarball from `https://github.com/{org}/{name}/archive/refs/tags/{version}.tar.gz`,
    /// extracts it to `~/.taida/store/{org}/{name}/{version}/`, creates a
    /// `.taida_installed` marker, and writes a C17 `_meta.toml` provenance
    /// sidecar with the tarball SHA-256.
    pub fn fetch_and_cache(&self, org: &str, name: &str, version: &str) -> Result<PathBuf, String> {
        self.fetch_and_cache_with_meta(org, name, version, None)
    }

    /// Fetch a package from GitHub and cache it, optionally recording a
    /// resolver-supplied commit SHA in the `_meta.toml` sidecar.
    ///
    /// C17-1 (Phase 1): records `tarball_sha256`, `fetched_at`, `source`,
    /// `version`. The `commit_sha` is supplied by C17-2 (Phase 2) once the
    /// resolver learns the SHA via `git ls-remote`. When `commit_sha` is
    /// `None` the sidecar stores an empty string -- the decision table in
    /// Phase 2 treats that as "sidecar present but SHA unknown".
    pub fn fetch_and_cache_with_meta(
        &self,
        org: &str,
        name: &str,
        version: &str,
        commit_sha: Option<&str>,
    ) -> Result<PathBuf, String> {
        // RCB-307: Reject path traversal in components
        Self::validate_path_component(org, "org")?;
        Self::validate_path_component(name, "package name")?;
        Self::validate_path_component(version, "version")?;

        let pkg_dir = self.package_path(org, name, version);

        // Already cached
        if self.is_cached(org, name, version) {
            return Ok(pkg_dir);
        }

        // Create parent directories
        std::fs::create_dir_all(&pkg_dir).map_err(|e| {
            format!(
                "Cannot create store directory '{}': {}",
                pkg_dir.display(),
                e
            )
        })?;

        // Download tarball from GitHub (or mock server via TAIDA_GITHUB_BASE_URL)
        let url = format!(
            "{}/{}/{}/archive/refs/tags/{}.tar.gz",
            github_base_url().trim_end_matches('/'),
            org,
            name,
            version
        );
        let tmp_dir = self
            .root
            .join(org)
            .join(name)
            .join(format!(".tmp-{}", version));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|e| format!("Cannot create temp directory: {}", e))?;

        let archive_path = tmp_dir.join("archive.tar.gz");

        // Use curl to download
        let curl_status = std::process::Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&archive_path)
            .arg(&url)
            .status()
            .map_err(|e| format!("Failed to run curl: {}", e))?;

        if !curl_status.success() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err(format!(
                "Failed to download package {}/{}@{} from {}",
                org, name, version, url
            ));
        }

        // C17-1: compute tarball SHA-256 before extraction. The archive is
        // read once here -- small enough for addons (tens of KiB to a few
        // MiB) that a single in-memory pass is acceptable.
        let tarball_sha256 = compute_file_sha256(&archive_path).map_err(|e| {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            format!(
                "Failed to hash tarball for {}/{}@{}: {}",
                org, name, version, e
            )
        })?;

        // Extract tarball (--strip-components=1 removes the top-level directory)
        let tar_status = std::process::Command::new("tar")
            .args(["xzf"])
            .arg(&archive_path)
            .args(["--strip-components=1", "-C"])
            .arg(&pkg_dir)
            .status()
            .map_err(|e| format!("Failed to run tar: {}", e))?;

        if !tar_status.success() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            let _ = std::fs::remove_dir_all(&pkg_dir);
            return Err(format!(
                "Failed to extract package {}/{}@{}",
                org, name, version
            ));
        }

        // Cleanup temp directory before verification so it never leaks
        let _ = std::fs::remove_dir_all(&tmp_dir);

        // Post-fetch manifest verification: ensure the extracted package's
        // packages.tdm identity and version match what was requested.
        Self::verify_fetched_package(&pkg_dir, org, name, version)?;

        // C17-1: write provenance sidecar before the installed marker. The
        // sidecar is written atomically so a crash after the marker but
        // before the sidecar leaves only an "unknown provenance" state that
        // the Phase 2 decision table treats as pessimistic-refresh.
        let meta = StoreMeta {
            schema_version: STORE_META_SCHEMA_VERSION,
            commit_sha: commit_sha.unwrap_or("").to_string(),
            tarball_sha256,
            tarball_etag: None,
            fetched_at: rfc3339_now(),
            source: format!("github:{}/{}", org, name),
            version: version.to_string(),
        };
        let meta_path = meta_path_for(&pkg_dir);
        if let Err(e) = write_meta_atomic(&meta_path, &meta) {
            // Clean up the half-installed package so the next install retries
            // from scratch rather than observing a manifest+data without
            // provenance metadata.
            let _ = std::fs::remove_dir_all(&pkg_dir);
            return Err(format!(
                "Failed to write store sidecar for {}/{}@{}: {}",
                org, name, version, e
            ));
        }

        // Create completion marker
        std::fs::write(pkg_dir.join(".taida_installed"), "")
            .map_err(|e| format!("Cannot create install marker: {}", e))?;

        Ok(pkg_dir)
    }

    /// Resolve a generation-only version (e.g. "a") to an exact version (e.g. "a.47").
    ///
    /// 1. Scans the local cache for matching versions
    /// 2. If not found locally, queries GitHub API for tags
    /// 3. Fetches and caches the resolved version
    pub fn resolve_generation(
        &self,
        org: &str,
        name: &str,
        generation: &str,
    ) -> Result<String, String> {
        // RCB-307: Reject path traversal in components
        Self::validate_path_component(org, "org")?;
        Self::validate_path_component(name, "package name")?;
        Self::validate_path_component(generation, "generation")?;

        // First, check local cache
        let pkg_parent = self.root.join(org).join(name);
        if pkg_parent.exists()
            && let Some(version) = self.find_latest_in_generation(&pkg_parent, generation)
        {
            return Ok(version);
        }

        self.resolve_generation_from_remote(org, name, generation)
    }

    /// Resolve a generation-only version by always querying GitHub API (bypass local cache).
    ///
    /// Used by `taida update` to find the latest version even when an older version is cached.
    pub fn resolve_generation_remote(
        &self,
        org: &str,
        name: &str,
        generation: &str,
    ) -> Result<String, String> {
        // RCB-307: Reject path traversal in components
        Self::validate_path_component(org, "org")?;
        Self::validate_path_component(name, "package name")?;
        Self::validate_path_component(generation, "generation")?;

        self.resolve_generation_from_remote(org, name, generation)
    }

    /// Internal: query GitHub API for the latest version in a generation, fetch and cache it.
    fn resolve_generation_from_remote(
        &self,
        org: &str,
        name: &str,
        generation: &str,
    ) -> Result<String, String> {
        // Query GitHub API for tags (or mock server via TAIDA_GITHUB_API_URL)
        let url = format!(
            "{}/repos/{}/{}/tags?per_page=100",
            github_api_url().trim_end_matches('/'),
            org,
            name
        );

        let output = std::process::Command::new("curl")
            .args(["-fsSL", "-H", "Accept: application/vnd.github.v3+json"])
            .arg(&url)
            .output()
            .map_err(|e| format!("Failed to query GitHub tags for {}/{}: {}", org, name, e))?;

        if !output.status.success() {
            return Err(format!(
                "Failed to fetch tags for {}/{}: HTTP error",
                org, name
            ));
        }

        let body = String::from_utf8_lossy(&output.stdout);
        let prefix_new = format!("{}.", generation);
        let prefix_legacy = format!("v{}.", generation);

        // Parse tag names from JSON (handles both pretty-printed and compressed JSON)
        // Tags can be "a.3", "a.3.alpha" (new) or "va.3" (legacy) — extract num and pick highest
        let mut best: Option<(u64, String)> = None; // (num, version_without_v)
        for tag in extract_json_name_values(&body) {
            let suffix = tag
                .strip_prefix(&prefix_new)
                .or_else(|| tag.strip_prefix(&prefix_legacy));
            if let Some(suffix) = suffix {
                // suffix is "num" or "num.label"
                let num_str = suffix.split('.').next().unwrap_or(suffix);
                if let Ok(num) = num_str.parse::<u64>() {
                    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
                    if best.as_ref().is_none_or(|(prev, _)| num > *prev) {
                        best = Some((num, version));
                    }
                }
            }
        }

        match best {
            Some((_, exact)) => {
                // Fetch and cache the resolved version
                self.fetch_and_cache(org, name, &exact)?;
                Ok(exact)
            }
            None => Err(format!(
                "No version found for {}/{}@{} (generation '{}')",
                org, name, generation, generation
            )),
        }
    }

    /// Find the latest version matching a generation in local cache.
    /// Handles both `gen.num` and `gen.num.label` directory names.
    fn find_latest_in_generation(&self, pkg_parent: &Path, generation: &str) -> Option<String> {
        let entries = std::fs::read_dir(pkg_parent).ok()?;
        let prefix = format!("{}.", generation);
        let mut best: Option<(u64, String)> = None; // (num, full_version)

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(suffix) = name.strip_prefix(&prefix) {
                // suffix is "num" or "num.label"
                let num_str = suffix.split('.').next().unwrap_or(suffix);
                if let Ok(num) = num_str.parse::<u64>() {
                    // Only count if actually installed
                    if entry.path().join(".taida_installed").exists()
                        && best.as_ref().is_none_or(|(prev, _)| num > *prev)
                    {
                        best = Some((num, name));
                    }
                }
            }
        }

        best.map(|(_, version)| version)
    }

    /// Post-fetch verification: ensure the extracted package's manifest
    /// declares an identity and version consistent with what was requested.
    ///
    /// For packages with a `packages.tdm` that declares a qualified name
    /// (`org/name`), the identity must exactly match `expected_org/expected_name`.
    /// The version in the manifest must match `expected_version`.
    ///
    /// For addon packages with `native/addon.toml`, the `package` field
    /// must also match the expected `org/name`.
    ///
    /// On mismatch, the package directory is cleaned up and an error is returned.
    fn verify_fetched_package(
        pkg_dir: &Path,
        expected_org: &str,
        expected_name: &str,
        expected_version: &str,
    ) -> Result<(), String> {
        let expected_qualified = format!("{}/{}", expected_org, expected_name);

        // 1. Verify packages.tdm if present
        if let Some(manifest) = crate::pkg::manifest::Manifest::from_dir(pkg_dir).map_err(|e| {
            let _ = std::fs::remove_dir_all(pkg_dir);
            format!(
                "Post-fetch verification failed for {}@{}: {}",
                expected_qualified, expected_version, e
            )
        })? {
            // If manifest declares a qualified name, it must match exactly
            if manifest.name.contains('/') && manifest.name != expected_qualified {
                let _ = std::fs::remove_dir_all(pkg_dir);
                return Err(format!(
                    "Post-fetch verification failed: package declares identity '{}' \
                     but was fetched as '{}@{}'. The tarball content does not match \
                     the requested package.",
                    manifest.name, expected_qualified, expected_version
                ));
            }

            // Version must match
            if manifest.version != expected_version {
                let _ = std::fs::remove_dir_all(pkg_dir);
                return Err(format!(
                    "Post-fetch verification failed: package declares version '{}' \
                     but was fetched as '{}@{}'. The tarball content does not match \
                     the requested package.",
                    manifest.version, expected_qualified, expected_version
                ));
            }
        }

        // 2. Verify native/addon.toml if present
        let addon_toml_path = pkg_dir.join("native").join("addon.toml");
        if addon_toml_path.exists() {
            let addon_manifest = crate::addon::manifest::parse_addon_manifest(&addon_toml_path)
                .map_err(|e| {
                    let _ = std::fs::remove_dir_all(pkg_dir);
                    format!(
                        "Post-fetch verification failed for {}@{}: \
                         addon.toml is present but cannot be parsed: {}",
                        expected_qualified, expected_version, e
                    )
                })?;
            if addon_manifest.package != expected_qualified {
                let _ = std::fs::remove_dir_all(pkg_dir);
                return Err(format!(
                    "Post-fetch verification failed: addon.toml declares package '{}' \
                     but was fetched as '{}@{}'. The tarball content does not match \
                     the requested package.",
                    addon_manifest.package, expected_qualified, expected_version
                ));
            }
        }

        Ok(())
    }
}

/// Extract all values of `"name"` keys from a JSON string.
///
/// Handles both pretty-printed and compressed JSON. Avoids matching
/// `"tag_name"`, `"full_name"`, etc. by checking that the character
/// immediately before `"name"` is `{`, `,`, or whitespace (after a comma/brace).
///
/// Returns a Vec of the string values found.
fn extract_json_name_values(json: &str) -> Vec<String> {
    let mut results = Vec::new();
    let bytes = json.as_bytes();
    let len = bytes.len();
    let target = b"\"name\"";
    let target_len = target.len();

    let mut i = 0;
    while i + target_len < len {
        // Look for "name" pattern
        if &bytes[i..i + target_len] == target {
            // Check preceding non-whitespace character is { or , (not part of a longer key)
            let mut j = i;
            while j > 0 {
                j -= 1;
                let c = bytes[j];
                if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                    continue;
                }
                // Must be preceded by { or , (start of object or after another field)
                if c == b'{' || c == b',' {
                    break;
                } else {
                    // Preceded by another character (e.g. part of "tag_name")
                    i += target_len;
                    continue; // Continue outer loop -- use goto-like jump
                }
            }

            // Skip past "name" and optional whitespace/colon
            let mut k = i + target_len;
            while k < len
                && (bytes[k] == b' ' || bytes[k] == b'\t' || bytes[k] == b'\n' || bytes[k] == b'\r')
            {
                k += 1;
            }
            if k < len && bytes[k] == b':' {
                k += 1;
                while k < len
                    && (bytes[k] == b' '
                        || bytes[k] == b'\t'
                        || bytes[k] == b'\n'
                        || bytes[k] == b'\r')
                {
                    k += 1;
                }
                if k < len && bytes[k] == b'"' {
                    k += 1; // skip opening quote
                    let start = k;
                    while k < len && bytes[k] != b'"' {
                        if bytes[k] == b'\\' {
                            k += 1; // skip escaped char
                        }
                        k += 1;
                    }
                    if k < len {
                        let value = String::from_utf8_lossy(&bytes[start..k]).to_string();
                        results.push(value);
                    }
                    i = k + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    results
}

// =============================================================================
// C17-1: store sidecar (`_meta.toml`)
// =============================================================================
//
// `_meta.toml` records provenance metadata next to the extracted tarball so
// `taida install` can later detect stale store entries when a tag is
// republished (retag / delete+recreate).
//
// The sidecar is intentionally a tiny flat TOML document. C17 scope is
// "add sidecar + stale detection"; a richer schema (content-addressable
// layout) is deferred to C18+.
//
// Current schema version: `STORE_META_SCHEMA_VERSION = 1`.
//
// On-disk layout:
// ```toml
// # auto-generated by taida install (C17)
// # Do not edit by hand.
// schema_version = 1
// commit_sha = "0cd5588720ac44e58a01e8f8831a62c023fab5cf"
// tarball_sha256 = "<hex>"
// # tarball_etag = "W/\"...\""  # optional; absent when None
// fetched_at = "2026-04-16T12:20:16Z"
// source = "github:taida-lang/terminal"
// version = "a.1"
// ```

/// Filename for the provenance sidecar placed alongside the extracted
/// tarball. Underscore prefix so it never collides with package files.
pub const STORE_META_FILENAME: &str = "_meta.toml";

/// Current schema version for `_meta.toml`. An older sidecar with a
/// different `schema_version` is treated as "unknown provenance" so the
/// caller (Phase 2) can force a refresh.
pub const STORE_META_SCHEMA_VERSION: u32 = 1;

/// Provenance metadata written alongside an extracted store package.
///
/// Written atomically via `write_meta_atomic` (tempfile + rename) so a
/// crashed install leaves either a complete sidecar or no sidecar at all.
/// Read via `read_meta`; missing sidecar returns `Ok(None)` (Phase 2
/// treats that as pessimistic refresh).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreMeta {
    /// Always `STORE_META_SCHEMA_VERSION` when written by current code.
    pub schema_version: u32,
    /// Commit SHA the version tag pointed at when fetched.
    ///
    /// Empty string means "unknown at fetch time" -- C17-1 writes this
    /// when the resolver has not yet queried the remote HEAD. The Phase 2
    /// decision table treats `commit_sha.is_empty()` as "sidecar present
    /// but SHA unknown" and falls back to pessimistic refresh.
    pub commit_sha: String,
    /// SHA-256 (hex) of the tarball before extraction.
    pub tarball_sha256: String,
    /// HTTP ETag returned by the archive host, if exposed. Optional; the
    /// field is omitted from the on-disk TOML when `None`.
    pub tarball_etag: Option<String>,
    /// RFC-3339 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) of when the tarball
    /// was fetched.
    pub fetched_at: String,
    /// Source identifier, e.g. `"github:taida-lang/terminal"`.
    pub source: String,
    /// Version string as requested, e.g. `"a.1"`.
    pub version: String,
}

/// Errors produced by the C17 store sidecar helpers.
#[derive(Debug)]
pub enum StoreError {
    /// I/O error while reading/writing the sidecar.
    Io(String),
    /// Sidecar could not be parsed (malformed TOML).
    Parse(String),
    /// Sidecar is well-formed but declares a schema version this build
    /// does not understand. Phase 2 treats this as pessimistic-refresh.
    UnknownMetaSchema { actual: u32, expected: u32 },
    /// Sidecar is missing a required key.
    MissingField(&'static str),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(m) => write!(f, "store sidecar I/O error: {}", m),
            StoreError::Parse(m) => write!(f, "store sidecar parse error: {}", m),
            StoreError::UnknownMetaSchema { actual, expected } => write!(
                f,
                "store sidecar schema_version={} unsupported (this build supports {})",
                actual, expected
            ),
            StoreError::MissingField(name) => {
                write!(f, "store sidecar missing required field '{}'", name)
            }
        }
    }
}

impl std::error::Error for StoreError {}

/// Return the sidecar path for an extracted package directory.
pub fn meta_path_for(pkg_dir: &Path) -> PathBuf {
    pkg_dir.join(STORE_META_FILENAME)
}

/// Read and parse a store sidecar.
///
/// Returns `Ok(None)` when the file does not exist (no sidecar is a valid
/// state for pre-C17 installs and is handled by the Phase 2 decision
/// table). Returns `Err` for malformed TOML or schema-version mismatches.
pub fn read_meta(path: &Path) -> Result<Option<StoreMeta>, StoreError> {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StoreError::Io(e.to_string())),
    };
    let meta = parse_meta_str(&source)?;
    Ok(Some(meta))
}

/// Write a store sidecar atomically (write to `<path>.tmp`, then rename).
///
/// The parent directory must already exist (typically the package
/// extraction directory). The temp file is removed on failure so callers
/// never observe a half-written sidecar.
pub fn write_meta_atomic(path: &Path, meta: &StoreMeta) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::Io(format!("sidecar path has no parent: {}", path.display())))?;
    if !parent.exists() {
        std::fs::create_dir_all(parent).map_err(|e| {
            StoreError::Io(format!(
                "cannot create sidecar directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(STORE_META_FILENAME);
    let tmp_path = parent.join(format!(".{}.tmp", file_name));
    // Best-effort cleanup of any leftover tmp from a prior crash.
    let _ = std::fs::remove_file(&tmp_path);

    let serialized = serialize_meta(meta);
    if let Err(e) = std::fs::write(&tmp_path, serialized.as_bytes()) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(StoreError::Io(format!(
            "cannot write temp sidecar {}: {}",
            tmp_path.display(),
            e
        )));
    }

    std::fs::rename(&tmp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        StoreError::Io(format!(
            "cannot atomically place sidecar {}: {}",
            path.display(),
            e
        ))
    })
}

/// Serialize a `StoreMeta` to the on-disk TOML form.
///
/// Kept as a free function (not `Display`) so tests and Phase 2 callers
/// can round-trip without relying on inherent method location.
fn serialize_meta(meta: &StoreMeta) -> String {
    let mut out = String::new();
    out.push_str("# auto-generated by taida install (C17)\n");
    out.push_str("# Do not edit by hand.\n");
    out.push_str(&format!("schema_version = {}\n", meta.schema_version));
    out.push_str(&format!(
        "commit_sha = \"{}\"\n",
        escape_toml_basic_string(&meta.commit_sha)
    ));
    out.push_str(&format!(
        "tarball_sha256 = \"{}\"\n",
        escape_toml_basic_string(&meta.tarball_sha256)
    ));
    if let Some(etag) = &meta.tarball_etag {
        out.push_str(&format!(
            "tarball_etag = \"{}\"\n",
            escape_toml_basic_string(etag)
        ));
    }
    out.push_str(&format!(
        "fetched_at = \"{}\"\n",
        escape_toml_basic_string(&meta.fetched_at)
    ));
    out.push_str(&format!(
        "source = \"{}\"\n",
        escape_toml_basic_string(&meta.source)
    ));
    out.push_str(&format!(
        "version = \"{}\"\n",
        escape_toml_basic_string(&meta.version)
    ));
    out
}

/// Parse a store sidecar from a TOML string.
///
/// Accepts the flat key=value shape produced by `serialize_meta`. Lines
/// starting with `#` are comments. Sections (`[...]`) are rejected --
/// the sidecar schema has no sections in v1.
fn parse_meta_str(source: &str) -> Result<StoreMeta, StoreError> {
    let mut schema_version: Option<u32> = None;
    let mut commit_sha: Option<String> = None;
    let mut tarball_sha256: Option<String> = None;
    let mut tarball_etag: Option<String> = None;
    let mut fetched_at: Option<String> = None;
    let mut source_field: Option<String> = None;
    let mut version: Option<String> = None;

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            return Err(StoreError::Parse(format!(
                "line {}: sections are not allowed in _meta.toml v1",
                line_no
            )));
        }

        let (key, value) = line.split_once('=').ok_or_else(|| {
            StoreError::Parse(format!("line {}: expected 'key = value'", line_no))
        })?;
        let key = key.trim();
        let value = value.trim();

        match key {
            "schema_version" => {
                let n: u32 = value.parse().map_err(|_| {
                    StoreError::Parse(format!(
                        "line {}: schema_version must be a non-negative integer, got {:?}",
                        line_no, value
                    ))
                })?;
                schema_version = Some(n);
            }
            "commit_sha" => commit_sha = Some(parse_basic_string(value, line_no)?),
            "tarball_sha256" => tarball_sha256 = Some(parse_basic_string(value, line_no)?),
            "tarball_etag" => tarball_etag = Some(parse_basic_string(value, line_no)?),
            "fetched_at" => fetched_at = Some(parse_basic_string(value, line_no)?),
            "source" => source_field = Some(parse_basic_string(value, line_no)?),
            "version" => version = Some(parse_basic_string(value, line_no)?),
            other => {
                // Unknown keys are tolerated (forward-compat) but reported
                // in debug builds via the `parse_meta_str` contract being
                // silent -- prefer ignore over error so v1 readers don't
                // reject v1.x sidecars with additive fields.
                let _ = other;
            }
        }
    }

    let schema_version = schema_version.ok_or(StoreError::MissingField("schema_version"))?;
    if schema_version != STORE_META_SCHEMA_VERSION {
        return Err(StoreError::UnknownMetaSchema {
            actual: schema_version,
            expected: STORE_META_SCHEMA_VERSION,
        });
    }

    Ok(StoreMeta {
        schema_version,
        commit_sha: commit_sha.ok_or(StoreError::MissingField("commit_sha"))?,
        tarball_sha256: tarball_sha256.ok_or(StoreError::MissingField("tarball_sha256"))?,
        tarball_etag,
        fetched_at: fetched_at.ok_or(StoreError::MissingField("fetched_at"))?,
        source: source_field.ok_or(StoreError::MissingField("source"))?,
        version: version.ok_or(StoreError::MissingField("version"))?,
    })
}

/// Parse a TOML basic string literal (`"..."`) with minimal escape
/// support: `\\`, `\"`, `\n`, `\r`, `\t`.
fn parse_basic_string(value: &str, line_no: usize) -> Result<String, StoreError> {
    let bytes = value.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
        return Err(StoreError::Parse(format!(
            "line {}: expected a quoted string, got {:?}",
            line_no, value
        )));
    }
    let inner = &value[1..value.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => {
                return Err(StoreError::Parse(format!(
                    "line {}: unsupported escape \\{} in string",
                    line_no, other
                )));
            }
            None => {
                return Err(StoreError::Parse(format!(
                    "line {}: dangling backslash in string",
                    line_no
                )));
            }
        }
    }
    Ok(out)
}

/// Escape a string for a TOML basic string literal.
fn escape_toml_basic_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Compute the SHA-256 (hex) of a file by streaming it through the
/// in-tree hasher. Used by C17-1 to record `tarball_sha256` in the
/// sidecar.
///
/// The tarball is read fully into memory. Addon tarballs are typically
/// tens of KiB to a few MiB, so a single-pass read is acceptable; if this
/// ever grows we can switch to streaming via `Read::read_to_end` chunks.
fn compute_file_sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("cannot read {} for hashing: {}", path.display(), e))?;
    Ok(crate::crypto::sha256_hex_bytes(&bytes))
}

/// Format `SystemTime::now()` as RFC-3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
///
/// Kept as a free function so the C17 sidecar can be written without
/// pulling in a time crate. Precision is whole seconds, matching the
/// granularity `taida install` needs for stale detection.
fn rfc3339_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339_utc(secs)
}

/// Format a Unix-epoch second count as RFC-3339 UTC.
///
/// Implements the civil-calendar arithmetic locally (Howard Hinnant's
/// `days_from_civil` inverse) so we do not need a dependency.
fn format_rfc3339_utc(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let time_of_day = unix_secs % 86_400;
    let hour = time_of_day / 3_600;
    let minute = (time_of_day % 3_600) / 60;
    let second = time_of_day % 60;

    // Howard Hinnant: civil_from_days
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146_096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, m, d, hour, minute, second
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_path_layout() {
        let store = GlobalStore::with_root(PathBuf::from("/tmp/taida_store_test"));
        let path = store.package_path("alice", "webframework", "b.12");
        assert_eq!(
            path,
            PathBuf::from("/tmp/taida_store_test/alice/webframework/b.12")
        );
    }

    #[test]
    fn test_is_cached_false_when_empty() {
        let store = GlobalStore::with_root(PathBuf::from("/tmp/taida_store_test_cached"));
        assert!(!store.is_cached("nonexistent", "pkg", "a.1"));
    }

    #[test]
    fn test_is_cached_true_with_marker() {
        let dir = PathBuf::from("/tmp/taida_store_test_marker");
        let _ = std::fs::remove_dir_all(&dir);
        let pkg_dir = dir.join("alice").join("http").join("b.12");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join(".taida_installed"), "").unwrap();

        let store = GlobalStore::with_root(dir.clone());
        assert!(store.is_cached("alice", "http", "b.12"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_latest_in_generation() {
        let dir = PathBuf::from("/tmp/taida_store_test_gen");
        let _ = std::fs::remove_dir_all(&dir);

        // Create mock versions
        let pkg_parent = dir.join("org").join("pkg");
        for v in &["a.1", "a.5", "a.12", "b.1"] {
            let vdir = pkg_parent.join(v);
            std::fs::create_dir_all(&vdir).unwrap();
            std::fs::write(vdir.join(".taida_installed"), "").unwrap();
        }

        let store = GlobalStore::with_root(dir.clone());
        let result = store.find_latest_in_generation(&pkg_parent, "a");
        assert_eq!(result, Some("a.12".to_string()));

        let result = store.find_latest_in_generation(&pkg_parent, "b");
        assert_eq!(result, Some("b.1".to_string()));

        let result = store.find_latest_in_generation(&pkg_parent, "c");
        assert_eq!(result, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_latest_in_generation_with_labels() {
        let dir = PathBuf::from("/tmp/taida_store_test_gen_label");
        let _ = std::fs::remove_dir_all(&dir);

        let pkg_parent = dir.join("org").join("pkg");
        for v in &["a.1", "a.3.alpha", "a.5.beta"] {
            let vdir = pkg_parent.join(v);
            std::fs::create_dir_all(&vdir).unwrap();
            std::fs::write(vdir.join(".taida_installed"), "").unwrap();
        }

        let store = GlobalStore::with_root(dir.clone());
        let result = store.find_latest_in_generation(&pkg_parent, "a");
        assert_eq!(result, Some("a.5.beta".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_is_cached_with_label() {
        let dir = PathBuf::from("/tmp/taida_store_test_label_cache");
        let _ = std::fs::remove_dir_all(&dir);
        let pkg_dir = dir.join("org").join("pkg").join("a.1.alpha");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join(".taida_installed"), "").unwrap();

        let store = GlobalStore::with_root(dir.clone());
        assert!(store.is_cached("org", "pkg", "a.1.alpha"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_json_name_values_pretty() {
        let json = r#"[
  {
    "name": "va.3",
    "zipball_url": "...",
    "tarball_url": "...",
    "commit": {}
  },
  {
    "name": "va.2",
    "node_id": "abc"
  }
]"#;
        let names = extract_json_name_values(json);
        assert_eq!(names, vec!["va.3", "va.2"]);
    }

    #[test]
    fn test_extract_json_name_values_compressed() {
        let json = r#"[{"name":"va.3","zipball_url":"..."},{"name":"va.1","node_id":"abc"}]"#;
        let names = extract_json_name_values(json);
        assert_eq!(names, vec!["va.3", "va.1"]);
    }

    #[test]
    fn test_extract_json_name_no_false_match() {
        // "tag_name" and "full_name" should NOT be matched
        let json = r#"[{"tag_name":"va.99","full_name":"repo","name":"va.5"}]"#;
        let names = extract_json_name_values(json);
        assert_eq!(names, vec!["va.5"]);
    }

    #[test]
    fn test_extract_json_name_empty() {
        let names = extract_json_name_values("[]");
        assert!(names.is_empty());
    }

    /// FL-29: GlobalStore fallback uses std::env::temp_dir() instead of "/tmp"
    /// Note: This test modifies environment variables and may be flaky under parallel
    /// execution. Run with `cargo test --test-threads=1` if it fails intermittently.
    #[test]
    fn test_global_store_fallback_uses_temp_dir() {
        let _guard = crate::util::env_test_lock().lock().unwrap();

        let original_home = std::env::var("HOME").ok();
        let original_userprofile = std::env::var("USERPROFILE").ok();

        unsafe {
            std::env::remove_var("HOME");
            std::env::remove_var("USERPROFILE");
        }

        let store = GlobalStore::new();
        let expected_root = std::env::temp_dir().join(".taida").join("store");
        assert_eq!(
            store.root, expected_root,
            "GlobalStore fallback should use std::env::temp_dir(), not hardcoded /tmp"
        );

        // Restore environment
        unsafe {
            if let Some(home) = original_home {
                std::env::set_var("HOME", home);
            } else {
                std::env::remove_var("HOME");
            }
            if let Some(up) = original_userprofile {
                std::env::set_var("USERPROFILE", up);
            } else {
                std::env::remove_var("USERPROFILE");
            }
        }
    }

    #[test]
    fn test_verify_fetched_package_matching_manifest() {
        let dir = PathBuf::from("/tmp/taida_verify_match");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("packages.tdm"), "<<<@b.11.rc3 alice/http\n").unwrap();

        let result = GlobalStore::verify_fetched_package(&dir, "alice", "http", "b.11.rc3");
        assert!(
            result.is_ok(),
            "matching manifest should pass: {:?}",
            result
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_verify_fetched_package_identity_mismatch() {
        let dir = PathBuf::from("/tmp/taida_verify_id_mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("packages.tdm"), "<<<@b.11.rc3 evil/hijacked\n").unwrap();

        let result = GlobalStore::verify_fetched_package(&dir, "alice", "http", "b.11.rc3");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Post-fetch verification failed"),
            "should report identity mismatch"
        );
    }

    #[test]
    fn test_verify_fetched_package_version_mismatch() {
        let dir = PathBuf::from("/tmp/taida_verify_ver_mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("packages.tdm"), "<<<@b.99.rc1 alice/http\n").unwrap();

        let result = GlobalStore::verify_fetched_package(&dir, "alice", "http", "b.11.rc3");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Post-fetch verification failed"),
            "should report version mismatch"
        );
    }

    #[test]
    fn test_verify_fetched_package_no_manifest_ok() {
        let dir = PathBuf::from("/tmp/taida_verify_no_manifest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // No packages.tdm — simple packages without manifest should pass
        let result = GlobalStore::verify_fetched_package(&dir, "alice", "http", "b.11.rc3");
        assert!(result.is_ok(), "no manifest should pass: {:?}", result);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_verify_fetched_package_bare_name_ok() {
        let dir = PathBuf::from("/tmp/taida_verify_bare_name");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Legacy manifest with bare name (no org/) — should pass
        // (bare names don't declare a qualified identity to compare against)
        std::fs::write(dir.join("packages.tdm"), "<<<@b.11.rc3\n").unwrap();

        let result = GlobalStore::verify_fetched_package(&dir, "alice", "http", "b.11.rc3");
        assert!(
            result.is_ok(),
            "bare name manifest should pass: {:?}",
            result
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_verify_fetched_package_corrupt_addon_toml_rejected() {
        let dir = PathBuf::from("/tmp/taida_verify_corrupt_addon");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("native")).unwrap();
        std::fs::write(dir.join("packages.tdm"), "<<<@b.11.rc3 alice/http\n").unwrap();
        // Write a corrupt addon.toml that cannot be parsed
        std::fs::write(
            dir.join("native").join("addon.toml"),
            "this is not valid toml {{{\n",
        )
        .unwrap();

        let result = GlobalStore::verify_fetched_package(&dir, "alice", "http", "b.11.rc3");
        assert!(result.is_err(), "corrupt addon.toml should be rejected");
        let err = result.unwrap_err();
        assert!(
            err.contains("Post-fetch verification failed")
                && err.contains("addon.toml is present but cannot be parsed"),
            "error should mention addon.toml parse failure, got: {}",
            err
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // =========================================================================
    // C17-1: store sidecar (`_meta.toml`) unit tests
    // =========================================================================

    fn sample_meta() -> StoreMeta {
        StoreMeta {
            schema_version: STORE_META_SCHEMA_VERSION,
            commit_sha: "0cd5588720ac44e58a01e8f8831a62c023fab5cf".to_string(),
            tarball_sha256:
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            tarball_etag: Some("W/\"abcd-1234\"".to_string()),
            fetched_at: "2026-04-16T12:20:16Z".to_string(),
            source: "github:taida-lang/terminal".to_string(),
            version: "a.1".to_string(),
        }
    }

    fn unique_tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "taida_store_meta_{}_{}_{}",
            tag,
            std::process::id(),
            nanos
        ))
    }

    #[test]
    fn test_meta_path_for_layout() {
        let pkg_dir = PathBuf::from("/tmp/anywhere/alice/http/b.12");
        assert_eq!(
            meta_path_for(&pkg_dir),
            PathBuf::from("/tmp/anywhere/alice/http/b.12/_meta.toml")
        );
    }

    #[test]
    fn test_write_read_roundtrip() {
        let dir = unique_tmp_dir("roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = meta_path_for(&dir);

        let original = sample_meta();
        write_meta_atomic(&path, &original).expect("write_meta_atomic");

        let loaded = read_meta(&path).expect("read_meta ok").expect("exists");
        assert_eq!(loaded, original);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_without_etag_roundtrip() {
        let dir = unique_tmp_dir("no_etag");
        std::fs::create_dir_all(&dir).unwrap();
        let path = meta_path_for(&dir);

        let mut meta = sample_meta();
        meta.tarball_etag = None;
        write_meta_atomic(&path, &meta).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            !contents.contains("tarball_etag"),
            "sidecar should omit tarball_etag when None, got:\n{}",
            contents
        );

        let loaded = read_meta(&path).unwrap().unwrap();
        assert_eq!(loaded.tarball_etag, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_meta_missing_returns_none() {
        let dir = unique_tmp_dir("missing");
        // Note: directory intentionally not created
        let path = meta_path_for(&dir);
        let result = read_meta(&path).expect("missing sidecar is ok");
        assert_eq!(result, None);
    }

    #[test]
    fn test_read_meta_schema_mismatch() {
        let dir = unique_tmp_dir("schema_mismatch");
        std::fs::create_dir_all(&dir).unwrap();
        let path = meta_path_for(&dir);
        // Write a sidecar with an unknown schema_version.
        std::fs::write(
            &path,
            "schema_version = 99\n\
             commit_sha = \"\"\n\
             tarball_sha256 = \"\"\n\
             fetched_at = \"2026-04-16T00:00:00Z\"\n\
             source = \"github:foo/bar\"\n\
             version = \"a.1\"\n",
        )
        .unwrap();

        let err = read_meta(&path).expect_err("schema mismatch must error");
        match err {
            StoreError::UnknownMetaSchema { actual, expected } => {
                assert_eq!(actual, 99);
                assert_eq!(expected, STORE_META_SCHEMA_VERSION);
            }
            other => panic!("expected UnknownMetaSchema, got {:?}", other),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_meta_missing_required_field() {
        let dir = unique_tmp_dir("missing_field");
        std::fs::create_dir_all(&dir).unwrap();
        let path = meta_path_for(&dir);
        // schema_version present, but tarball_sha256 missing.
        std::fs::write(
            &path,
            "schema_version = 1\n\
             commit_sha = \"\"\n\
             fetched_at = \"2026-04-16T00:00:00Z\"\n\
             source = \"github:foo/bar\"\n\
             version = \"a.1\"\n",
        )
        .unwrap();

        let err = read_meta(&path).expect_err("missing field must error");
        match err {
            StoreError::MissingField(name) => assert_eq!(name, "tarball_sha256"),
            other => panic!("expected MissingField, got {:?}", other),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_meta_rejects_sections() {
        let dir = unique_tmp_dir("sections");
        std::fs::create_dir_all(&dir).unwrap();
        let path = meta_path_for(&dir);
        std::fs::write(
            &path,
            "schema_version = 1\n[unexpected]\ncommit_sha = \"\"\n",
        )
        .unwrap();

        let err = read_meta(&path).expect_err("sections are rejected in v1");
        match err {
            StoreError::Parse(m) => {
                assert!(m.contains("sections are not allowed"), "got: {}", m);
            }
            other => panic!("expected Parse, got {:?}", other),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_meta_atomic_no_tmp_file_leftover_on_success() {
        let dir = unique_tmp_dir("atomic_success");
        std::fs::create_dir_all(&dir).unwrap();
        let path = meta_path_for(&dir);

        write_meta_atomic(&path, &sample_meta()).unwrap();

        // The tempfile pattern is `.<name>.tmp`.
        let tmp = dir.join(format!(".{}.tmp", STORE_META_FILENAME));
        assert!(
            !tmp.exists(),
            "tempfile {} should be renamed away",
            tmp.display()
        );
        assert!(path.exists(), "final sidecar should exist");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_meta_atomic_overwrites_stale_tmp() {
        // Simulate a crashed write: a `.{name}.tmp` from a prior install
        // lingers in the directory. `write_meta_atomic` must clean it up
        // and still produce a valid sidecar.
        let dir = unique_tmp_dir("atomic_stale_tmp");
        std::fs::create_dir_all(&dir).unwrap();
        let path = meta_path_for(&dir);
        let tmp = dir.join(format!(".{}.tmp", STORE_META_FILENAME));
        std::fs::write(&tmp, b"garbage from prior crash").unwrap();

        write_meta_atomic(&path, &sample_meta()).unwrap();

        assert!(!tmp.exists(), "stale tempfile must be removed");
        let loaded = read_meta(&path).unwrap().unwrap();
        assert_eq!(loaded, sample_meta());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_meta_atomic_creates_parent_dir_if_missing() {
        // If the package directory somehow does not exist yet,
        // write_meta_atomic should create it rather than fail.
        let dir = unique_tmp_dir("atomic_no_parent");
        let path = meta_path_for(&dir);
        assert!(!dir.exists());
        write_meta_atomic(&path, &sample_meta()).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_escape_and_parse_basic_string_roundtrip() {
        let original = "line one\nline\ttwo \"quoted\" \\backslash\\";
        let serialized = escape_toml_basic_string(original);
        // The serialized form is the inner payload of a basic string;
        // wrap it for round-trip through parse_basic_string.
        let wrapped = format!("\"{}\"", serialized);
        let parsed = parse_basic_string(&wrapped, 1).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn test_format_rfc3339_utc_known_epochs() {
        assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
        // 2026-04-16T12:20:16Z: days_from_1970 =
        //   (56*365 + 14 leap days to 2026-01-01 exclusive of 2024 already
        //    counted) + (31+28+31+15)  -- computed independently and
        //   cross-checked with `date -u -d '2026-04-16T12:20:16Z' +%s`.
        assert_eq!(format_rfc3339_utc(1_776_342_016), "2026-04-16T12:20:16Z");
        // Leap day edge: 2024-02-29T00:00:00Z -> 1_709_164_800
        assert_eq!(format_rfc3339_utc(1_709_164_800), "2024-02-29T00:00:00Z");
        // Pre-epoch-style boundary (first second of 2000): 946_684_800
        assert_eq!(format_rfc3339_utc(946_684_800), "2000-01-01T00:00:00Z");
    }

    #[test]
    fn test_rfc3339_now_matches_format_signature() {
        let now = rfc3339_now();
        // Shape: YYYY-MM-DDTHH:MM:SSZ == 20 chars
        assert_eq!(now.len(), 20, "got {:?}", now);
        assert!(now.ends_with('Z'));
        assert_eq!(&now[4..5], "-");
        assert_eq!(&now[7..8], "-");
        assert_eq!(&now[10..11], "T");
        assert_eq!(&now[13..14], ":");
        assert_eq!(&now[16..17], ":");
    }

    #[test]
    fn test_serialize_meta_emits_generated_header() {
        let out = serialize_meta(&sample_meta());
        assert!(
            out.starts_with("# auto-generated by taida install (C17)\n"),
            "header missing, got:\n{}",
            out
        );
        assert!(out.contains("# Do not edit by hand."));
    }

    #[test]
    fn test_parse_meta_str_tolerates_unknown_forward_compat_fields() {
        // Future sidecar versions may add fields. v1 readers should
        // ignore them rather than reject the sidecar outright.
        let toml = "schema_version = 1\n\
                    commit_sha = \"deadbeef\"\n\
                    tarball_sha256 = \"abcd\"\n\
                    fetched_at = \"2026-04-16T12:20:16Z\"\n\
                    source = \"github:foo/bar\"\n\
                    version = \"a.1\"\n\
                    future_field = \"v1.1 addition\"\n";
        let meta = parse_meta_str(toml).unwrap();
        assert_eq!(meta.commit_sha, "deadbeef");
        assert_eq!(meta.version, "a.1");
    }

    #[test]
    fn test_fetch_and_cache_writes_sidecar_after_extract() {
        // Integration-style test: use a local mock by wiring a pre-built
        // tarball into the extraction path. We cannot easily exercise
        // curl + tar here without network, so we validate the sidecar
        // contract by calling write_meta_atomic directly on a simulated
        // package directory. End-to-end coverage is owned by Phase 5.
        let dir = unique_tmp_dir("fetch_sim");
        std::fs::create_dir_all(&dir).unwrap();
        // Pretend we just extracted a tarball here.
        std::fs::write(dir.join("main.td"), "<<< @(main)\n").unwrap();

        let meta = StoreMeta {
            schema_version: STORE_META_SCHEMA_VERSION,
            commit_sha: "".to_string(), // Phase 1: unknown, Phase 2 fills in
            tarball_sha256:
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            tarball_etag: None,
            fetched_at: rfc3339_now(),
            source: "github:taida-lang/terminal".to_string(),
            version: "a.1".to_string(),
        };
        let meta_path = meta_path_for(&dir);
        write_meta_atomic(&meta_path, &meta).unwrap();

        // Contract: sidecar lives next to the extracted tree.
        assert!(meta_path.exists());
        let loaded = read_meta(&meta_path).unwrap().unwrap();
        assert_eq!(loaded.version, "a.1");
        assert_eq!(loaded.source, "github:taida-lang/terminal");
        assert!(
            loaded.commit_sha.is_empty(),
            "Phase 1 sidecar leaves commit_sha empty; Phase 2 fills it"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compute_file_sha256_matches_known_vector() {
        let dir = unique_tmp_dir("sha256");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("payload.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let hex = compute_file_sha256(&path).unwrap();
        assert_eq!(
            hex,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

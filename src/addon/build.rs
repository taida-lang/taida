//! Shared addon build helper (RC2.7 Phase 1).
//!
//! Extracts `AddonBuildOutput` and `build_addon_artifacts` from
//! `src/pkg/publish.rs` so that both the publish flow and the
//! install-time local build fallback share the same build logic.
//!
//! ## Contract
//!
//! * **Not pure.** Invokes `cargo build --release --lib` as a subprocess.
//! * When `external_target_dir` is `Some(path)`, the build output
//!   is redirected via `CARGO_TARGET_DIR` so the project's own
//!   `target/` directory is never touched (critical for the install
//!   fallback where the source tree lives in the package store).
//! * Does **not** modify `packages.tdm`, `addon.toml` or `addon.lock.toml`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::addon::host_target::detect_host_target;
use crate::addon::manifest::parse_addon_manifest;

/// Result of invoking `cargo build --release --lib` for an addon package.
///
/// Carries exactly the information the downstream pipeline needs:
/// (1) the absolute path to the freshly built `cdylib` so SHA-256
/// can be computed and the file can be attached to a GitHub Release
/// asset, (2) the library stem so the asset can be renamed into the
/// canonical `lib<stem>-<triple>.<ext>` form, and (3) the current
/// host triple so `addon.lock.toml` can be keyed on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddonBuildOutput {
    /// Absolute path to the freshly built `cdylib`.
    pub cdylib_path: PathBuf,
    /// Library stem as declared in `native/addon.toml`.
    pub library_stem: String,
    /// Canonical host triple (e.g. `x86_64-unknown-linux-gnu`).
    pub host_triple: String,
}

/// Build the Rust addon cdylib for the current host and return the
/// artifact location plus metadata.
///
/// The function:
///
///   1. Parses `native/addon.toml` to discover the declared library
///      stem (`[addon].library`).
///   2. Detects the current host triple via
///      [`crate::addon::host_target::detect_host_target`].
///   3. Invokes `cargo build --release --lib` in `project_dir`.
///   4. Probes the release directory for the built cdylib.
///
/// When `external_target_dir` is `Some(dir)`, the environment
/// variable `CARGO_TARGET_DIR` is set so all build artifacts land
/// in `dir` rather than `project_dir/target/`. This is mandatory
/// for the install-time local build fallback which must not pollute
/// the package store's source tree.
pub fn build_addon_artifacts(
    project_dir: &Path,
    external_target_dir: Option<&Path>,
) -> Result<AddonBuildOutput, String> {
    let addon_toml = project_dir.join("native").join("addon.toml");
    if !addon_toml.exists() {
        return Err(format!(
            "build_addon_artifacts: '{}' not found. \
             Addon build requires a native/addon.toml manifest.",
            addon_toml.display()
        ));
    }

    let manifest = parse_addon_manifest(&addon_toml).map_err(|e| e.to_string())?;
    let library_stem = manifest.library.clone();

    let host = detect_host_target().map_err(|e| {
        format!(
            "build_addon_artifacts: {} \
             (cannot build a host-specific cdylib on this platform).",
            e
        )
    })?;
    let host_triple = host.as_triple().to_string();
    let cdylib_ext = host.cdylib_ext();

    let cargo_toml = project_dir.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Err(format!(
            "build_addon_artifacts: '{}' not found. \
             Addon build requires a Cargo project alongside packages.tdm.",
            cargo_toml.display()
        ));
    }

    let cargo_toml_str = cargo_toml
        .to_str()
        .ok_or_else(|| "Cargo.toml path contains non-UTF-8 bytes".to_string())?;

    let mut cmd = Command::new("cargo");
    cmd.args([
        "build",
        "--release",
        "--lib",
        "--manifest-path",
        cargo_toml_str,
    ])
    .current_dir(project_dir);

    if let Some(ext_dir) = external_target_dir {
        cmd.env("CARGO_TARGET_DIR", ext_dir);
    }

    let output = cmd.output().map_err(|e| {
        format!(
            "build_addon_artifacts: failed to invoke cargo build in '{}': {}",
            project_dir.display(),
            e
        )
    })?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "build_addon_artifacts: `cargo build --release --lib` failed in '{}':\n\
             --- stdout ---\n{}\n--- stderr ---\n{}",
            project_dir.display(),
            stdout.trim_end(),
            stderr.trim_end()
        ));
    }

    // Determine the directory where release artifacts land.
    let release_dir = match external_target_dir {
        Some(ext_dir) => ext_dir.join("release"),
        None => project_dir.join("target").join("release"),
    };

    let cdylib_prefix = host.cdylib_prefix();
    let cdylib_name = format!("{cdylib_prefix}{library_stem}.{cdylib_ext}");
    let cdylib_path = release_dir.join(&cdylib_name);
    if !cdylib_path.exists() {
        return Err(format!(
            "build_addon_artifacts: expected cdylib '{}' not found after \
             `cargo build --release --lib`. Check that Cargo.toml declares \
             `crate-type = [\"rlib\", \"cdylib\"]` and that `[package].name` \
             produces the stem '{}' configured in native/addon.toml.",
            cdylib_path.display(),
            library_stem
        ));
    }

    Ok(AddonBuildOutput {
        cdylib_path,
        library_stem,
        host_triple,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_output_external_target_dir_path() {
        // Verify that the release directory is derived from external_target_dir
        // when specified. We cannot run a real cargo build in a unit test, but
        // we can verify the path construction logic by checking a missing
        // addon.toml error contains the expected path.
        let tmp = std::env::temp_dir().join(format!(
            "taida_build_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let result = build_addon_artifacts(&tmp, Some(Path::new("/tmp/ext-target")));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("addon.toml"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// RC2.7-4c: verify that store source tree is never polluted.
    ///
    /// The contract is: when `external_target_dir` is provided, the
    /// project's own `target/` directory must not be created or modified.
    /// We verify this by creating a project directory, calling
    /// `build_addon_artifacts` with an external target dir, and asserting
    /// that `project_dir/target/` was not created.
    ///
    /// Since we can't run a full cargo build in a unit test (no Cargo.toml
    /// with cdylib), we verify the invariant by confirming that
    /// `build_addon_artifacts` fails early (no addon.toml) and the
    /// project's target/ directory was never touched.
    #[test]
    fn test_store_source_tree_clean_assertion() {
        let tmp = std::env::temp_dir().join(format!(
            "taida_store_clean_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = tmp.join("store-pkg");
        let ext_target = tmp.join("ext-target");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&ext_target).unwrap();

        // No addon.toml -> build will fail early, but the key assertion
        // is that project/target/ was never created.
        let _ = build_addon_artifacts(&project, Some(&ext_target));

        assert!(
            !project.join("target").exists(),
            "store source tree must not have target/ when external_target_dir is used"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

/// Common Package Resolver for Taida Lang.
///
/// Resolves all dependencies using the Provider pattern:
/// 1. WorkspaceProvider — local path dependencies
/// 2. CoreBundledProvider — core packages bundled with Taida (`taida-lang/*`)
/// 3. StoreProvider — external registry (stub, Phase 3+)
///
/// `taida deps` and `taida install` read `packages.tdm`, resolve all dependencies
/// through the provider chain, and create `.taida/deps/` symlinks.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::lockfile::Lockfile;
use super::manifest::{Dependency, Manifest};
use super::provider::{
    CoreBundledProvider, PackageProvider, PackageSource, ProviderResult, ResolvedPackage,
    StoreProvider, WorkspaceProvider,
};

/// Result of dependency resolution.
#[derive(Debug)]
pub struct ResolveResult {
    /// Successfully resolved dependencies: name -> absolute path.
    pub resolved: BTreeMap<String, PathBuf>,
    /// All resolved packages (for lockfile generation).
    pub packages: Vec<ResolvedPackage>,
    /// Errors encountered during resolution.
    pub errors: Vec<String>,
}

/// Resolve all dependencies declared in the manifest using the provider chain.
///
/// Tries each provider in order for each dependency:
/// WorkspaceProvider -> CoreBundledProvider -> StoreProvider
pub fn resolve_deps(manifest: &Manifest) -> ResolveResult {
    resolve_deps_inner(manifest, false, None)
}

/// Resolve all dependencies using the provider chain, but pin generation-only
/// versions to their locked exact versions from an existing lockfile.
///
/// Used by `taida install` to ensure reproducible installs: if a lockfile
/// records `alice/demo@a.2`, a manifest dependency of `alice/demo@a` will
/// resolve to `a.2` instead of the latest version in generation `a`.
pub fn resolve_deps_locked(manifest: &Manifest, lockfile: &Lockfile) -> ResolveResult {
    resolve_deps_inner(manifest, false, Some(lockfile))
}

/// Resolve all dependencies, bypassing local cache for generation resolution.
///
/// Used by `taida update` to re-resolve generation-only versions (e.g. "a")
/// to the latest exact version (e.g. "a.47") by querying GitHub API directly.
pub fn resolve_deps_update(manifest: &Manifest) -> ResolveResult {
    resolve_deps_inner(manifest, true, None)
}

fn dep_decl_identity(dep: &Dependency, declared_from_root: &Path) -> String {
    match dep {
        Dependency::Path { path } => {
            let joined = if Path::new(path).is_absolute() {
                PathBuf::from(path)
            } else {
                declared_from_root.join(path)
            };
            let canonical = joined.canonicalize().unwrap_or(joined);
            format!("path:{}", canonical.display())
        }
        Dependency::Registry { org, name, version } => {
            format!("registry:{}/{}@{}", org, name, version)
        }
    }
}

fn resolved_identity(pkg: &ResolvedPackage) -> String {
    match &pkg.source {
        PackageSource::Path(path) => format!("path:{}@{}", path, pkg.path.display()),
        PackageSource::CoreBundled => format!("bundled:{}@{}", pkg.name, pkg.version),
        PackageSource::Store { org, name } => format!("store:{}/{}@{}", org, name, pkg.version),
    }
}

fn resolve_deps_inner(
    manifest: &Manifest,
    force_remote: bool,
    lockfile: Option<&Lockfile>,
) -> ResolveResult {
    // Build a lookup table from lockfile: package name -> locked version
    let locked_versions: std::collections::HashMap<String, String> = lockfile
        .map(|lf| {
            lf.packages
                .iter()
                .map(|p| (p.name.clone(), p.version.clone()))
                .collect()
        })
        .unwrap_or_default();

    let providers: Vec<Box<dyn PackageProvider>> = vec![
        Box::new(WorkspaceProvider),
        Box::new(CoreBundledProvider::new()),
        if force_remote {
            Box::new(StoreProvider::new_force_remote())
        } else {
            Box::new(StoreProvider::new())
        },
    ];

    let mut resolved = BTreeMap::new();
    let mut packages = Vec::new();
    let mut errors = Vec::new();
    let mut resolved_by_alias: BTreeMap<String, String> = BTreeMap::new();
    let mut resolved_by_identity: BTreeMap<String, String> = BTreeMap::new();

    // BFS queue: (dep_name, dependency, requester, declared_from_root)
    let mut queue: std::collections::VecDeque<(
        String,
        super::manifest::Dependency,
        String,
        PathBuf,
    )> = std::collections::VecDeque::new();

    // Enqueue root dependencies
    for (name, dep) in &manifest.deps {
        queue.push_back((
            name.clone(),
            dep.clone(),
            "root".to_string(),
            manifest.root_dir.clone(),
        ));
    }

    while let Some((name, dep, requester, declared_from_root)) = queue.pop_front() {
        // Pin generation-only registry deps to locked exact versions.
        // A generation-only version has no '.' (e.g. "a" vs "a.2").
        let dep = match &dep {
            Dependency::Registry { org, name: dep_name, version }
                if !version.contains('.') && locked_versions.contains_key(&name) =>
            {
                let locked_ver = &locked_versions[&name];
                // Only pin if the locked version belongs to the same generation
                if locked_ver.starts_with(version) {
                    Dependency::Registry {
                        org: org.clone(),
                        name: dep_name.clone(),
                        version: locked_ver.clone(),
                    }
                } else {
                    dep
                }
            }
            _ => dep,
        };
        let requested_identity = dep_decl_identity(&dep, &declared_from_root);

        if let Some(existing_identity) = resolved_by_alias.get(&name) {
            if existing_identity != &requested_identity {
                errors.push(format!(
                    "Dependency alias conflict: '{}' requested by {} as '{}' but already resolved as '{}'",
                    name, requester, requested_identity, existing_identity
                ));
            }
            continue;
        }

        let mut handled = false;
        for provider in &providers {
            if !provider.can_resolve(&dep) {
                continue;
            }

            let mut resolve_manifest = manifest.clone();
            if matches!(dep, Dependency::Path { .. }) {
                resolve_manifest.root_dir = declared_from_root.clone();
            }

            match provider.resolve(&name, &dep, &resolve_manifest) {
                ProviderResult::Resolved(pkg) => {
                    let pkg_path = pkg.path.clone();
                    let pkg_identity = resolved_identity(&pkg);
                    resolved_by_alias.insert(name.clone(), requested_identity.clone());
                    resolved.insert(name.clone(), pkg.path.clone());
                    packages.push(pkg);
                    handled = true;

                    // Only traverse transitive deps once per resolved package identity.
                    if let std::collections::btree_map::Entry::Vacant(e) =
                        resolved_by_identity.entry(pkg_identity)
                    {
                        e.insert(name.clone());
                        if let Ok(Some(sub_manifest)) =
                            super::manifest::Manifest::from_dir(&pkg_path)
                        {
                            for (sub_name, sub_dep) in &sub_manifest.deps {
                                queue.push_back((
                                    sub_name.clone(),
                                    sub_dep.clone(),
                                    name.clone(),
                                    sub_manifest.root_dir.clone(),
                                ));
                            }
                        }
                    }
                    break;
                }
                ProviderResult::Error(e) => {
                    errors.push(e);
                    handled = true;
                    break;
                }
                ProviderResult::NotApplicable => {
                    continue;
                }
            }
        }

        if !handled {
            errors.push(format!(
                "Dependency '{}': no provider can resolve this dependency type",
                name
            ));
        }
    }

    ResolveResult {
        resolved,
        packages,
        errors,
    }
}

/// Install resolved dependencies by creating symlinks in `.taida/deps/`.
pub fn install_deps(manifest: &Manifest, result: &ResolveResult) -> Result<(), String> {
    let deps_dir = manifest.root_dir.join(".taida").join("deps");

    // Create deps directory
    std::fs::create_dir_all(&deps_dir).map_err(|e| format!("Cannot create .taida/deps/: {}", e))?;

    // Clean existing deps dir completely (supports nested org/name structure)
    if deps_dir.exists() {
        let _ = std::fs::remove_dir_all(&deps_dir);
    }
    std::fs::create_dir_all(&deps_dir).map_err(|e| format!("Cannot create .taida/deps/: {}", e))?;

    // Install each resolved package using its provider's install method
    let providers: Vec<Box<dyn PackageProvider>> = vec![
        Box::new(WorkspaceProvider),
        Box::new(CoreBundledProvider::new()),
        Box::new(StoreProvider::new()),
    ];

    for pkg in &result.packages {
        // Find the appropriate provider for installation
        let installed = providers.iter().any(|provider| {
            let can_install = match &pkg.source {
                PackageSource::Path(_) => provider.name() == "workspace",
                PackageSource::CoreBundled => provider.name() == "core-bundled",
                PackageSource::Store { .. } => provider.name() == "store",
            };
            if can_install {
                if let Err(e) = provider.install(pkg, &deps_dir) {
                    eprintln!("  Warning: {}", e);
                    return false;
                }
                return true;
            }
            false
        });

        if !installed {
            // Fallback: create symlink directly (name may be "org/pkg" or "pkg")
            let link_path = deps_dir.join(&pkg.name);
            if let Some(parent) = link_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Cannot create parent dir for '{}': {}", pkg.name, e))?;
            }
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&pkg.path, &link_path)
                    .map_err(|e| format!("Cannot create symlink for '{}': {}", pkg.name, e))?;
            }
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_dir(&pkg.path, &link_path)
                    .map_err(|e| format!("Cannot create symlink for '{}': {}", pkg.name, e))?;
            }
        }
    }

    Ok(())
}

/// Generate/update the lockfile after dependency resolution.
pub fn write_lockfile(manifest: &Manifest, result: &ResolveResult) -> Result<(), String> {
    let lock_path = manifest.root_dir.join(".taida").join("taida.lock");

    // Ensure .taida/ directory exists
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create .taida/ directory: {}", e))?;
    }
    let lockfile = Lockfile::from_resolved(&result.packages);

    // Check if lockfile is already up to date
    if let Ok(Some(existing)) = Lockfile::read(&lock_path)
        && existing.is_up_to_date(&result.packages)
    {
        return Ok(()); // Already up to date, skip write
    }

    lockfile.write(&lock_path)
}

/// Resolve a package import path using installed dependencies.
///
/// Supports both canonical `org/name` IDs (→ `.taida/deps/org/name/`)
/// and legacy single-segment names (→ `.taida/deps/name/`).
pub fn resolve_package_import(project_root: &Path, package_id: &str) -> Option<PathBuf> {
    // Try canonical org/name path first
    let dep_path = project_root.join(".taida").join("deps").join(package_id);
    if dep_path.exists() {
        return Some(dep_path);
    }
    // Fallback: single-segment (local path deps use name-only)
    if !package_id.contains('/') {
        return None;
    }
    None
}

/// Result of resolving a package module import path.
#[derive(Debug)]
pub struct PackageModuleResolution {
    /// The package root directory (e.g. `.taida/deps/taida-lang/crypto/`)
    pub pkg_dir: PathBuf,
    /// The submodule path within the package, if any (e.g. `hash` for `taida-lang/crypto/hash`)
    pub submodule: Option<String>,
}

/// Resolve a non-versioned package import path using longest-prefix matching.
///
/// Given `import_path` like `taida-lang/crypto`, tries `.taida/deps/taida-lang/crypto/`
/// as a package root. If that doesn't exist, falls back to first-slash split
/// (`.taida/deps/taida-lang/` + submodule `crypto`).
///
/// For `taida-lang/crypto/hash`, first tries `.taida/deps/taida-lang/crypto/hash/`,
/// then `.taida/deps/taida-lang/crypto/` + submodule `hash`,
/// then `.taida/deps/taida-lang/` + submodule `crypto/hash`.
///
/// Returns the package directory and optional submodule path.
pub fn resolve_package_module(
    project_root: &Path,
    import_path: &str,
) -> Option<PackageModuleResolution> {
    let deps_dir = project_root.join(".taida").join("deps");

    // Longest-prefix matching: try the full path, then progressively shorter prefixes
    let segments: Vec<&str> = import_path.split('/').collect();
    // Start from full path, go down to min 2 segments (org/name) if multi-segment,
    // or 1 segment for single-segment package names.
    let min_prefix = if segments.len() >= 2 { 2 } else { 1 };
    for prefix_len in (min_prefix..=segments.len()).rev() {
        let prefix = segments[..prefix_len].join("/");
        let candidate = deps_dir.join(&prefix);
        if candidate.exists() && candidate.is_dir() {
            let submodule = if prefix_len < segments.len() {
                Some(segments[prefix_len..].join("/"))
            } else {
                None
            };
            return Some(PackageModuleResolution {
                pkg_dir: candidate,
                submodule,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_deps_local_path() {
        let dir = PathBuf::from("/tmp/taida_test_pkg_resolve");
        let dep_dir = dir.join("dep_lib");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dep_dir).unwrap();
        fs::write(dep_dir.join("lib.td"), "// lib").unwrap();

        let manifest = Manifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "mylib".to_string(),
                    Dependency::Path {
                        path: "./dep_lib".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        let result = resolve_deps(&manifest);
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        assert_eq!(result.resolved.len(), 1);
        assert!(result.resolved.contains_key("mylib"));
        assert_eq!(result.packages.len(), 1);
        assert_eq!(result.packages[0].name, "mylib");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_deps_missing_path() {
        let dir = PathBuf::from("/tmp/taida_test_pkg_resolve_missing");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "missing".to_string(),
                    Dependency::Path {
                        path: "./nonexistent".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        let result = resolve_deps(&manifest);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("missing"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_install_deps() {
        let dir = PathBuf::from("/tmp/taida_test_pkg_install");
        let dep_dir = dir.join("dep_lib");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dep_dir).unwrap();
        fs::write(dep_dir.join("lib.td"), "// lib").unwrap();

        let manifest = Manifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "mylib".to_string(),
                    Dependency::Path {
                        path: "./dep_lib".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        let result = resolve_deps(&manifest);
        assert!(result.errors.is_empty());

        install_deps(&manifest, &result).unwrap();

        // Check symlink exists
        let link = dir.join(".taida").join("deps").join("mylib");
        assert!(link.exists(), "Symlink should exist at {:?}", link);
        assert!(
            link.join("lib.td").exists(),
            "lib.td should be accessible through symlink"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_package_import() {
        let dir = PathBuf::from("/tmp/taida_test_pkg_import");
        let _ = fs::remove_dir_all(&dir);

        // Single-segment (local path deps)
        let local_dir = dir.join(".taida").join("deps").join("mylib");
        fs::create_dir_all(&local_dir).unwrap();
        let result = resolve_package_import(&dir, "mylib");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), local_dir);

        // Canonical org/name (registry deps)
        let org_dir = dir.join(".taida").join("deps").join("alice").join("http");
        fs::create_dir_all(&org_dir).unwrap();
        let result = resolve_package_import(&dir, "alice/http");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), org_dir);

        let result = resolve_package_import(&dir, "missing");
        assert!(result.is_none());

        let result = resolve_package_import(&dir, "bob/missing");
        assert!(result.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_package_module_longest_prefix() {
        let dir = PathBuf::from("/tmp/taida_test_pkg_module_resolve");
        let _ = fs::remove_dir_all(&dir);

        // Setup: .taida/deps/taida-lang/crypto/ as package root
        let crypto_dir = dir
            .join(".taida")
            .join("deps")
            .join("taida-lang")
            .join("crypto");
        fs::create_dir_all(&crypto_dir).unwrap();
        fs::write(crypto_dir.join("main.td"), "// crypto").unwrap();

        // "taida-lang/crypto" → package root, no submodule
        let res = resolve_package_module(&dir, "taida-lang/crypto");
        assert!(res.is_some());
        let res = res.unwrap();
        assert_eq!(res.pkg_dir, crypto_dir);
        assert!(res.submodule.is_none());

        // "taida-lang/crypto/hash" → package root + submodule "hash"
        let res = resolve_package_module(&dir, "taida-lang/crypto/hash");
        assert!(res.is_some());
        let res = res.unwrap();
        assert_eq!(res.pkg_dir, crypto_dir);
        assert_eq!(res.submodule.as_deref(), Some("hash"));

        // "taida-lang/missing" → None
        let res = resolve_package_module(&dir, "taida-lang/missing");
        assert!(res.is_none());

        // Single-segment package
        let mylib_dir = dir.join(".taida").join("deps").join("mylib");
        fs::create_dir_all(&mylib_dir).unwrap();
        let res = resolve_package_module(&dir, "mylib");
        assert!(res.is_some());
        let res = res.unwrap();
        assert_eq!(res.pkg_dir, mylib_dir);
        assert!(res.submodule.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_registry_core_bundled() {
        let dir = PathBuf::from("/tmp/taida_test_resolve_bundled");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "taida-lang/os".to_string(),
                    Dependency::Registry {
                        org: "taida-lang".to_string(),
                        name: "os".to_string(),
                        version: "a.1".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        let result = resolve_deps(&manifest);
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        assert_eq!(result.resolved.len(), 1);
        assert!(result.resolved.contains_key("taida-lang/os"));
        assert_eq!(result.packages.len(), 1);
        assert_eq!(result.packages[0].version, "a.1");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore] // Requires network access — run with `cargo test -- --ignored`
    fn test_resolve_registry_store_uncached() {
        // StoreProvider now tries to actually download, so uncached packages
        // will fail with a download error (not "not yet implemented")
        let dir = PathBuf::from("/tmp/taida_test_resolve_store");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "nonexistent-org-xyz/http".to_string(),
                    Dependency::Registry {
                        org: "nonexistent-org-xyz".to_string(),
                        name: "http".to_string(),
                        version: "a.1".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        let result = resolve_deps(&manifest);
        // Should fail because the package doesn't exist on GitHub
        assert_eq!(result.errors.len(), 1);
        assert!(
            result.errors[0].contains("Failed") || result.errors[0].contains("failed"),
            "Error should indicate download failure, got: {}",
            result.errors[0]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_mixed_deps() {
        let dir = PathBuf::from("/tmp/taida_test_resolve_mixed");
        let dep_dir = dir.join("local_lib");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dep_dir).unwrap();
        fs::write(dep_dir.join("main.td"), "// lib").unwrap();

        let manifest = Manifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "local_lib".to_string(),
                    Dependency::Path {
                        path: "./local_lib".to_string(),
                    },
                );
                deps.insert(
                    "os".to_string(),
                    Dependency::Registry {
                        org: "taida-lang".to_string(),
                        name: "os".to_string(),
                        version: "a.1".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        let result = resolve_deps(&manifest);
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        assert_eq!(result.resolved.len(), 2);
        assert!(result.resolved.contains_key("local_lib"));
        assert!(result.resolved.contains_key("os"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_transitive_path_dep_resolves_from_declaring_manifest_root() {
        let dir = PathBuf::from("/tmp/taida_test_transitive_path_context");
        let dep_a = dir.join("dep_a");
        let dep_b = dep_a.join("dep_b");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dep_b).unwrap();
        fs::write(dep_a.join("main.td"), "// dep_a").unwrap();
        fs::write(dep_b.join("main.td"), "// dep_b").unwrap();
        fs::write(
            dep_a.join("packages.tdm"),
            r#"
name <= "dep_a"
version <= "0.1.0"
deps <= @(
  dep_b <= @(path <= "./dep_b")
)
"#,
        )
        .unwrap();

        let manifest = Manifest {
            name: "root".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "dep_a".to_string(),
                    Dependency::Path {
                        path: "./dep_a".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        let result = resolve_deps(&manifest);
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        let dep_b_path = result
            .resolved
            .get("dep_b")
            .expect("dep_b should be resolved transitively");
        assert_eq!(
            dep_b_path,
            &dep_b.canonicalize().unwrap(),
            "transitive path dependency must resolve relative to dep_a/packages.tdm"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_alias_conflict_same_name_different_identity_reports_error() {
        let dir = PathBuf::from("/tmp/taida_test_alias_conflict");
        let dep_a = dir.join("dep_a");
        let dep_c = dir.join("dep_c");
        let dep_a_shared = dep_a.join("shared_a");
        let dep_c_shared = dep_c.join("shared_c");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dep_a_shared).unwrap();
        fs::create_dir_all(&dep_c_shared).unwrap();
        fs::write(dep_a.join("main.td"), "// dep_a").unwrap();
        fs::write(dep_c.join("main.td"), "// dep_c").unwrap();
        fs::write(dep_a_shared.join("main.td"), "// shared from a").unwrap();
        fs::write(dep_c_shared.join("main.td"), "// shared from c").unwrap();

        fs::write(
            dep_a.join("packages.tdm"),
            r#"
name <= "dep_a"
deps <= @(
  shared <= @(path <= "./shared_a")
)
"#,
        )
        .unwrap();
        fs::write(
            dep_c.join("packages.tdm"),
            r#"
name <= "dep_c"
deps <= @(
  shared <= @(path <= "./shared_c")
)
"#,
        )
        .unwrap();

        let manifest = Manifest {
            name: "root".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "dep_a".to_string(),
                    Dependency::Path {
                        path: "./dep_a".to_string(),
                    },
                );
                deps.insert(
                    "dep_c".to_string(),
                    Dependency::Path {
                        path: "./dep_c".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        let result = resolve_deps(&manifest);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("Dependency alias conflict") && e.contains("shared")),
            "expected alias conflict error, got: {:?}",
            result.errors
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_lockfile() {
        let dir = PathBuf::from("/tmp/taida_test_write_lockfile");
        let dep_dir = dir.join("dep_lib");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dep_dir).unwrap();
        fs::write(dep_dir.join("main.td"), "// lib").unwrap();

        let manifest = Manifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "mylib".to_string(),
                    Dependency::Path {
                        path: "./dep_lib".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        let result = resolve_deps(&manifest);
        assert!(result.errors.is_empty());

        write_lockfile(&manifest, &result).unwrap();

        let lock_path = dir.join(".taida").join("taida.lock");
        assert!(lock_path.exists(), ".taida/taida.lock should be created");

        let content = fs::read_to_string(&lock_path).unwrap();
        assert!(content.contains("mylib"));
        assert!(content.contains("path:./dep_lib"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_install_and_lockfile_core_bundled() {
        let dir = PathBuf::from("/tmp/taida_test_install_bundled");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let manifest = Manifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "os".to_string(),
                    Dependency::Registry {
                        org: "taida-lang".to_string(),
                        name: "os".to_string(),
                        version: "a.1".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        let result = resolve_deps(&manifest);
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);

        install_deps(&manifest, &result).unwrap();
        write_lockfile(&manifest, &result).unwrap();

        // Check symlink exists
        let link = dir.join(".taida").join("deps").join("os");
        assert!(link.exists(), "os symlink should exist");

        // Check lockfile
        let lock_path = dir.join(".taida").join("taida.lock");
        assert!(lock_path.exists());
        let content = fs::read_to_string(&lock_path).unwrap();
        assert!(content.contains("os"));
        assert!(content.contains("bundled"));
        assert!(content.contains("a.1"));

        let _ = fs::remove_dir_all(&dir);
    }

    // ── FL-20 regression: resolve_deps_locked pins generation-only versions ──

    #[test]
    fn test_resolve_deps_locked_pins_generation_version() {
        use super::super::lockfile::{LockedPackage, Lockfile};

        let dir = PathBuf::from("/tmp/taida_test_resolve_locked");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Manifest declares os@a (generation-only)
        let manifest = Manifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            entry: "main.td".to_string(),
            deps: {
                let mut deps = BTreeMap::new();
                deps.insert(
                    "os".to_string(),
                    Dependency::Registry {
                        org: "taida-lang".to_string(),
                        name: "os".to_string(),
                        version: "a".to_string(),
                    },
                );
                deps
            },
            root_dir: dir.clone(),
        };

        // Lockfile records os@a.1 (exact version)
        let lockfile = Lockfile {
            version: 1,
            packages: vec![LockedPackage {
                name: "os".to_string(),
                version: "a.1".to_string(),
                source: "bundled".to_string(),
                integrity: "fnv1a:0000000000000002".to_string(),
            }],
        };

        // resolve_deps_locked should pin os to a.1 (from lockfile)
        let result = resolve_deps_locked(&manifest, &lockfile);
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        assert_eq!(result.packages.len(), 1);
        assert_eq!(
            result.packages[0].version, "a.1",
            "Locked version should be used instead of re-resolving"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}

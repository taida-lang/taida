use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::manifest::{Manifest, is_valid_taida_version};

const DEFAULT_PROPOSALS_REPO: &str = "taida-community/proposals";

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaidaVersionParts {
    generation: String,
    number: Option<u64>,
    label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishPlan {
    pub version: String,
    pub generation: String,
    pub number: u64,
    pub label: Option<String>,
    pub previous_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishPreparation {
    pub package_name: String,
    pub version: String,
    pub integrity: String,
    pub previous_version: Option<String>,
    pub source_repo: Option<String>,
    pub updated_manifest_source: String,
}

pub fn proposals_repo() -> String {
    env::var("TAIDA_PUBLISH_PROPOSALS_REPO").unwrap_or_else(|_| DEFAULT_PROPOSALS_REPO.to_string())
}

pub fn validate_label(label: &str) -> Result<(), String> {
    if is_valid_taida_version(&format!("a.1.{label}")) {
        Ok(())
    } else {
        Err(format!(
            "Invalid publish label '{}'. Expected [a-z0-9][a-z0-9-]* with no trailing hyphen.",
            label
        ))
    }
}

pub fn read_git_tags(project_dir: &Path) -> Result<Vec<String>, String> {
    // Fetch remote tags to ensure local tag list is up to date.
    // Failure here is non-fatal (e.g. no remote configured, offline).
    let _ = Command::new("git")
        .args(["fetch", "--tags", "--quiet"])
        .current_dir(project_dir)
        .output();

    let output = Command::new("git")
        .args(["tag", "--list"])
        .current_dir(project_dir)
        .output()
        .map_err(|e| {
            format!(
                "Failed to run git tag in '{}': {}",
                project_dir.display(),
                e
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "taida publish requires a git repository. git tag failed: {}",
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

pub fn git_origin_url(project_dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(project_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() { None } else { Some(url) }
}

pub fn plan_publish_version(
    manifest_version: &str,
    git_tags: &[String],
    label: Option<&str>,
) -> Result<PublishPlan, String> {
    if let Some(label) = label {
        validate_label(label)?;
    }

    let desired = parse_taida_version(manifest_version);
    let all_versions: Vec<TaidaVersionParts> = git_tags
        .iter()
        .filter_map(|tag| parse_taida_tag(tag))
        .collect();
    let latest = all_versions
        .iter()
        .max_by_key(|version| version.number.unwrap_or(0));

    match latest {
        Some(latest) => {
            let latest_generation = latest.generation.clone();
            let desired_generation = desired
                .as_ref()
                .map(|version| version.generation.clone())
                .unwrap_or_else(|| latest_generation.clone());
            let next_generation_name = next_generation(&latest_generation);

            // Existing generation (patch) or next(latest) (new breaking change)
            let generation_exists = all_versions
                .iter()
                .any(|v| v.generation == desired_generation);

            if !generation_exists && desired_generation != next_generation_name {
                return Err(format!(
                    "Generation '{}' does not exist and is not the next generation '{}'. \
                     To patch an existing generation, use one of: {}. \
                     To introduce a breaking change, use '{}'.",
                    desired_generation,
                    next_generation_name,
                    all_versions
                        .iter()
                        .map(|v| v.generation.as_str())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join(", "),
                    next_generation_name,
                ));
            }

            let number = latest.number.unwrap_or(0) + 1;
            let label = label.map(ToOwned::to_owned);
            Ok(PublishPlan {
                version: format_version(&desired_generation, number, label.as_deref()),
                generation: desired_generation,
                number,
                label,
                previous_version: Some(format_version(
                    &latest.generation,
                    latest.number.unwrap_or(0),
                    latest.label.as_deref(),
                )),
            })
        }
        None => {
            let generation = desired
                .as_ref()
                .map(|version| version.generation.clone())
                .unwrap_or_else(|| "a".to_string());
            let number = desired
                .as_ref()
                .and_then(|version| version.number)
                .unwrap_or(1);
            let label = label
                .map(ToOwned::to_owned)
                .or_else(|| desired.as_ref().and_then(|version| version.label.clone()));
            Ok(PublishPlan {
                version: format_version(&generation, number, label.as_deref()),
                generation,
                number,
                label,
                previous_version: None,
            })
        }
    }
}

pub fn rewrite_export_version(source: &str, new_version: &str) -> Result<String, String> {
    let export_pos = source.find("<<<").ok_or_else(|| {
        "packages.tdm must contain an export (`<<<`) before publishing.".to_string()
    })?;
    let after_export = &source[export_pos + 3..];

    if let Some(rest) = after_export.strip_prefix('@') {
        let version_len = rest
            .chars()
            .take_while(|ch| {
                ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '.' || *ch == '-'
            })
            .count();
        let version_start = export_pos + 4;
        let version_end = version_start + version_len;
        if version_len == 0 {
            return Err("packages.tdm export version is malformed.".to_string());
        }
        let mut updated = String::with_capacity(source.len() + new_version.len());
        updated.push_str(&source[..version_start]);
        updated.push_str(new_version);
        updated.push_str(&source[version_end..]);
        Ok(updated)
    } else {
        let mut updated = String::with_capacity(source.len() + new_version.len() + 1);
        updated.push_str(&source[..export_pos + 3]);
        updated.push('@');
        updated.push_str(new_version);
        updated.push_str(&source[export_pos + 3..]);
        Ok(updated)
    }
}

pub fn compute_publish_integrity(project_dir: &Path) -> String {
    let mut files = Vec::new();
    collect_publish_files(project_dir, project_dir, &mut files);
    files.sort();

    let mut state: u64 = 0xcbf29ce484222325;
    for file in files {
        if let Ok(rel) = file.strip_prefix(project_dir) {
            for byte in rel.to_string_lossy().as_bytes() {
                state ^= *byte as u64;
                state = state.wrapping_mul(0x100000001b3);
            }
        }
        // Separator between filename and content to prevent collision
        // e.g. file "ab" + content "cd" vs file "abc" + content "d"
        state ^= 0xff;
        state = state.wrapping_mul(0x100000001b3);
        if let Ok(content) = std::fs::read(&file) {
            for byte in &content {
                state ^= *byte as u64;
                state = state.wrapping_mul(0x100000001b3);
            }
        }
        // Separator between files
        state ^= 0x00;
        state = state.wrapping_mul(0x100000001b3);
    }

    format!("fnv1a:{state:016x}")
}

pub fn check_worktree_clean(project_dir: &Path) -> Result<(), String> {
    let status = run_git(project_dir, &["status", "--porcelain"])?;
    if !status.is_empty() {
        return Err(format!(
            "Working tree has uncommitted changes. Commit or stash them before publishing.\n{}",
            status
        ));
    }
    Ok(())
}

pub fn prepare_publish(
    project_dir: &Path,
    manifest: &Manifest,
    packages_source: &str,
    _author: &str,
    label: Option<&str>,
) -> Result<PublishPreparation, String> {
    validate_package_name(&manifest.name)?;
    check_worktree_clean(project_dir)?;

    let git_tags = read_git_tags(project_dir)?;
    let plan = plan_publish_version(&manifest.version, &git_tags, label)?;
    let updated_manifest_source = rewrite_export_version(packages_source, &plan.version)?;
    let source_repo = git_origin_url(project_dir);

    if let Some(repo) = &source_repo
        && let Some((_owner, repo_name)) = parse_github_repo(repo)
        && repo_name != manifest.name
    {
        eprintln!(
            "Warning: package name '{}' does not match git remote repository '{}'. This is allowed for forks.",
            manifest.name, repo_name
        );
    }

    let integrity = compute_publish_integrity(project_dir);

    Ok(PublishPreparation {
        package_name: manifest.name.clone(),
        version: plan.version,
        integrity,
        previous_version: plan.previous_version,
        source_repo,
        updated_manifest_source,
    })
}

/// Git commit + tag + push を実行する。
///
/// Performs operations in a safe order with rollback on failure:
/// 1. Check if the tag already exists on remote (fail early)
/// 2. Stage + commit + tag locally
/// 3. Push commit + tag
/// 4. On push failure, rollback local commit and tag
pub fn git_commit_tag_push(
    project_dir: &Path,
    version: &str,
    package_name: &str,
) -> Result<(), String> {
    let tag = version.to_string();
    let tag_ref = format!("refs/tags/{tag}");

    // Pre-check: verify the tag does not already exist on remote
    if let Ok(output) = Command::new("git")
        .args(["ls-remote", "--tags", "origin", &tag_ref])
        .current_dir(project_dir)
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.status.success() && !stdout.trim().is_empty() {
            return Err(format!(
                "Tag '{}' already exists on remote. Cannot publish duplicate version.",
                tag
            ));
        }
    }

    // Stage packages.tdm
    run_git(project_dir, &["add", "packages.tdm"])?;

    // Commit
    let message = format!("publish: {package_name}@{version}");
    run_git(project_dir, &["commit", "-m", &message])?;

    // Tag
    if let Err(e) = run_git(project_dir, &["tag", &tag]) {
        // Rollback: undo the commit
        let _ = run_git(project_dir, &["reset", "--soft", "HEAD~1"]);
        return Err(e);
    }

    // Push commit
    if let Err(e) = run_git(project_dir, &["push", "origin", "HEAD"]) {
        // Rollback: delete local tag, undo commit
        let _ = run_git(project_dir, &["tag", "-d", &tag]);
        let _ = run_git(project_dir, &["reset", "--soft", "HEAD~1"]);
        return Err(e);
    }

    // Push tag
    if let Err(e) = run_git(project_dir, &["push", "origin", &tag_ref]) {
        // Rollback: revert the pushed commit, delete local tag
        let _ = run_git(project_dir, &["tag", "-d", &tag]);
        let _ = run_git(project_dir, &["revert", "HEAD", "--no-edit"]);
        let _ = run_git(project_dir, &["push", "origin", "HEAD"]);
        return Err(format!(
            "Tag push failed (commit was pushed but tag was not): {}",
            e
        ));
    }

    Ok(())
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("Failed to run git {}: {}", args.join(" "), e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {} failed: {}", args.join(" "), stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// taida-community/proposals への Issue 作成用 pre-filled URL を生成する。
pub fn proposals_url(author: &str, package_name: &str, version: &str, integrity: &str) -> String {
    let title = format!("publish: {author}/{package_name}@{version}");
    let body = format!(
        "## Publish Request\n\n- author: `{author}`\n- package: `{package_name}`\n- version: `@{version}`\n- integrity: `{integrity}`\n"
    );
    let repo = proposals_repo();
    format!(
        "https://github.com/{repo}/issues/new?title={}&body={}",
        urlencoded(&title),
        urlencoded(&body),
    )
}

fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
    }
    out
}

fn parse_taida_version(version: &str) -> Option<TaidaVersionParts> {
    if !is_valid_taida_version(version) {
        return None;
    }

    let mut parts = version.split('.');
    let generation = parts.next()?.to_string();
    let next = parts.next();
    let last = parts.next();

    match (next, last) {
        (None, None) => Some(TaidaVersionParts {
            generation,
            number: None,
            label: None,
        }),
        (Some(number), None) => Some(TaidaVersionParts {
            generation,
            number: number.parse().ok(),
            label: None,
        }),
        (Some(number), Some(label)) => Some(TaidaVersionParts {
            generation,
            number: number.parse().ok(),
            label: Some(label.to_string()),
        }),
        _ => None,
    }
}

fn parse_taida_tag(tag: &str) -> Option<TaidaVersionParts> {
    // Accept both "a.1" and legacy "va.1" tags
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let parsed = parse_taida_version(version)?;
    parsed.number?;
    Some(parsed)
}

fn next_generation(generation: &str) -> String {
    let mut chars: Vec<u8> = generation.bytes().collect();
    let mut index = chars.len();
    let mut carry = true;

    while index > 0 && carry {
        index -= 1;
        if chars[index] == b'z' {
            chars[index] = b'a';
        } else {
            chars[index] += 1;
            carry = false;
        }
    }

    if carry {
        chars.insert(0, b'a');
    }

    String::from_utf8(chars).unwrap_or_else(|_| "a".to_string())
}

fn format_version(generation: &str, number: u64, label: Option<&str>) -> String {
    match label {
        Some(label) => format!("{generation}.{number}.{label}"),
        None => format!("{generation}.{number}"),
    }
}

fn validate_package_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Package name must not be empty.".to_string());
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(format!(
            "Invalid package name '{}'. Package names must not start or end with '-'.",
            name
        ));
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(format!(
            "Invalid package name '{}'. Expected lowercase letters, digits, and hyphens only.",
            name
        ));
    }
    Ok(())
}

fn parse_github_repo(remote: &str) -> Option<(String, String)> {
    if let Some(rest) = remote.strip_prefix("git@github.com:") {
        return parse_owner_repo(rest);
    }
    if let Some(rest) = remote.strip_prefix("ssh://git@github.com/") {
        return parse_owner_repo(rest);
    }
    if let Some(rest) = remote.strip_prefix("https://github.com/") {
        return parse_owner_repo(rest);
    }
    None
}

fn parse_owner_repo(rest: &str) -> Option<(String, String)> {
    let trimmed = rest.trim_end_matches(".git").trim_end_matches('/');
    let mut parts = trimmed.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

fn collect_publish_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if should_skip_path(root, &path) {
            continue;
        }
        if path.is_dir() {
            collect_publish_files(root, &path, out);
        } else {
            out.push(path);
        }
    }
}

fn should_skip_path(root: &Path, path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    if path.is_dir() && matches!(name, ".git" | ".taida" | "target" | "node_modules") {
        return true;
    }
    if let Ok(relative) = path.strip_prefix(root) {
        return relative.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some(".git" | ".taida" | "target" | "node_modules")
            )
        });
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_publish_version_same_generation() {
        let plan = plan_publish_version("a.3", &["va.3".to_string()], None).unwrap();
        assert_eq!(plan.version, "a.4");
        assert_eq!(plan.previous_version, Some("a.3".to_string()));
    }

    #[test]
    fn test_plan_publish_version_allows_manual_generation_bump() {
        let plan = plan_publish_version("b", &["va.3".to_string()], Some("alpha")).unwrap();
        assert_eq!(plan.version, "b.4.alpha");
        assert_eq!(plan.generation, "b");
    }

    #[test]
    fn test_plan_publish_version_rejects_skipped_generation() {
        let err = plan_publish_version("c", &["va.3".to_string()], None).unwrap_err();
        assert!(err.contains("does not exist and is not the next generation"));
    }

    #[test]
    fn test_plan_publish_version_initial_publish_uses_explicit_version() {
        let plan = plan_publish_version("b.7.beta", &[], None).unwrap();
        assert_eq!(plan.version, "b.7.beta");
        assert_eq!(plan.previous_version, None);
    }

    #[test]
    fn test_rewrite_export_version_replaces_existing_version() {
        let source = ">>> taida-lang/os@a.1\n<<<@a.3 @(run)\n";
        let updated = rewrite_export_version(source, "a.4.rc").unwrap();
        assert!(updated.contains("<<<@a.4.rc @(run)"));
    }

    #[test]
    fn test_rewrite_export_version_adds_missing_version() {
        let source = ">>> taida-lang/os@a.1\n<<< @(run)\n";
        let updated = rewrite_export_version(source, "a.1").unwrap();
        assert!(updated.contains("<<<@a.1 @(run)"));
    }

    #[test]
    fn test_compute_publish_integrity_ignores_git_and_taida_dirs() {
        let dir = std::env::temp_dir().join(format!(
            "taida_publish_hash_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join(".taida")).unwrap();
        std::fs::write(dir.join("packages.tdm"), "<<<@(run)\n").unwrap();
        std::fs::write(dir.join(".git").join("config"), "secret").unwrap();
        std::fs::write(dir.join(".taida").join("taida.lock"), "lock").unwrap();

        let hash1 = compute_publish_integrity(&dir);
        std::fs::write(dir.join(".git").join("config"), "changed-secret").unwrap();
        std::fs::write(dir.join(".taida").join("taida.lock"), "changed-lock").unwrap();
        let hash2 = compute_publish_integrity(&dir);

        assert_eq!(hash1, hash2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_github_repo_https_and_ssh() {
        assert_eq!(
            parse_github_repo("https://github.com/taida-community/proposals.git"),
            Some(("taida-community".to_string(), "proposals".to_string()))
        );
        assert_eq!(
            parse_github_repo("git@github.com:taida-community/proposals.git"),
            Some(("taida-community".to_string(), "proposals".to_string()))
        );
    }

    // ── Layer 1: validate_package_name ──

    #[test]
    fn test_validate_package_name_valid() {
        assert!(validate_package_name("my-package").is_ok());
        assert!(validate_package_name("http").is_ok());
        assert!(validate_package_name("a1b2").is_ok());
    }

    #[test]
    fn test_validate_package_name_empty() {
        assert!(validate_package_name("").is_err());
    }

    #[test]
    fn test_validate_package_name_leading_trailing_hyphen() {
        assert!(validate_package_name("-pkg").is_err());
        assert!(validate_package_name("pkg-").is_err());
    }

    #[test]
    fn test_validate_package_name_uppercase_rejected() {
        assert!(validate_package_name("MyPkg").is_err());
    }

    #[test]
    fn test_validate_package_name_special_chars_rejected() {
        assert!(validate_package_name("my_pkg").is_err());
        assert!(validate_package_name("my.pkg").is_err());
        assert!(validate_package_name("my/pkg").is_err());
    }

    // ── Layer 1: validate_label ──

    #[test]
    fn test_validate_label_valid() {
        assert!(validate_label("alpha").is_ok());
        assert!(validate_label("rc-1").is_ok());
        assert!(validate_label("beta2").is_ok());
    }

    #[test]
    fn test_validate_label_invalid() {
        assert!(validate_label("Alpha").is_err());
        assert!(validate_label("-bad").is_err());
        assert!(validate_label("bad-").is_err());
    }

    // ── Layer 1: integrity separator collision ──

    #[test]
    fn test_integrity_different_for_boundary_shift() {
        // "file_a" + content "bc" vs "file_ab" + content "c"
        // Without separator these could collide; with separator they must differ.
        let dir1 = std::env::temp_dir().join(format!(
            "taida_integrity_sep1_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir2 = std::env::temp_dir().join(format!(
            "taida_integrity_sep2_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir1).unwrap();
        std::fs::create_dir_all(&dir2).unwrap();

        std::fs::write(dir1.join("ab"), "cd").unwrap();
        std::fs::write(dir2.join("a"), "bcd").unwrap();

        let h1 = compute_publish_integrity(&dir1);
        let h2 = compute_publish_integrity(&dir2);
        assert_ne!(
            h1, h2,
            "Different file name/content boundaries must produce different hashes"
        );

        let _ = std::fs::remove_dir_all(&dir1);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    // ── Layer 1: should_skip_path with node_modules ──

    #[test]
    fn test_integrity_ignores_node_modules() {
        let dir = std::env::temp_dir().join(format!(
            "taida_publish_nm_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("main.td"), "stdout(1)\n").unwrap();
        std::fs::write(dir.join("node_modules").join("dep.js"), "big blob").unwrap();

        let hash1 = compute_publish_integrity(&dir);
        std::fs::write(dir.join("node_modules").join("dep.js"), "changed blob").unwrap();
        let hash2 = compute_publish_integrity(&dir);

        assert_eq!(
            hash1, hash2,
            "node_modules changes must not affect integrity"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Layer 1: rewrite_export_version edge cases ──

    #[test]
    fn test_rewrite_export_version_no_export_errors() {
        let source = ">>> taida-lang/os@a.1\nsome code\n";
        assert!(rewrite_export_version(source, "a.2").is_err());
    }

    // ── Layer 1: next_generation ──

    #[test]
    fn test_next_generation() {
        assert_eq!(next_generation("a"), "b");
        assert_eq!(next_generation("z"), "aa");
        assert_eq!(next_generation("az"), "ba");
    }

    // ── Layer 1: plan_publish_version — generation does not reset number ──

    #[test]
    fn test_plan_publish_generation_bump_continues_number() {
        // a.10 → b should produce b.11, not b.1
        let plan = plan_publish_version("b", &["va.10".to_string()], None).unwrap();
        assert_eq!(plan.version, "b.11");
        assert_eq!(plan.number, 11);
    }

    // ── Manual tag interference scenarios ──

    #[test]
    fn test_manual_tag_same_generation_large_number() {
        // Someone manually tags b.100; next publish should be b.101
        let plan =
            plan_publish_version("b", &["b.3".to_string(), "b.100".to_string()], None).unwrap();
        assert_eq!(plan.version, "b.101");
        assert_eq!(plan.number, 101);
    }

    #[test]
    fn test_manual_tag_different_generation_patch_allowed() {
        // Tags: b.3, c.50; packages.tdm says @b
        // b is an existing generation → patch allowed, number = 51
        let plan =
            plan_publish_version("b", &["b.3".to_string(), "c.50".to_string()], None).unwrap();
        assert_eq!(plan.version, "b.51");
        assert_eq!(plan.generation, "b");
    }

    #[test]
    fn test_manual_tag_nonexistent_generation_rejected() {
        // Tags: a.1, c.50; packages.tdm says @d
        // d does not exist and next(c) = d → allowed (new breaking change)
        let plan =
            plan_publish_version("d", &["a.1".to_string(), "c.50".to_string()], None).unwrap();
        assert_eq!(plan.version, "d.51");

        // But @e is rejected (next(c) = d, not e)
        let err =
            plan_publish_version("e", &["a.1".to_string(), "c.50".to_string()], None).unwrap_err();
        assert!(err.contains("does not exist"));
    }

    #[test]
    fn test_manual_tag_non_taida_version_ignored() {
        // Non-Taida tags (semver, random) should be ignored by parse_taida_tag
        let plan = plan_publish_version(
            "b",
            &[
                "b.3".to_string(),
                "v99.0.0".to_string(),
                "release-2026".to_string(),
                "foo".to_string(),
            ],
            None,
        )
        .unwrap();
        assert_eq!(plan.version, "b.4");
    }

    // ── Label scenarios ──

    #[test]
    fn test_label_on_old_generation_patch() {
        // Tags: a.1, b.3; publish @a --label hotfix → a.4.hotfix
        let plan =
            plan_publish_version("a", &["a.1".to_string(), "b.3".to_string()], Some("hotfix"))
                .unwrap();
        assert_eq!(plan.version, "a.4.hotfix");
        assert_eq!(plan.generation, "a");
    }

    #[test]
    fn test_label_on_new_generation() {
        // Tags: a.3; publish @b --label alpha → b.4.alpha
        let plan = plan_publish_version("b", &["a.3".to_string()], Some("alpha")).unwrap();
        assert_eq!(plan.version, "b.4.alpha");
        assert_eq!(plan.generation, "b");
    }

    #[test]
    fn test_label_with_manual_tag_different_generation() {
        // Tags: a.1, c.50; publish @a --label patch → a.51.patch
        let plan =
            plan_publish_version("a", &["a.1".to_string(), "c.50".to_string()], Some("patch"))
                .unwrap();
        assert_eq!(plan.version, "a.51.patch");
        assert_eq!(plan.generation, "a");
    }
}

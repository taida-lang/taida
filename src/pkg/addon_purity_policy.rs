//! Project/global policy for F48 addon worker purity decisions.

use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddonPurityMode {
    Deny,
    AllowAudited,
    AllowDeclared,
}

impl AddonPurityMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::AllowAudited => "allow audited",
            Self::AllowDeclared => "allow declared",
        }
    }

    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "deny" => Ok(Self::Deny),
            "allow audited" => Ok(Self::AllowAudited),
            "allow declared" => Ok(Self::AllowDeclared),
            other => Err(format!(
                "[E1630] invalid addon purity policy '{}' (allowed: deny, allow audited, allow declared)",
                other
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddonPurityPolicy {
    pub mode: AddonPurityMode,
    overrides: BTreeSet<String>,
}

impl Default for AddonPurityPolicy {
    fn default() -> Self {
        Self {
            mode: AddonPurityMode::AllowAudited,
            overrides: BTreeSet::new(),
        }
    }
}

impl AddonPurityPolicy {
    pub fn is_override_trusted(&self, package_id: &str, function: &str) -> bool {
        self.overrides
            .contains(&format!("{}::{}", package_id, function))
    }

    pub fn allows_declared(&self) -> bool {
        matches!(self.mode, AddonPurityMode::AllowDeclared)
    }

    pub fn allows_audited(&self) -> bool {
        matches!(
            self.mode,
            AddonPurityMode::AllowAudited | AddonPurityMode::AllowDeclared
        )
    }
}

/// Resolve active addon-purity policy.
///
/// Project policy in `packages.tdm` wins over global `~/.taida/config.toml`;
/// when neither exists, F48 defaults to `allow audited`.
pub fn load_addon_purity_policy(project_root: &Path) -> Result<AddonPurityPolicy, String> {
    if let Some(policy) = read_project_policy(project_root)? {
        return Ok(policy);
    }
    if let Some(policy) = read_global_policy()? {
        return Ok(policy);
    }
    Ok(AddonPurityPolicy::default())
}

fn read_project_policy(project_root: &Path) -> Result<Option<AddonPurityPolicy>, String> {
    let path = project_root.join("packages.tdm");
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read '{}': {}", path.display(), e)),
    };
    parse_addon_purity_policy_from_source(&source)
}

fn read_global_policy() -> Result<Option<AddonPurityPolicy>, String> {
    let home = match crate::util::taida_home_dir() {
        Ok(home) => home,
        Err(_) => return Ok(None),
    };
    let path = home.join(".taida").join("config.toml");
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read '{}': {}", path.display(), e)),
    };
    parse_addon_purity_policy_from_source(&source)
}

pub fn parse_addon_purity_policy_from_source(
    source: &str,
) -> Result<Option<AddonPurityPolicy>, String> {
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Section {
        None,
        Parallelism,
        Overrides,
    }

    let mut section = Section::None;
    let mut seen_parallelism = false;
    let mut mode: Option<AddonPurityMode> = None;
    let mut overrides = BTreeSet::new();

    for raw in source.lines() {
        let line = strip_inline_comment(raw).trim().to_string();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line.starts_with('[') {
            section = match line.as_str() {
                "[parallelism]" => {
                    seen_parallelism = true;
                    Section::Parallelism
                }
                "[parallelism.addon_purity_overrides]" => {
                    seen_parallelism = true;
                    Section::Overrides
                }
                _ => Section::None,
            };
            continue;
        }

        match section {
            Section::None => continue,
            Section::Parallelism => {
                let (key, value) = parse_key_value(&line, "parallelism")?;
                match key.as_str() {
                    "addon_purity" => {
                        mode = Some(AddonPurityMode::parse(&value)?);
                    }
                    other => {
                        return Err(format!(
                            "[E1630] unknown [parallelism] key '{}' (expected addon_purity)",
                            other
                        ));
                    }
                }
            }
            Section::Overrides => {
                let (key, value) = parse_key_value(&line, "parallelism.addon_purity_overrides")?;
                if value != "trusted" {
                    return Err(format!(
                        "[E1630] addon purity override '{}' must be \"trusted\"",
                        key
                    ));
                }
                validate_override_key(&key)?;
                overrides.insert(key);
            }
        }
    }

    if !seen_parallelism {
        return Ok(None);
    }

    Ok(Some(AddonPurityPolicy {
        mode: mode.unwrap_or(AddonPurityMode::AllowAudited),
        overrides,
    }))
}

fn parse_key_value(line: &str, table: &str) -> Result<(String, String), String> {
    let Some((key_raw, value_raw)) = line.split_once('=') else {
        return Err(format!(
            "[E1630] invalid [{}] line '{}' (expected key = \"value\")",
            table, line
        ));
    };
    let key = parse_key(key_raw.trim()).ok_or_else(|| {
        format!(
            "[E1630] invalid [{}] key '{}' (expected plain or quoted key)",
            table,
            key_raw.trim()
        )
    })?;
    let value = parse_quoted_string(value_raw.trim()).ok_or_else(|| {
        format!(
            "[E1630] invalid [{}] value for '{}' (expected plain quoted string)",
            table, key
        )
    })?;
    Ok((key, value))
}

fn parse_key(raw: &str) -> Option<String> {
    if let Some(quoted) = parse_quoted_string(raw) {
        return Some(quoted);
    }
    if raw.is_empty()
        || !raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some(raw.to_string())
}

fn parse_quoted_string(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    if inner.contains('\\') {
        return None;
    }
    Some(inner.to_string())
}

fn validate_override_key(key: &str) -> Result<(), String> {
    let Some((package, function)) = key.split_once("::") else {
        return Err(format!(
            "[E1630] addon purity override '{}' must be '<org>/<name>::<function>'",
            key
        ));
    };
    if package.split('/').count() < 2
        || package.starts_with('/')
        || package.ends_with('/')
        || function.is_empty()
        || !function
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(format!(
            "[E1630] addon purity override '{}' must be '<org>/<name>::<function>'",
            key
        ));
    }
    Ok(())
}

fn strip_inline_comment(raw: &str) -> &str {
    let mut in_string = false;
    for (idx, ch) in raw.char_indices() {
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if !in_string && ch == '#' {
            return &raw[..idx];
        }
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_project_policy_with_function_override() {
        let source = r#"
[parallelism]
addon_purity = "allow declared"

[parallelism.addon_purity_overrides]
"example/math::fast_sum" = "trusted"
"#;
        let policy = parse_addon_purity_policy_from_source(source)
            .expect("policy parse")
            .expect("policy present");
        assert_eq!(policy.mode, AddonPurityMode::AllowDeclared);
        assert!(policy.is_override_trusted("example/math", "fast_sum"));
    }

    #[test]
    fn absent_policy_returns_none() {
        assert!(
            parse_addon_purity_policy_from_source("<<<@a.1 x/y\n")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn rejects_invalid_policy_mode() {
        let err = parse_addon_purity_policy_from_source(
            r#"
[parallelism]
addon_purity = "trust me"
"#,
        )
        .unwrap_err();
        assert!(err.contains("[E1630]"));
    }

    #[test]
    fn rejects_malformed_override() {
        let err = parse_addon_purity_policy_from_source(
            r#"
[parallelism.addon_purity_overrides]
"bad" = "trusted"
"#,
        )
        .unwrap_err();
        assert!(err.contains("[E1630]"));
    }
}

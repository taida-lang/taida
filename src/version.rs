/// Return the Taida release version.
///
/// In release builds, `TAIDA_RELEASE_TAG` (e.g. "@a.7.beta") is set by CI.
/// In dev builds, falls back to `CARGO_PKG_VERSION` from Cargo.toml.
pub fn taida_version() -> &'static str {
    option_env!("TAIDA_RELEASE_TAG").unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taida_version_falls_back_to_cargo_pkg_version() {
        let version = taida_version();
        assert!(!version.is_empty());
        // Without TAIDA_RELEASE_TAG set, should equal CARGO_PKG_VERSION
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
    }
}

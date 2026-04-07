//! Host target detection for prebuild addon distribution.
//!
//! Uses `target_lexicon::HOST` to detect the current platform and maps
//! it to a canonical `HostTarget` enum. Only 5 targets are supported in v1.

use std::fmt;

/// Canonical host targets supported in ABI v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostTarget {
    X86_64LinuxGnu,
    Aarch64LinuxGnu,
    X86_64MacOs,
    Aarch64MacOs,
    X86_64Windows,
}

impl HostTarget {
    /// Returns the canonical target triple string.
    pub fn as_triple(&self) -> &'static str {
        match self {
            Self::X86_64LinuxGnu => "x86_64-unknown-linux-gnu",
            Self::Aarch64LinuxGnu => "aarch64-unknown-linux-gnu",
            Self::X86_64MacOs => "x86_64-apple-darwin",
            Self::Aarch64MacOs => "aarch64-apple-darwin",
            Self::X86_64Windows => "x86_64-pc-windows-msvc",
        }
    }

    /// Parse a triple string into a HostTarget. Only exact canonical forms are accepted.
    pub fn from_triple(triple: &str) -> Option<Self> {
        match triple {
            "x86_64-unknown-linux-gnu" => Some(Self::X86_64LinuxGnu),
            "aarch64-unknown-linux-gnu" => Some(Self::Aarch64LinuxGnu),
            "x86_64-apple-darwin" => Some(Self::X86_64MacOs),
            "aarch64-apple-darwin" => Some(Self::Aarch64MacOs),
            "x86_64-pc-windows-msvc" => Some(Self::X86_64Windows),
            _ => None,
        }
    }

    /// Returns the cdylib extension for this target's OS.
    pub fn cdylib_ext(&self) -> &'static str {
        match self {
            Self::X86_64LinuxGnu | Self::Aarch64LinuxGnu => "so",
            Self::X86_64MacOs | Self::Aarch64MacOs => "dylib",
            Self::X86_64Windows => "dll",
        }
    }

    /// Returns the OS name for error messages.
    pub fn os_name(&self) -> &'static str {
        match self {
            Self::X86_64LinuxGnu | Self::Aarch64LinuxGnu => "linux",
            Self::X86_64MacOs | Self::Aarch64MacOs => "macos",
            Self::X86_64Windows => "windows",
        }
    }
}

/// Error when the host target is not supported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedHost {
    pub host_triple: String,
}

impl fmt::Display for UnsupportedHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unsupported host target '{}' (not available for prebuild addons)",
            self.host_triple
        )
    }
}

/// Returns all supported target triples.
pub fn supported_targets() -> &'static [&'static str] {
    &[
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ]
}

/// Detects the current host target.
#[cfg(feature = "native")]
pub fn detect_host_target() -> Result<HostTarget, UnsupportedHost> {
    use std::sync::OnceLock;

    static HOST: OnceLock<Result<HostTarget, UnsupportedHost>> = OnceLock::new();

    HOST.get_or_init(|| {
        let triple = target_lexicon::HOST.to_string();
        HostTarget::from_triple(&triple).ok_or(UnsupportedHost {
            host_triple: triple,
        })
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_target_as_triple() {
        assert_eq!(
            HostTarget::X86_64LinuxGnu.as_triple(),
            "x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            HostTarget::Aarch64LinuxGnu.as_triple(),
            "aarch64-unknown-linux-gnu"
        );
        assert_eq!(HostTarget::X86_64MacOs.as_triple(), "x86_64-apple-darwin");
        assert_eq!(HostTarget::Aarch64MacOs.as_triple(), "aarch64-apple-darwin");
        assert_eq!(
            HostTarget::X86_64Windows.as_triple(),
            "x86_64-pc-windows-msvc"
        );
    }

    #[test]
    fn host_target_from_triple() {
        assert_eq!(
            HostTarget::from_triple("x86_64-unknown-linux-gnu"),
            Some(HostTarget::X86_64LinuxGnu)
        );
        assert_eq!(
            HostTarget::from_triple("aarch64-unknown-linux-gnu"),
            Some(HostTarget::Aarch64LinuxGnu)
        );
        assert_eq!(
            HostTarget::from_triple("x86_64-apple-darwin"),
            Some(HostTarget::X86_64MacOs)
        );
        assert_eq!(
            HostTarget::from_triple("aarch64-apple-darwin"),
            Some(HostTarget::Aarch64MacOs)
        );
        assert_eq!(
            HostTarget::from_triple("x86_64-pc-windows-msvc"),
            Some(HostTarget::X86_64Windows)
        );
    }

    #[test]
    fn host_target_from_triple_rejects_non_canonical() {
        assert_eq!(HostTarget::from_triple("arm64-apple-darwin"), None);
        assert_eq!(HostTarget::from_triple("x86_64-linux-gnu"), None);
        assert_eq!(HostTarget::from_triple("riscv64-unknown-linux-gnu"), None);
    }

    #[test]
    fn host_target_cdylib_ext() {
        assert_eq!(HostTarget::X86_64LinuxGnu.cdylib_ext(), "so");
        assert_eq!(HostTarget::Aarch64LinuxGnu.cdylib_ext(), "so");
        assert_eq!(HostTarget::X86_64MacOs.cdylib_ext(), "dylib");
        assert_eq!(HostTarget::Aarch64MacOs.cdylib_ext(), "dylib");
        assert_eq!(HostTarget::X86_64Windows.cdylib_ext(), "dll");
    }

    #[test]
    fn supported_targets_count() {
        assert_eq!(supported_targets().len(), 5);
    }

    #[cfg(feature = "native")]
    #[test]
    fn detect_host_target_returns_ok() {
        let result = detect_host_target();
        assert!(
            result.is_ok(),
            "detect_host_target should succeed on supported hosts, got: {:?}",
            result
        );
    }
}

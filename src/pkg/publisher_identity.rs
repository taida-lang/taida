const KNOWN_PUBLISHERS: &[&str] = &["taida-lang"];

pub(crate) fn confusable_known_publisher(login: &str) -> Option<&'static str> {
    let canonical_login = canonical_publisher_login(login);
    let skeleton = publisher_skeleton(canonical_login);
    KNOWN_PUBLISHERS
        .iter()
        .copied()
        .find(|known| canonical_login != *known && skeleton == publisher_skeleton(known))
}

fn canonical_publisher_login(login: &str) -> &str {
    login.strip_suffix("[bot]").unwrap_or(login)
}

fn publisher_skeleton(login: &str) -> String {
    let mut out = String::with_capacity(login.len());
    for ch in login.chars() {
        match ch.to_ascii_lowercase() {
            '-' | '_' => {}
            '0' => out.push('o'),
            '1' | 'i' => out.push('l'),
            '3' => out.push('e'),
            '4' => out.push('a'),
            '5' => out.push('s'),
            '7' => out.push('t'),
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_publisher_homographs() {
        assert_eq!(confusable_known_publisher("taida-lang"), None);
        assert_eq!(confusable_known_publisher("taida-lang[bot]"), None);
        assert_eq!(confusable_known_publisher("taida-1ang"), Some("taida-lang"));
        assert_eq!(
            confusable_known_publisher("taida-1ang[bot]"),
            Some("taida-lang")
        );
        assert_eq!(confusable_known_publisher("taidalang"), Some("taida-lang"));
        assert_eq!(
            confusable_known_publisher("taidalang[bot]"),
            Some("taida-lang")
        );
        assert_eq!(confusable_known_publisher("alice"), None);
    }
}

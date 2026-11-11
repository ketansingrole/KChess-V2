//! Diagnostics redaction: every log a user may hand over (the app's diagnostics export, the
//! shared service's `logs/service.log`, the command line's stderr) passes through `redact`.

use std::sync::OnceLock;

use regex::Regex;

/// The home folder, bearer tokens, Lichess tokens, credential fields and
/// email addresses.
pub fn redact(text: &str) -> String {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            (r"(?i)\bBearer\s+[A-Za-z0-9._~+/-]+=*", "Bearer [redacted]"),
            (r"\b(?:lip|lio)_[A-Za-z0-9_-]+\b", "[redacted]"),
            (
                r#"(?i)(["']?(?:access_token|refresh_token|id_token|token|password|authorization|cookie|code_verifier|code_challenge|client_secret|verifier|challenge|code|state)["']?\s*[:=]\s*)(["']?)[^\s,"'&}]+(["']?)"#,
                "${1}${2}[redacted]${3}",
            ),
            (r"[\w.+-]+@[\w-]+\.[\w.]+", "[redacted]"),
        ]
        .into_iter()
        .filter_map(|(pattern, with)| Regex::new(pattern).ok().map(|rule| (rule, with)))
        .collect()
    });
    let mut text = match dirs::home_dir() {
        Some(home) if !home.as_os_str().is_empty() => text.replace(&*home.to_string_lossy(), "~"),
        _ => text.to_string(),
    };
    for (rule, with) in rules {
        text = rule.replace_all(&text, *with).into_owned();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn secrets_never_reach_the_log() {
        let line = redact(
            "GET /api/account Authorization: Bearer lip_abc123 token=xyz \"password\": \"hunter2\" by me@example.com",
        );
        assert!(!line.contains("abc123"), "{line}");
        assert!(!line.contains("xyz"), "{line}");
        assert!(!line.contains("hunter2"), "{line}");
        assert!(!line.contains("example.com"), "{line}");
        assert!(line.contains("GET /api/account"));
        // Usernames stay, so logs can still be matched to an account.
        assert!(redact("synced @KChessTester").contains("KChessTester"));
    }
}

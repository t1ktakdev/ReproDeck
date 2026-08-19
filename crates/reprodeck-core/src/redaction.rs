use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RedactionResult {
    Included(String),
    Redacted { reason: String },
    Excluded { reason: String },
}

fn bearer_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)bearer\s+[A-Za-z0-9\-\._~\+\/]+=*")
            .expect("static bearer regex must compile")
    })
}

fn key_value_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(password|passwd|token|secret|api[_-]?key|authorization|cookie)\s*[=:]\s*[^\s,;]+")
            .expect("static key/value regex must compile")
    })
}

fn jwt_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b")
            .expect("static JWT regex must compile")
    })
}

fn aws_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b")
            .expect("static AWS access-key regex must compile")
    })
}

fn github_token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\bgh(?:p|o|u|s|r)_[A-Za-z0-9]{20,}\b")
            .expect("static GitHub token regex must compile")
    })
}

fn long_hex_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b[0-9a-fA-F]{40,128}\b").expect("static long-hex regex must compile")
    })
}

fn long_token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z0-9_\-]{48,}\b").expect("static long-token regex must compile")
    })
}

/// Redact common credential forms before text is persisted in receipts or
/// text evidence. The raw input is never returned on a matching secret span.
pub fn redact_text(input: &str) -> String {
    let mut value = bearer_regex()
        .replace_all(input, "Bearer [REDACTED]")
        .into_owned();
    value = key_value_regex()
        .replace_all(&value, "$1=[REDACTED]")
        .into_owned();
    value = jwt_regex()
        .replace_all(&value, "[REDACTED_JWT]")
        .into_owned();
    value = aws_key_regex()
        .replace_all(&value, "[REDACTED_AWS_KEY]")
        .into_owned();
    value = github_token_regex()
        .replace_all(&value, "[REDACTED_GITHUB_TOKEN]")
        .into_owned();
    value = long_hex_regex()
        .replace_all(&value, "[REDACTED_TOKEN]")
        .into_owned();
    long_token_regex()
        .replace_all(&value, "[REDACTED_TOKEN]")
        .into_owned()
}

pub fn redact_path(path: &Path) -> RedactionResult {
    let display = path.to_string_lossy();
    let filename = path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let secret_patterns = [
        ".env",
        ".pem",
        ".key",
        ".pfx",
        ".p12",
        "id_rsa",
        "id_ed25519",
        "credentials",
        "secrets",
    ];
    for pattern in secret_patterns {
        if filename.contains(pattern) {
            return RedactionResult::Redacted {
                reason: format!("filename matches secret pattern: {pattern}"),
            };
        }
    }
    RedactionResult::Included(display.into_owned())
}

pub fn redact_env(key: &str, value: &str) -> RedactionResult {
    let upper = key.to_ascii_uppercase();
    let sensitive = [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "API_KEY",
        "APIKEY",
        "AUTHORIZATION",
        "COOKIE",
        "PRIVATE_KEY",
    ];
    for marker in sensitive {
        if upper.contains(marker) {
            return RedactionResult::Redacted {
                reason: format!("environment name contains sensitive marker: {marker}"),
            };
        }
    }
    if redact_text(value) != value {
        return RedactionResult::Redacted {
            reason: "environment value resembles a credential".to_string(),
        };
    }
    RedactionResult::Included(value.to_string())
}

pub fn artifact_store_path(session: &str, data: &[u8]) -> String {
    let hash = hex::encode(Sha256::digest(data));
    format!("artifacts/{session}/{hash}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_paths_detect_secret_files() {
        assert!(matches!(
            redact_path(Path::new(".env")),
            RedactionResult::Redacted { .. }
        ));
        assert!(matches!(
            redact_path(Path::new("config.pem")),
            RedactionResult::Redacted { .. }
        ));
        assert!(matches!(
            redact_path(Path::new("README.md")),
            RedactionResult::Included(_)
        ));
    }

    #[test]
    fn redact_env_detects_name_and_value_secrets() {
        assert!(matches!(
            redact_env("TOKEN", "secret"),
            RedactionResult::Redacted { .. }
        ));
        assert!(matches!(
            redact_env("MY_VAR", "hello"),
            RedactionResult::Included(_)
        ));
        assert!(matches!(
            redact_env("MY_VAR", "Bearer abcdef123456"),
            RedactionResult::Redacted { .. }
        ));
    }

    #[test]
    fn redact_text_covers_common_secret_shapes() {
        let github = "ghp_abcdefghijklmnopqrstuvwxyz123456";
        let jwt = "abcdefgh.ijklmnop.qrstuvwx";
        let aws = "AKIAABCDEFGHIJKLMNOP";
        let input = format!(
            "Authorization=Bearer secret-token password=hunter2 {github} {jwt} {aws}"
        );
        let redacted = redact_text(&input);
        assert!(!redacted.contains("secret-token"));
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains(github));
        assert!(!redacted.contains(jwt));
        assert!(!redacted.contains(aws));
        assert!(redacted.contains("REDACTED"));
    }
}

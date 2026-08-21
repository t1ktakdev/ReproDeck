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

fn regex(pattern: &'static str, slot: &'static OnceLock<Regex>) -> &'static Regex {
    slot.get_or_init(|| Regex::new(pattern).expect("static redaction regex"))
}

pub fn redact_text(input: &str) -> String {
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static KEY_VALUE: OnceLock<Regex> = OnceLock::new();
    static JWT: OnceLock<Regex> = OnceLock::new();
    static AWS: OnceLock<Regex> = OnceLock::new();
    static GITHUB: OnceLock<Regex> = OnceLock::new();
    static LONG_HEX: OnceLock<Regex> = OnceLock::new();
    static LONG_TOKEN: OnceLock<Regex> = OnceLock::new();

    let mut value = regex(r"(?i)bearer\s+[A-Za-z0-9\-\._~\+\/]+=*", &BEARER)
        .replace_all(input, "Bearer [REDACTED]")
        .into_owned();
    value = regex(
        r"(?i)(password|passwd|token|secret|api[_-]?key|authorization|cookie)\s*[=:]\s*[^\s,;]+",
        &KEY_VALUE,
    )
    .replace_all(&value, "$1=[REDACTED]")
    .into_owned();
    value = regex(
        r"\b[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b",
        &JWT,
    )
    .replace_all(&value, "[REDACTED_JWT]")
    .into_owned();
    value = regex(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", &AWS)
        .replace_all(&value, "[REDACTED_AWS_KEY]")
        .into_owned();
    value = regex(r"\bgh(?:p|o|u|s|r)_[A-Za-z0-9]{20,}\b", &GITHUB)
        .replace_all(&value, "[REDACTED_GITHUB_TOKEN]")
        .into_owned();
    value = regex(r"\b[0-9a-fA-F]{40,128}\b", &LONG_HEX)
        .replace_all(&value, "[REDACTED_TOKEN]")
        .into_owned();
    regex(r"\b[A-Za-z0-9_\-]{48,}\b", &LONG_TOKEN)
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
    fn path_rules_cover_common_secret_files() {
        for name in [
            ".env",
            ".env.local",
            "config.pem",
            "id_ed25519",
            "client.p12",
        ] {
            assert!(matches!(
                redact_path(Path::new(name)),
                RedactionResult::Redacted { .. }
            ));
        }
        assert!(matches!(
            redact_path(Path::new("README.md")),
            RedactionResult::Included(_)
        ));
    }

    #[test]
    fn text_redaction_removes_common_tokens() {
        let input = "password=hunter2 Authorization=Bearer abcdefghijklmnop ghp_abcdefghijklmnopqrstuvwxyz123456 AKIAABCDEFGHIJKLMNOP";
        let redacted = redact_text(input);
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("abcdefghijklmnopqrstuvwxyz123456"));
        assert!(!redacted.contains("AKIAABCDEFGHIJKLMNOP"));
        assert!(redacted.contains("REDACTED"));
    }
}

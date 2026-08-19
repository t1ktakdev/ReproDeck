use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Serialize)]
pub enum RedactionResult {
    Included(String),
    Redacted { reason: String },
    Excluded { reason: String },
}

pub fn redact_path(path: &Path) -> RedactionResult {
    let s = path.to_string_lossy();
    let filename = path
        .file_name()
        .map(|v| v.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let secret_patterns = [".env", ".pem", ".key", "id_rsa", "credentials", "secrets"];
    for p in &secret_patterns {
        if filename.contains(p) {
            return RedactionResult::Redacted {
                reason: format!("filename matches secret pattern: {}", p),
            };
        }
    }
    RedactionResult::Included(s.to_string())
}

pub fn redact_env(key: &str, value: &str) -> RedactionResult {
    let k = key.to_uppercase();
    let sensitive = [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "API_KEY",
        "AUTHORIZATION",
        "COOKIE",
    ];
    for s in &sensitive {
        if k.contains(s) {
            return RedactionResult::Redacted {
                reason: format!("env name contains sensitive token: {}", s),
            };
        }
    }
    // additional detection for authorization bearer
    let auth_re = Regex::new(r"(?i)bearer\s+[A-Za-z0-9\-\._~\+\/]+=*").unwrap();
    if auth_re.is_match(value) {
        return RedactionResult::Redacted {
            reason: "authorization bearer token".to_string(),
        };
    }
    RedactionResult::Included(value.to_string())
}

pub fn artifact_store_path(session: &str, data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hex::encode(hasher.finalize());
    format!("artifacts/{}/{}", session, hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_paths_detects_env() {
        let r = redact_path(Path::new(".env"));
        assert!(matches!(r, RedactionResult::Redacted { .. }));
        let r2 = redact_path(Path::new("config.pem"));
        assert!(matches!(r2, RedactionResult::Redacted { .. }));
        let r3 = redact_path(Path::new("README.md"));
        assert!(matches!(r3, RedactionResult::Included(_)));
    }

    #[test]
    fn redact_env_detects_tokens() {
        let r = redact_env("TOKEN", "secret");
        assert!(matches!(r, RedactionResult::Redacted { .. }));
        let r2 = redact_env("MY_VAR", "hello");
        assert!(matches!(r2, RedactionResult::Included(_)));
        let r3 = redact_env("Authorization", "Bearer abcdef");
        assert!(matches!(r3, RedactionResult::Redacted { .. }));
    }
}

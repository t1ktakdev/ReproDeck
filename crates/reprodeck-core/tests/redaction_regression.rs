use reprodeck_core::redaction::{artifact_store_path, redact_env, redact_text, RedactionResult};

#[test]
fn sensitive_environment_names_are_redacted_case_insensitively() {
    for key in [
        "github_token",
        "Api_Key",
        "database_password",
        "private_key_path",
    ] {
        assert!(matches!(
            redact_env(key, "harmless-value"),
            RedactionResult::Redacted { .. }
        ));
    }
}

#[test]
fn credential_like_environment_values_are_redacted() {
    let result = redact_env("APP_CONFIG", "password=hunter2");

    assert!(matches!(result, RedactionResult::Redacted { .. }));
}

#[test]
fn ordinary_environment_values_remain_available() {
    assert_eq!(
        redact_env("APP_MODE", "development"),
        RedactionResult::Included("development".to_string())
    );
}

#[test]
fn artifact_store_paths_are_deterministic_and_session_scoped() {
    let first = artifact_store_path("session-a", b"same payload");
    let repeated = artifact_store_path("session-a", b"same payload");
    let different_payload = artifact_store_path("session-a", b"different payload");
    let different_session = artifact_store_path("session-b", b"same payload");

    assert_eq!(first, repeated);
    assert_ne!(first, different_payload);
    assert_ne!(first, different_session);
    assert!(first.starts_with("artifacts/session-a/"));
}

#[test]
fn normal_text_is_not_changed_by_redaction() {
    let input = "ReproDeck session completed successfully";
    assert_eq!(redact_text(input), input);
}

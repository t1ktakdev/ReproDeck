use serde::{Deserialize, Serialize};
use std::path::Path;

/// Permission level for potentially unsafe actions (commands, file access).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Permission {
    /// Always allow without prompting.
    Allow,
    /// Ask the user before the action. This is the safe default.
    #[default]
    Ask,
    /// Always deny.
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionReason {
    Configured,
    HardDeniedPrivilegeEscalation,
    UnsafeVerificationCommand,
    OpaqueShellCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDecision {
    pub permission: Permission,
    pub reason: PermissionReason,
    pub explanation: String,
}

impl PermissionDecision {
    fn configured(permission: Permission) -> Self {
        Self {
            permission,
            reason: PermissionReason::Configured,
            explanation: "Configured command permission applies.".to_string(),
        }
    }
}

fn executable_name(executable: &str) -> String {
    let path = Path::new(executable);
    let name = path
        .file_stem()
        .or_else(|| path.file_name())
        .and_then(|value| value.to_str())
        .unwrap_or(executable);
    name.to_ascii_lowercase()
}

fn first_arg(args: &[String]) -> Option<&str> {
    args.iter()
        .map(String::as_str)
        .find(|arg| !arg.trim().is_empty() && !arg.starts_with('-'))
}

/// Apply the additional policy used specifically for BEFORE/AFTER verification.
/// Verification is evidence-gathering, so it must not silently mutate Git
/// history, publish changes, escalate privileges, or hide arbitrary commands in
/// an opaque shell string.
///
/// The configured permission is still authoritative for ordinary safe commands:
/// `Ask` remains Ask and `Deny` remains Deny. Only a configured `Allow` can be
/// reduced by the verification safety policy below.
pub fn verification_command_permission(
    executable: &str,
    args: &[String],
    configured: Permission,
) -> PermissionDecision {
    if configured != Permission::Allow {
        return PermissionDecision::configured(configured);
    }

    let executable = executable_name(executable);

    if matches!(executable.as_str(), "sudo" | "doas" | "pkexec" | "runas") {
        return PermissionDecision {
            permission: Permission::Deny,
            reason: PermissionReason::HardDeniedPrivilegeEscalation,
            explanation: "Privilege escalation is not allowed from verification.".to_string(),
        };
    }

    if matches!(
        executable.as_str(),
        "sh" | "bash" | "zsh" | "fish" | "cmd" | "powershell" | "pwsh"
    ) {
        return PermissionDecision {
            permission: Permission::Ask,
            reason: PermissionReason::OpaqueShellCommand,
            explanation: "Shell-wrapped verification commands require explicit approval."
                .to_string(),
        };
    }

    if executable == "git" {
        let subcommand = first_arg(args).unwrap_or("").to_ascii_lowercase();
        if matches!(
            subcommand.as_str(),
            "push"
                | "commit"
                | "reset"
                | "clean"
                | "checkout"
                | "switch"
                | "merge"
                | "rebase"
                | "cherry-pick"
                | "revert"
                | "tag"
                | "branch"
                | "worktree"
                | "stash"
        ) {
            return PermissionDecision {
                permission: Permission::Ask,
                reason: PermissionReason::UnsafeVerificationCommand,
                explanation: format!(
                    "`git {subcommand}` may mutate or publish repository state and requires explicit approval."
                ),
            };
        }
    }

    PermissionDecision::configured(Permission::Allow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn ask_and_deny_are_never_upgraded() {
        assert_eq!(
            verification_command_permission("cargo", &args(&["test"]), Permission::Ask).permission,
            Permission::Ask
        );
        assert_eq!(
            verification_command_permission("cargo", &args(&["test"]), Permission::Deny).permission,
            Permission::Deny
        );
    }

    #[test]
    fn ordinary_test_command_can_use_configured_allow() {
        let decision = verification_command_permission(
            "cargo",
            &args(&["test", "auth::refresh"]),
            Permission::Allow,
        );
        assert_eq!(decision.permission, Permission::Allow);
        assert_eq!(decision.reason, PermissionReason::Configured);
    }

    #[test]
    fn privilege_escalation_is_hard_denied() {
        for executable in ["sudo", "doas", "pkexec", "runas.exe"] {
            let decision = verification_command_permission(
                executable,
                &args(&["anything"]),
                Permission::Allow,
            );
            assert_eq!(decision.permission, Permission::Deny);
            assert_eq!(
                decision.reason,
                PermissionReason::HardDeniedPrivilegeEscalation
            );
        }
    }

    #[test]
    fn opaque_shell_requires_approval() {
        let decision = verification_command_permission(
            "powershell.exe",
            &args(&["-Command", "Remove-Item -Recurse ."]),
            Permission::Allow,
        );
        assert_eq!(decision.permission, Permission::Ask);
        assert_eq!(decision.reason, PermissionReason::OpaqueShellCommand);
    }

    #[test]
    fn mutating_git_commands_require_approval() {
        for subcommand in ["push", "commit", "reset", "clean", "rebase", "worktree"] {
            let decision = verification_command_permission(
                "git.exe",
                &args(&[subcommand]),
                Permission::Allow,
            );
            assert_eq!(decision.permission, Permission::Ask, "{subcommand}");
            assert_eq!(decision.reason, PermissionReason::UnsafeVerificationCommand);
        }
    }

    #[test]
    fn read_only_git_commands_can_run_when_allowed() {
        for subcommand in ["status", "diff", "rev-parse", "show"] {
            let decision = verification_command_permission(
                "git",
                &args(&[subcommand]),
                Permission::Allow,
            );
            assert_eq!(decision.permission, Permission::Allow, "{subcommand}");
        }
    }
}

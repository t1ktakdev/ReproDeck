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
    ExplicitApproval,
    HardDeniedPrivilegeEscalation,
    HardDeniedVerificationMutation,
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

    fn explicitly_approved() -> Self {
        Self {
            permission: Permission::Allow,
            reason: PermissionReason::ExplicitApproval,
            explanation: "The user explicitly approved this verification command once.".to_string(),
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

fn is_privilege_escalation(executable: &str) -> bool {
    matches!(executable, "sudo" | "doas" | "pkexec" | "runas")
}

fn is_shell(executable: &str) -> bool {
    matches!(
        executable,
        "sh" | "bash" | "zsh" | "fish" | "cmd" | "powershell" | "pwsh"
    )
}

fn is_mutating_git(subcommand: &str) -> bool {
    matches!(
        subcommand,
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
    )
}

/// Evaluate a BEFORE/AFTER verification command.
///
/// Verification is evidence gathering, not a repository mutation mechanism.
/// Privilege escalation and mutating/publishing Git operations are therefore
/// hard-denied even when a caller claims explicit approval. Opaque shell
/// wrappers and configured `Ask` rules can be satisfied by a one-shot explicit
/// approval, while configured `Deny` remains authoritative.
pub fn verification_command_permission_with_approval(
    executable: &str,
    args: &[String],
    configured: Permission,
    explicitly_approved_once: bool,
) -> PermissionDecision {
    let executable = executable_name(executable);

    if is_privilege_escalation(&executable) {
        return PermissionDecision {
            permission: Permission::Deny,
            reason: PermissionReason::HardDeniedPrivilegeEscalation,
            explanation: "Privilege escalation is not allowed from verification.".to_string(),
        };
    }

    if executable == "git" {
        let subcommand = first_arg(args).unwrap_or("").to_ascii_lowercase();
        if is_mutating_git(&subcommand) {
            return PermissionDecision {
                permission: Permission::Deny,
                reason: PermissionReason::HardDeniedVerificationMutation,
                explanation: format!(
                    "`git {subcommand}` is not permitted from verification because verification must not mutate or publish repository state."
                ),
            };
        }
    }

    if configured == Permission::Deny {
        return PermissionDecision::configured(Permission::Deny);
    }

    if is_shell(&executable) && !explicitly_approved_once {
        return PermissionDecision {
            permission: Permission::Ask,
            reason: PermissionReason::OpaqueShellCommand,
            explanation: "Shell-wrapped verification commands require explicit one-shot approval."
                .to_string(),
        };
    }

    match configured {
        Permission::Allow => PermissionDecision::configured(Permission::Allow),
        Permission::Ask if explicitly_approved_once => PermissionDecision::explicitly_approved(),
        Permission::Ask => PermissionDecision::configured(Permission::Ask),
        Permission::Deny => PermissionDecision::configured(Permission::Deny),
    }
}

/// Evaluate a verification command without an explicit one-shot approval.
pub fn verification_command_permission(
    executable: &str,
    args: &[String],
    configured: Permission,
) -> PermissionDecision {
    verification_command_permission_with_approval(executable, args, configured, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn ask_and_deny_are_not_silently_upgraded() {
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
    fn explicit_approval_satisfies_ask_once() {
        let decision = verification_command_permission_with_approval(
            "cargo",
            &args(&["test"]),
            Permission::Ask,
            true,
        );
        assert_eq!(decision.permission, Permission::Allow);
        assert_eq!(decision.reason, PermissionReason::ExplicitApproval);
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
    fn privilege_escalation_is_hard_denied_even_after_approval() {
        for executable in ["sudo", "doas", "pkexec", "runas.exe"] {
            let decision = verification_command_permission_with_approval(
                executable,
                &args(&["anything"]),
                Permission::Allow,
                true,
            );
            assert_eq!(decision.permission, Permission::Deny);
            assert_eq!(
                decision.reason,
                PermissionReason::HardDeniedPrivilegeEscalation
            );
        }
    }

    #[test]
    fn opaque_shell_requires_approval_but_can_be_approved_once() {
        let shell_args = args(&["-Command", "cargo test"]);
        let first =
            verification_command_permission("powershell.exe", &shell_args, Permission::Allow);
        assert_eq!(first.permission, Permission::Ask);
        assert_eq!(first.reason, PermissionReason::OpaqueShellCommand);

        let approved = verification_command_permission_with_approval(
            "powershell.exe",
            &shell_args,
            Permission::Allow,
            true,
        );
        assert_eq!(approved.permission, Permission::Allow);
    }

    #[test]
    fn mutating_git_commands_are_hard_denied_from_verification() {
        for subcommand in ["push", "commit", "reset", "clean", "rebase", "worktree"] {
            let decision = verification_command_permission_with_approval(
                "git.exe",
                &args(&[subcommand]),
                Permission::Allow,
                true,
            );
            assert_eq!(decision.permission, Permission::Deny, "{subcommand}");
            assert_eq!(
                decision.reason,
                PermissionReason::HardDeniedVerificationMutation
            );
        }
    }

    #[test]
    fn read_only_git_commands_can_run_when_allowed() {
        for subcommand in ["status", "diff", "rev-parse", "show"] {
            let decision =
                verification_command_permission("git", &args(&[subcommand]), Permission::Allow);
            assert_eq!(decision.permission, Permission::Allow, "{subcommand}");
        }
    }

    #[test]
    fn explicit_approval_does_not_override_configured_deny() {
        let decision = verification_command_permission_with_approval(
            "cargo",
            &args(&["test"]),
            Permission::Deny,
            true,
        );
        assert_eq!(decision.permission, Permission::Deny);
    }
}

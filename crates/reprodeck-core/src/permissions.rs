use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Permission {
    Allow,
    #[default]
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionReason {
    Configured,
    ExplicitApproval,
    HardDeniedPrivilegeEscalation,
    HardDeniedDestructiveCommand,
    HardDeniedOriginalGitMutation,
    OpaqueShellCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDecision {
    pub permission: Permission,
    pub reason: PermissionReason,
    pub explanation: String,
}

fn executable_name(executable: &str) -> String {
    Path::new(executable)
        .file_stem()
        .or_else(|| Path::new(executable).file_name())
        .and_then(|value| value.to_str())
        .unwrap_or(executable)
        .to_ascii_lowercase()
}

fn first_positional(args: &[String]) -> Option<String> {
    args.iter()
        .find(|arg| !arg.trim().is_empty() && !arg.starts_with('-'))
        .map(|arg| arg.to_ascii_lowercase())
}

fn hard_deny(executable: &str, args: &[String], original_repo: bool) -> Option<PermissionDecision> {
    let executable = executable_name(executable);
    if matches!(executable.as_str(), "sudo" | "doas" | "pkexec" | "runas") {
        return Some(PermissionDecision {
            permission: Permission::Deny,
            reason: PermissionReason::HardDeniedPrivilegeEscalation,
            explanation: "Privilege escalation is never executed by ReproDeck.".into(),
        });
    }

    if matches!(
        executable.as_str(),
        "format" | "diskpart" | "mkfs" | "fdisk" | "parted"
    ) {
        return Some(PermissionDecision {
            permission: Permission::Deny,
            reason: PermissionReason::HardDeniedDestructiveCommand,
            explanation: "Disk formatting or partitioning commands are blocked.".into(),
        });
    }

    if original_repo && executable == "git" {
        let sub = first_positional(args).unwrap_or_default();
        if matches!(
            sub.as_str(),
            "reset"
                | "clean"
                | "checkout"
                | "switch"
                | "merge"
                | "rebase"
                | "cherry-pick"
                | "push"
                | "commit"
        ) {
            return Some(PermissionDecision {
                permission: Permission::Deny,
                reason: PermissionReason::HardDeniedOriginalGitMutation,
                explanation:
                    "Mutating Git commands are not allowed against the original repository.".into(),
            });
        }
    }
    None
}

pub fn command_permission(
    executable: &str,
    args: &[String],
    configured: Permission,
    explicitly_approved_once: bool,
    original_repo: bool,
) -> PermissionDecision {
    if let Some(decision) = hard_deny(executable, args, original_repo) {
        return decision;
    }

    let shell = matches!(
        executable_name(executable).as_str(),
        "sh" | "bash" | "zsh" | "fish" | "cmd" | "powershell" | "pwsh"
    );
    if shell && !explicitly_approved_once {
        return PermissionDecision {
            permission: Permission::Ask,
            reason: PermissionReason::OpaqueShellCommand,
            explanation: "Shell-wrapped commands require explicit one-shot approval.".into(),
        };
    }

    match configured {
        Permission::Deny => PermissionDecision {
            permission: Permission::Deny,
            reason: PermissionReason::Configured,
            explanation: "The command policy denies execution.".into(),
        },
        Permission::Ask if explicitly_approved_once => PermissionDecision {
            permission: Permission::Allow,
            reason: PermissionReason::ExplicitApproval,
            explanation: "Approved once by the user.".into(),
        },
        Permission::Ask => PermissionDecision {
            permission: Permission::Ask,
            reason: PermissionReason::Configured,
            explanation: "User approval is required before execution.".into(),
        },
        Permission::Allow => PermissionDecision {
            permission: Permission::Allow,
            reason: PermissionReason::Configured,
            explanation: "The command policy allows execution.".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).into()).collect()
    }

    #[test]
    fn privilege_escalation_is_always_denied() {
        for exe in ["sudo", "doas", "pkexec", "runas.exe"] {
            assert_eq!(
                command_permission(exe, &[], Permission::Allow, true, false).permission,
                Permission::Deny
            );
        }
    }

    #[test]
    fn original_repo_mutation_is_denied() {
        assert_eq!(
            command_permission(
                "git",
                &args(&["reset", "--hard"]),
                Permission::Allow,
                true,
                true
            )
            .permission,
            Permission::Deny
        );
        assert_eq!(
            command_permission("git", &args(&["status"]), Permission::Allow, false, true)
                .permission,
            Permission::Allow
        );
    }

    #[test]
    fn shell_wrapper_requires_explicit_approval() {
        let a = args(&["-Command", "npm test"]);
        assert_eq!(
            command_permission("pwsh", &a, Permission::Allow, false, false).permission,
            Permission::Ask
        );
        assert_eq!(
            command_permission("pwsh", &a, Permission::Allow, true, false).permission,
            Permission::Allow
        );
    }
}

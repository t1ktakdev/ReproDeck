use serde::{Deserialize, Serialize};

/// Permission level for potentially unsafe actions (commands, file access).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Permission {
    /// Always allow without prompting
    Allow,
    /// Ask the user (requires UI), treated as Deny in non-interactive contexts
    #[default]
    Ask,
    /// Always deny
    Deny,
}

use reprodeck_core::shadow_session::{self, ShadowChange, ShadowSessionError, ShadowWorkspaceRecord};
use serde::{Deserialize, Serialize};

use super::BridgeError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ShadowWorkspaceDto {
    pub session_id: String,
    pub repo_id: String,
    pub repo_path: String,
    pub base_commit: String,
    pub branch: String,
    pub worktree_path: String,
    pub original_branch: String,
    pub changes: Vec<ShadowChange>,
}

fn map_shadow_error(error: ShadowSessionError) -> BridgeError {
    match error {
        ShadowSessionError::SessionNotFound(_) => {
            BridgeError::new("not_found", "Session not found.")
        }
        ShadowSessionError::RepositoryNotAttached(_) => BridgeError::new(
            "repository_required",
            "Attach a Git repository before creating an isolated workspace.",
        ),
        ShadowSessionError::ShadowNotFound(_) => {
            BridgeError::new("shadow_not_found", "This session has no isolated workspace.")
        }
        ShadowSessionError::StaleShadow => BridgeError::new(
            "shadow_stale",
            "The isolated workspace can no longer be resumed safely.",
        ),
        ShadowSessionError::NoChanges => BridgeError::new(
            "no_changes",
            "The isolated workspace has no changes to checkpoint.",
        ),
        ShadowSessionError::AppliedStateCleanupFailed => BridgeError::new(
            "applied_cleanup_pending",
            "Changes were applied, but ReproDeck could not clear the local workspace record.",
        ),
        ShadowSessionError::DiscardedStateCleanupFailed => BridgeError::new(
            "discarded_cleanup_pending",
            "The isolated workspace was discarded, but ReproDeck could not clear its local record.",
        ),
        _ => BridgeError::new(
            "shadow_workspace_error",
            "ReproDeck could not complete the isolated workspace operation.",
        ),
    }
}

fn dto(
    conn: &rusqlite::Connection,
    record: ShadowWorkspaceRecord,
) -> Result<ShadowWorkspaceDto, BridgeError> {
    let changes = shadow_session::list_session_shadow_changes(conn, &record.session_id)
        .map_err(map_shadow_error)?;
    Ok(ShadowWorkspaceDto {
        session_id: record.session_id,
        repo_id: record.repo_id,
        repo_path: record.repo_path,
        base_commit: record.base_commit,
        branch: record.branch,
        worktree_path: record.worktree_path,
        original_branch: record.original_branch,
        changes,
    })
}

#[tauri::command]
pub(crate) fn get_shadow_workspace(
    session_id: String,
) -> Result<Option<ShadowWorkspaceDto>, BridgeError> {
    let conn = super::open_conn()?;
    let record = shadow_session::get_session_shadow(&conn, &session_id).map_err(map_shadow_error)?;
    record.map(|record| dto(&conn, record)).transpose()
}

#[tauri::command]
pub(crate) fn create_shadow_workspace(
    session_id: String,
) -> Result<ShadowWorkspaceDto, BridgeError> {
    let conn = super::open_conn()?;
    let record =
        shadow_session::create_session_shadow(&conn, &session_id).map_err(map_shadow_error)?;
    dto(&conn, record)
}

#[tauri::command]
pub(crate) fn refresh_shadow_workspace(
    session_id: String,
) -> Result<ShadowWorkspaceDto, BridgeError> {
    let conn = super::open_conn()?;
    let record = shadow_session::get_session_shadow(&conn, &session_id)
        .map_err(map_shadow_error)?
        .ok_or_else(|| BridgeError::new("shadow_not_found", "This session has no isolated workspace."))?;
    dto(&conn, record)
}

#[tauri::command]
pub(crate) fn finalize_shadow_workspace(
    session_id: String,
) -> Result<ShadowWorkspaceDto, BridgeError> {
    let conn = super::open_conn()?;
    shadow_session::finalize_session_shadow(&conn, &session_id).map_err(map_shadow_error)?;
    let record = shadow_session::get_session_shadow(&conn, &session_id)
        .map_err(map_shadow_error)?
        .ok_or_else(|| BridgeError::new("shadow_not_found", "This session has no isolated workspace."))?;
    dto(&conn, record)
}

#[tauri::command]
pub(crate) fn apply_shadow_workspace(session_id: String) -> Result<(), BridgeError> {
    let conn = super::open_conn()?;
    shadow_session::apply_session_shadow(&conn, &session_id).map_err(map_shadow_error)
}

#[tauri::command]
pub(crate) fn discard_shadow_workspace(session_id: String) -> Result<(), BridgeError> {
    let conn = super::open_conn()?;
    shadow_session::discard_session_shadow(&conn, &session_id).map_err(map_shadow_error)
}

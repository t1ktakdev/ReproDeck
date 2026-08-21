use crate::timeline;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Draft,
    Preparing,
    CapturingEnvironment,
    CreatingWorkspace,
    Ready,
    Reproducing,
    FailureCaptured,
    Fixing,
    Verifying,
    Verified,
    ReadyToApply,
    Applying,
    Applied,
    Discarded,
    Failed,
    Cancelled,
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::str::FromStr for SessionState {
    type Err = StateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        use SessionState::*;
        match value {
            "Draft" => Ok(Draft),
            "Preparing" => Ok(Preparing),
            "CapturingEnvironment" => Ok(CapturingEnvironment),
            "CreatingWorkspace" => Ok(CreatingWorkspace),
            "Ready" | "Active" => Ok(Ready),
            "Reproducing" => Ok(Reproducing),
            "FailureCaptured" => Ok(FailureCaptured),
            "Fixing" => Ok(Fixing),
            "Verifying" => Ok(Verifying),
            "Verified" => Ok(Verified),
            "ReadyToApply" => Ok(ReadyToApply),
            "Applying" => Ok(Applying),
            "Applied" => Ok(Applied),
            "Discarded" => Ok(Discarded),
            "Failed" => Ok(Failed),
            "Cancelled" => Ok(Cancelled),
            other => Err(StateError::Unknown(other.to_owned())),
        }
    }
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("unknown session state: {0}")]
    Unknown(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("invalid session transition: {from} -> {to}")]
    InvalidTransition {
        from: SessionState,
        to: SessionState,
    },
    #[error(transparent)]
    Timeline(#[from] timeline::TimelineError),
}

pub fn can_transition(from: SessionState, to: SessionState) -> bool {
    use SessionState::*;
    if from == to {
        return true;
    }
    if matches!(to, Failed | Cancelled | Discarded) && !matches!(from, Applied | Discarded) {
        return true;
    }
    matches!(
        (from, to),
        (Draft, Preparing)
            | (Preparing, CapturingEnvironment)
            | (Preparing, CreatingWorkspace)
            | (CapturingEnvironment, CreatingWorkspace)
            | (CreatingWorkspace, Ready)
            | (Ready, Reproducing)
            | (Ready, Fixing)
            | (Ready, Discarded)
            | (Reproducing, Ready)
            | (Reproducing, FailureCaptured)
            | (Reproducing, Fixing)
            | (FailureCaptured, Reproducing)
            | (FailureCaptured, Fixing)
            | (FailureCaptured, Verifying)
            | (FailureCaptured, Discarded)
            | (Fixing, Reproducing)
            | (Fixing, Verifying)
            | (Fixing, Discarded)
            | (Verifying, Verified)
            | (Verifying, Fixing)
            | (Verified, Reproducing)
            | (Verified, Verifying)
            | (Verified, Fixing)
            | (Verified, ReadyToApply)
            | (Verified, Discarded)
            | (ReadyToApply, Reproducing)
            | (ReadyToApply, Verifying)
            | (ReadyToApply, Fixing)
            | (ReadyToApply, Applying)
            | (ReadyToApply, Discarded)
            | (Applying, Applied)
            | (Failed, Preparing)
            | (Cancelled, Preparing)
    )
}

pub fn require_transition(from: SessionState, to: SessionState) -> Result<(), StateError> {
    if can_transition(from, to) {
        Ok(())
    } else {
        Err(StateError::InvalidTransition { from, to })
    }
}

pub fn transition_session(
    conn: &Connection,
    session_id: &str,
    to: SessionState,
) -> Result<SessionState, StateError> {
    let session = timeline::get_session_record(conn, session_id)?
        .ok_or_else(|| StateError::SessionNotFound(session_id.to_owned()))?;
    let from: SessionState = session.state.parse()?;
    require_transition(from, to)?;
    if from != to {
        timeline::update_session_state(conn, session_id, &to.to_string())?;
    }
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use tempfile::NamedTempFile;

    #[test]
    fn happy_path_is_accepted() {
        let path = [
            SessionState::Draft,
            SessionState::Preparing,
            SessionState::CreatingWorkspace,
            SessionState::Ready,
            SessionState::Reproducing,
            SessionState::FailureCaptured,
            SessionState::Fixing,
            SessionState::Verifying,
            SessionState::Verified,
            SessionState::ReadyToApply,
            SessionState::Applying,
            SessionState::Applied,
        ];
        for pair in path.windows(2) {
            assert!(can_transition(pair[0], pair[1]), "{pair:?}");
        }
    }

    #[test]
    fn cannot_skip_to_applied() {
        assert!(!can_transition(SessionState::Draft, SessionState::Applied));
    }

    #[test]
    fn rerunning_before_invalidates_ready_to_apply_path() {
        assert!(can_transition(
            SessionState::Verified,
            SessionState::Reproducing
        ));
        assert!(can_transition(
            SessionState::ReadyToApply,
            SessionState::Reproducing
        ));
        assert!(can_transition(
            SessionState::Reproducing,
            SessionState::FailureCaptured
        ));
    }

    #[test]
    fn persisted_transition_is_validated() {
        let db = NamedTempFile::new().unwrap();
        let conn = init_db(db.path()).unwrap();
        timeline::create_session(&conn, "session", "Draft", None).unwrap();
        transition_session(&conn, "session", SessionState::Preparing).unwrap();
        assert_eq!(
            timeline::get_session_record(&conn, "session")
                .unwrap()
                .unwrap()
                .state,
            "Preparing"
        );
        assert!(transition_session(&conn, "session", SessionState::Applied).is_err());
    }
}

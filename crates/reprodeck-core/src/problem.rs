use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProblemStatus {
    Signal,
    Suspected,
    Reproduced,
    RootCaused,
    FixProposed,
    Verified,
    Applied,
    Dismissed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootCauseClaim {
    pub cause: String,
    pub supporting_evidence_ids: Vec<String>,
    pub contradicting_evidence_ids: Vec<String>,
    pub confidence_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProblemTransition {
    pub from: ProblemStatus,
    pub to: ProblemStatus,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProblemTransitionError {
    #[error("invalid problem transition: {from:?} -> {to:?}")]
    Invalid {
        from: ProblemStatus,
        to: ProblemStatus,
    },
    #[error("transition to {0:?} requires evidence")]
    EvidenceRequired(ProblemStatus),
}

fn transition_allowed(from: ProblemStatus, to: ProblemStatus) -> bool {
    use ProblemStatus::*;
    matches!(
        (from, to),
        (Signal, Suspected)
            | (Signal, Reproduced)
            | (Signal, Dismissed)
            | (Suspected, Reproduced)
            | (Suspected, Dismissed)
            | (Reproduced, RootCaused)
            | (Reproduced, FixProposed)
            | (Reproduced, Dismissed)
            | (RootCaused, FixProposed)
            | (RootCaused, Dismissed)
            | (FixProposed, Verified)
            | (FixProposed, RootCaused)
            | (Verified, Applied)
            | (Verified, FixProposed)
            | (Dismissed, Signal)
    )
}

pub fn validate_transition(transition: &ProblemTransition) -> Result<(), ProblemTransitionError> {
    if !transition_allowed(transition.from, transition.to) {
        return Err(ProblemTransitionError::Invalid {
            from: transition.from,
            to: transition.to,
        });
    }
    if matches!(
        transition.to,
        ProblemStatus::Reproduced | ProblemStatus::RootCaused | ProblemStatus::Verified
    ) && transition.evidence_ids.is_empty()
    {
        return Err(ProblemTransitionError::EvidenceRequired(transition.to));
    }
    Ok(())
}

impl RootCauseClaim {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        if self.cause.trim().is_empty() {
            return Err("root cause text is empty");
        }
        if self.supporting_evidence_ids.is_empty() {
            return Err("root cause requires supporting evidence");
        }
        if self.confidence_percent > 100 {
            return Err("confidence must be between 0 and 100");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cannot_jump_from_signal_to_verified() {
        let result = validate_transition(&ProblemTransition {
            from: ProblemStatus::Signal,
            to: ProblemStatus::Verified,
            evidence_ids: vec!["e1".into()],
        });
        assert!(matches!(
            result,
            Err(ProblemTransitionError::Invalid { .. })
        ));
    }

    #[test]
    fn reproduced_and_verified_require_evidence() {
        for (from, to) in [
            (ProblemStatus::Signal, ProblemStatus::Reproduced),
            (ProblemStatus::FixProposed, ProblemStatus::Verified),
        ] {
            assert_eq!(
                validate_transition(&ProblemTransition {
                    from,
                    to,
                    evidence_ids: Vec::new()
                }),
                Err(ProblemTransitionError::EvidenceRequired(to))
            );
        }
    }

    #[test]
    fn evidence_backed_path_is_valid() {
        for (from, to) in [
            (ProblemStatus::Signal, ProblemStatus::Reproduced),
            (ProblemStatus::Reproduced, ProblemStatus::RootCaused),
            (ProblemStatus::RootCaused, ProblemStatus::FixProposed),
            (ProblemStatus::FixProposed, ProblemStatus::Verified),
            (ProblemStatus::Verified, ProblemStatus::Applied),
        ] {
            validate_transition(&ProblemTransition {
                from,
                to,
                evidence_ids: vec!["e1".into()],
            })
            .unwrap();
        }
    }
}

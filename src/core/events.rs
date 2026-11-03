use crate::core::types::ApproachChoice;
use serde::{Deserialize, Serialize};

/// One typed in-memory session event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub seq: u64,
    pub at_ms: u64,
    pub kind: EventKind,
}

/// Kind of session event; payloads never carry secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    SessionCreated,
    WaitingForCandidateStarted,
    CandidateConnected,
    ExpertQuestionsSet,
    ExtraQuestionAdded,
    QuestionStarted {
        index: usize,
    },
    ApproachSelected {
        index: usize,
        choice: ApproachChoice,
    },
    TokensLocked {
        index: usize,
        expected: u64,
    },
    Continued {
        index: usize,
    },
    Redirected {
        index: usize,
        instruction: String,
    },
    ApproachChanged {
        index: usize,
    },
    TestsRequested {
        index: usize,
    },
    ReQuestionRequested {
        index: usize,
    },
    ReQuestionResolved {
        index: usize,
        approved: bool,
    },
    QuestionFinished {
        index: usize,
    },
    QuestionSkipped {
        index: usize,
    },
    /// Candidate switched the active question; a finished or skipped
    /// target reopens, and every slot keeps its recorded approach.
    QuestionReopened {
        index: usize,
    },
    TestRunCompleted {
        index: usize,
        passed: u32,
        failed: u32,
    },
    PasteFlagged {
        index: usize,
        inserted_bytes: u64,
    },
    ChatBoundaryViolated {
        index: Option<usize>,
    },
    CandidateDisconnected,
    CandidateReconnected {
        downtime_ms: u64,
    },
    ExpertEnded,
    TechnicalFailure,
    Completed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_round_trips_through_json() {
        let event = SessionEvent {
            seq: 2,
            at_ms: 42,
            kind: EventKind::TokensLocked {
                index: 1,
                expected: 500,
            },
        };
        let json = serde_json::to_string(&event).expect("event serializes");
        let back: SessionEvent = serde_json::from_str(&json).expect("event parses");
        assert_eq!(back, event);
    }

    #[test]
    fn evidence_events_round_trip_through_json() {
        let kinds = [
            EventKind::ApproachSelected {
                index: 3,
                choice: ApproachChoice::Other("sliding window".to_string()),
            },
            EventKind::ApproachSelected {
                index: 0,
                choice: ApproachChoice::Preset(2),
            },
            EventKind::Redirected {
                index: 3,
                instruction: "use a heap".to_string(),
            },
        ];
        for kind in kinds {
            let event = SessionEvent {
                seq: 1,
                at_ms: 7,
                kind,
            };
            let json = serde_json::to_string(&event).expect("event serializes");
            let back: SessionEvent = serde_json::from_str(&json).expect("event parses");
            assert_eq!(back, event);
        }
    }
}

//! Date evidence recovery ladder (Task 1.4).
//!
//! Fixed order: Wait → ROI zoom OCR → alt preprocess hook → full grab → rebind → exhaust.
//! Exhaustion yields DATE_EVIDENCE_TIMEOUT, never DATE_MISMATCH.

/// Soft wait between recovery observes.
pub const DATE_RECOVERY_WAIT_MS: u64 = 350;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateRecoveryStage {
    WaitStabilize,
    RoiZoomOcr,
    AltPreprocess,
    FullGrab,
    Rebind,
    Exhausted,
}

impl DateRecoveryStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WaitStabilize => "wait_stabilize",
            Self::RoiZoomOcr => "roi_zoom_ocr",
            Self::AltPreprocess => "alt_preprocess",
            Self::FullGrab => "full_grab",
            Self::Rebind => "rebind",
            Self::Exhausted => "exhausted",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::WaitStabilize => Self::RoiZoomOcr,
            Self::RoiZoomOcr => Self::AltPreprocess,
            Self::AltPreprocess => Self::FullGrab,
            Self::FullGrab => Self::Rebind,
            Self::Rebind | Self::Exhausted => Self::Exhausted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateRecoveryBudget {
    pub max_attempts: u32,
    pub deadline_monotonic_ms: u64,
    pub attempts: u32,
    pub stage: DateRecoveryStage,
}

impl DateRecoveryBudget {
    pub fn new(max_attempts: u32, deadline_monotonic_ms: u64) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            deadline_monotonic_ms,
            attempts: 0,
            stage: DateRecoveryStage::WaitStabilize,
        }
    }

    /// Default budget for live runs (~8 attempts / 12s).
    pub fn for_live_run(now_ms: u64) -> Self {
        Self::new(8, now_ms.saturating_add(12_000))
    }

    pub fn remaining_ok(&self, now_ms: u64) -> bool {
        self.attempts < self.max_attempts && now_ms < self.deadline_monotonic_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateRecoveryAction {
    /// Host should Wait then re-observe.
    Wait {
        stage: DateRecoveryStage,
        approx_ms: u64,
        reason: &'static str,
    },
    /// Hint: rebind window lease (host maps to Rebind intent).
    Rebind { reason: &'static str },
    /// Budget gone — terminal but not DATE_MISMATCH.
    Timeout,
}

/// Advance recovery one step while evidence remains Insufficient.
pub fn next_date_recovery(budget: &mut DateRecoveryBudget, now_ms: u64) -> DateRecoveryAction {
    if !budget.remaining_ok(now_ms) {
        budget.stage = DateRecoveryStage::Exhausted;
        return DateRecoveryAction::Timeout;
    }
    budget.attempts = budget.attempts.saturating_add(1);
    let stage = budget.stage;
    let action = match stage {
        DateRecoveryStage::WaitStabilize => DateRecoveryAction::Wait {
            stage,
            approx_ms: DATE_RECOVERY_WAIT_MS,
            reason: "DATE_EVIDENCE_WEAK",
        },
        DateRecoveryStage::RoiZoomOcr => DateRecoveryAction::Wait {
            stage,
            approx_ms: DATE_RECOVERY_WAIT_MS,
            reason: "DATE_EVIDENCE_WEAK",
        },
        DateRecoveryStage::AltPreprocess => DateRecoveryAction::Wait {
            stage,
            approx_ms: DATE_RECOVERY_WAIT_MS,
            reason: "DATE_EVIDENCE_WEAK",
        },
        DateRecoveryStage::FullGrab => DateRecoveryAction::Wait {
            stage,
            approx_ms: DATE_RECOVERY_WAIT_MS,
            reason: "DATE_EVIDENCE_WEAK",
        },
        DateRecoveryStage::Rebind => DateRecoveryAction::Rebind {
            reason: "DATE_EVIDENCE_WEAK",
        },
        DateRecoveryStage::Exhausted => DateRecoveryAction::Timeout,
    };
    budget.stage = stage.next();
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_order_then_timeout() {
        let mut b = DateRecoveryBudget::new(5, 10_000);
        let a0 = next_date_recovery(&mut b, 0);
        assert!(matches!(
            a0,
            DateRecoveryAction::Wait {
                stage: DateRecoveryStage::WaitStabilize,
                ..
            }
        ));
        let a1 = next_date_recovery(&mut b, 100);
        assert!(matches!(
            a1,
            DateRecoveryAction::Wait {
                stage: DateRecoveryStage::RoiZoomOcr,
                ..
            }
        ));
        let _ = next_date_recovery(&mut b, 200); // alt
        let _ = next_date_recovery(&mut b, 300); // full
        let a4 = next_date_recovery(&mut b, 400);
        assert!(matches!(a4, DateRecoveryAction::Rebind { .. }));
        let a5 = next_date_recovery(&mut b, 500);
        assert_eq!(a5, DateRecoveryAction::Timeout);
        assert_eq!(b.stage, DateRecoveryStage::Exhausted);
    }

    #[test]
    fn deadline_forces_timeout_without_mismatch() {
        let mut b = DateRecoveryBudget::new(20, 100);
        let a = next_date_recovery(&mut b, 200);
        assert_eq!(a, DateRecoveryAction::Timeout);
    }
}

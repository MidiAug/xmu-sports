//! Session-internal pending execution: opaque tokens for the host.
//!
//! The host must never rebuild Navigate / AtomicComposite / captcha / submit
//! actions from GlanceReport fields. It only holds `execution_token` and calls
//! `execute_pending(token)` on the session.

use vision_contracts::{ActionPlan, ActionType, Idempotency, ReasonCode};

use crate::intent::{AtomicStep, CaptchaKeyboardPlan};
use crate::state::Page;

/// Opaque token returned to the host after observe.
pub type ExecutionToken = String;

#[derive(Debug, Clone, PartialEq)]
pub enum PendingKind {
    Navigate {
        label: String,
        expected_page: Page,
    },
    AtomicComposite {
        label: String,
        steps: Vec<AtomicStep>,
        expected_page: Page,
    },
    /// Fragile shell nav that required a streak before arming.
    FragileNavigate {
        label: String,
        expected_page: Page,
        streak: u32,
    },
    CaptchaWorkflow {
        label: String,
        keyboard: CaptchaKeyboardPlan,
    },
    SubmitOnce {
        label: String,
        expected_page: Page,
    },
}

/// Full pending execution kept inside the session (not serialized to the host).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingExecution {
    pub token: ExecutionToken,
    pub kind: PendingKind,
    /// Primary ActionPlan (first click / submit). Source frame is authoritative.
    pub plan: ActionPlan,
    pub consumed: bool,
}

impl PendingExecution {
    pub fn action_plan_id(&self) -> &str {
        &self.plan.action_plan_id
    }

    pub fn source_frame_id(&self) -> &str {
        self.plan.source_frame_id()
    }

    pub fn is_expired(&self, now_monotonic_ms: u64) -> bool {
        self.plan.is_expired(now_monotonic_ms)
    }
}

/// In-session store: at most one pending execution at a time for P0.
#[derive(Debug, Default)]
pub struct PendingStore {
    current: Option<PendingExecution>,
    token_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingError {
    NotFound,
    Expired,
    AlreadyConsumed,
    RunMismatch,
}

impl PendingError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::NotFound => "PENDING_NOT_FOUND",
            Self::Expired => ReasonCode::PLAN_EXPIRED,
            Self::AlreadyConsumed => ReasonCode::PLAN_ALREADY_CONSUMED,
            Self::RunMismatch => ReasonCode::RUN_ID_MISMATCH,
        }
    }
}

impl PendingStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.current = None;
    }

    pub fn peek(&self) -> Option<&PendingExecution> {
        self.current.as_ref()
    }

    /// Install a new pending execution; replaces any previous unconsumed one.
    pub fn install(
        &mut self,
        run_id: &str,
        kind: PendingKind,
        plan: ActionPlan,
    ) -> Result<ExecutionToken, PendingError> {
        if plan.run_id != run_id {
            return Err(PendingError::RunMismatch);
        }
        self.token_seq = self.token_seq.saturating_add(1);
        let token = format!("tok-{}-{}", run_id, self.token_seq);
        self.current = Some(PendingExecution {
            token: token.clone(),
            kind,
            plan,
            consumed: false,
        });
        Ok(token)
    }

    /// Take pending for execution: marks consumed immediately (one-shot).
    /// Failed inject callers must not reuse the token.
    pub fn take_for_execute(
        &mut self,
        token: &str,
        run_id: &str,
        now_monotonic_ms: u64,
    ) -> Result<PendingExecution, PendingError> {
        let cur = self.current.as_mut().ok_or(PendingError::NotFound)?;
        if cur.token != token {
            return Err(PendingError::NotFound);
        }
        if cur.plan.run_id != run_id {
            return Err(PendingError::RunMismatch);
        }
        if cur.consumed {
            return Err(PendingError::AlreadyConsumed);
        }
        if cur.is_expired(now_monotonic_ms) {
            cur.consumed = true;
            return Err(PendingError::Expired);
        }
        cur.consumed = true;
        Ok(cur.clone())
    }

    /// Helper: build Navigate pending from an already-built click plan.
    pub fn install_navigate(
        &mut self,
        run_id: &str,
        label: &str,
        expected_page: Page,
        plan: ActionPlan,
    ) -> Result<ExecutionToken, PendingError> {
        self.install(
            run_id,
            PendingKind::Navigate {
                label: label.to_string(),
                expected_page,
            },
            plan,
        )
    }
}

/// Map ActionType / intent into PendingKind for simple navigate/submit cases.
pub fn pending_kind_from_click(
    label: &str,
    expected_page: Page,
    action_type: ActionType,
    fragile_streak: Option<u32>,
) -> PendingKind {
    if matches!(action_type, ActionType::SubmitOnce) {
        return PendingKind::SubmitOnce {
            label: label.to_string(),
            expected_page,
        };
    }
    if let Some(streak) = fragile_streak {
        return PendingKind::FragileNavigate {
            label: label.to_string(),
            expected_page,
            streak,
        };
    }
    PendingKind::Navigate {
        label: label.to_string(),
        expected_page,
    }
}

/// Convenience: submit-once plans must use NonIdempotentSubmit.
pub fn assert_submit_idempotency(plan: &ActionPlan) -> bool {
    matches!(plan.action_type, ActionType::SubmitOnce)
        == matches!(plan.idempotency, Idempotency::NonIdempotentSubmit)
}

/// Re-export AnchorBox for callers assembling pending from intent.
pub use vision_contracts::AnchorBox as PendingAnchor;

#[cfg(test)]
mod tests {
    use super::*;
    use vision_contracts::{AnchorBox, SizeI};

    fn sample_plan(run_id: &str, created: u64, ttl: u64) -> ActionPlan {
        ActionPlan {
            action_plan_id: "ap-test".into(),
            run_id: run_id.into(),
            lease_id: "L1".into(),
            generation: 1,
            frame_id: "f-src".into(),
            page_body_fingerprint: "fp".into(),
            source_capture_pixel_size: SizeI {
                width: 100,
                height: 200,
            },
            anchor_box: Some(AnchorBox {
                x0: 1,
                y0: 1,
                x1: 10,
                y1: 10,
            }),
            action_type: ActionType::Click,
            confidence: 0.95,
            expected_transition: "next".into(),
            created_monotonic_ms: created,
            ttl_ms: ttl,
            idempotency: Idempotency::Idempotent,
            text: None,
        }
    }

    #[test]
    fn token_one_shot_rejects_replay() {
        let mut store = PendingStore::new();
        let plan = sample_plan("run-a", 100, 500);
        let tok = store
            .install_navigate("run-a", "预定", Page::SelectSlot, plan)
            .unwrap();
        let first = store.take_for_execute(&tok, "run-a", 200).unwrap();
        assert_eq!(first.source_frame_id(), "f-src");
        let err = store.take_for_execute(&tok, "run-a", 250).unwrap_err();
        assert_eq!(err, PendingError::AlreadyConsumed);
    }

    #[test]
    fn expired_token_rejected() {
        let mut store = PendingStore::new();
        let plan = sample_plan("run-a", 100, 50);
        let tok = store
            .install_navigate("run-a", "预定", Page::SelectSlot, plan)
            .unwrap();
        let err = store.take_for_execute(&tok, "run-a", 200).unwrap_err();
        assert_eq!(err, PendingError::Expired);
    }

    #[test]
    fn cross_run_rejected() {
        let mut store = PendingStore::new();
        let plan = sample_plan("run-a", 100, 500);
        let tok = store
            .install_navigate("run-a", "预定", Page::SelectSlot, plan)
            .unwrap();
        let err = store.take_for_execute(&tok, "run-b", 150).unwrap_err();
        assert_eq!(err, PendingError::RunMismatch);
    }

    #[test]
    fn unknown_token_rejected() {
        let mut store = PendingStore::new();
        let plan = sample_plan("run-a", 100, 500);
        let _ = store
            .install_navigate("run-a", "预定", Page::SelectSlot, plan)
            .unwrap();
        let err = store
            .take_for_execute("tok-missing", "run-a", 150)
            .unwrap_err();
        assert_eq!(err, PendingError::NotFound);
    }

    #[test]
    fn install_rejects_plan_run_mismatch() {
        let mut store = PendingStore::new();
        let plan = sample_plan("run-a", 100, 500);
        let err = store
            .install_navigate("run-other", "预定", Page::SelectSlot, plan)
            .unwrap_err();
        assert_eq!(err, PendingError::RunMismatch);
    }

    #[test]
    fn atomic_and_submit_kinds_cover_complex_actions() {
        let mut store = PendingStore::new();
        let mut plan = sample_plan("run-a", 100, 500);
        plan.action_type = ActionType::SubmitOnce;
        plan.idempotency = Idempotency::NonIdempotentSubmit;
        assert!(assert_submit_idempotency(&plan));
        let tok = store
            .install(
                "run-a",
                PendingKind::SubmitOnce {
                    label: "确认预约".into(),
                    expected_page: Page::ConfirmOrder,
                },
                plan,
            )
            .unwrap();
        let pending = store.take_for_execute(&tok, "run-a", 120).unwrap();
        assert!(matches!(pending.kind, PendingKind::SubmitOnce { .. }));

        let mut store2 = PendingStore::new();
        let plan2 = sample_plan("run-a", 100, 500);
        let tok2 = store2
            .install(
                "run-a",
                PendingKind::AtomicComposite {
                    label: "搜索厦大体育".into(),
                    steps: vec![
                        AtomicStep::Click {
                            label: "magnifier".into(),
                            anchor: AnchorBox {
                                x0: 1,
                                y0: 1,
                                x1: 5,
                                y1: 5,
                            },
                        },
                        AtomicStep::TypeText {
                            text: "厦大体育".into(),
                        },
                    ],
                    expected_page: Page::WechatMpSearch,
                },
                plan2,
            )
            .unwrap();
        let p2 = store2.take_for_execute(&tok2, "run-a", 110).unwrap();
        assert!(matches!(p2.kind, PendingKind::AtomicComposite { .. }));
    }
}

use thiserror::Error;
use vision_contracts::{
    ActionPlan, AnchorBox, GuardedAction, Idempotency, ReasonCode, SizeI, TargetSurfaceLease,
    ValidateOutcome,
};

use crate::state::UiSnapshot;

#[derive(Debug, Error)]
pub enum GuardError {
    #[error("{0}")]
    Blocked(String),
}

/// Inputs for cheap pre-inject verification.
///
/// Anchor semantics always come from `plan` (source frame). Screen transform uses
/// the verification frame. When no new WGC frame is available, set
/// `verification_is_new_capture = false` and only allow within `same_frame_max_age_ms`.
pub struct GuardInput<'a> {
    pub plan: &'a ActionPlan,
    pub lease: &'a TargetSurfaceLease,
    pub validate: &'a ValidateOutcome,
    /// Must equal `plan.run_id`.
    pub session_run_id: &'a str,
    pub fresh_snap: &'a UiSnapshot,
    pub verification_frame_id: &'a str,
    pub verification_generation: u64,
    pub verification_capture_size: SizeI,
    /// Observed anchor on the verification frame (same label / ROI). None skips
    /// geometric drift (e.g. TypeText with no box) but still checks size/lease.
    pub verification_anchor: Option<&'a AnchorBox>,
    pub now_monotonic_ms: u64,
    /// True when `verification_frame_id` is a freshly grabbed WGC frame.
    pub verification_is_new_capture: bool,
    /// When verification reuses the source frame, max age since plan creation.
    pub same_frame_max_age_ms: u64,
    pub input_enabled: bool,
    pub min_confidence: f32,
    pub max_anchor_drift_px: i32,
    /// Submit-once replay: true if this plan/token was already consumed.
    pub already_consumed: bool,
}

/// Observe → Classify → Candidate → Guard. Never produces GuardedAction if unsafe.
pub fn guard_action(input: GuardInput<'_>) -> Result<GuardedAction, GuardError> {
    if !input.input_enabled {
        return Err(GuardError::Blocked(ReasonCode::INPUT_DISABLED.into()));
    }
    if input.already_consumed {
        return Err(GuardError::Blocked(ReasonCode::PLAN_ALREADY_CONSUMED.into()));
    }
    if input.plan.confidence < input.min_confidence {
        return Err(GuardError::Blocked(ReasonCode::LOW_CONFIDENCE.into()));
    }
    if input.session_run_id != input.plan.run_id {
        return Err(GuardError::Blocked(ReasonCode::RUN_ID_MISMATCH.into()));
    }
    if input.lease.lease_id != input.plan.lease_id {
        return Err(GuardError::Blocked(ReasonCode::LEASE_ID_MISMATCH.into()));
    }
    if input.plan.is_expired(input.now_monotonic_ms) {
        return Err(GuardError::Blocked(ReasonCode::PLAN_EXPIRED.into()));
    }
    match input.validate {
        ValidateOutcome::Valid => {}
        ValidateOutcome::RebindRequired { reason } => {
            return Err(GuardError::Blocked(format!(
                "{}:{reason}",
                ReasonCode::LEASE_REBIND_REQUIRED
            )));
        }
        ValidateOutcome::Invalid { reason } => {
            return Err(GuardError::Blocked(format!(
                "{}:{reason}",
                ReasonCode::LEASE_INVALID
            )));
        }
    }
    if input.verification_generation != input.plan.generation
        || input.lease.generation != input.plan.generation
    {
        return Err(GuardError::Blocked(
            ReasonCode::LEASE_REBIND_REQUIRED.into(),
        ));
    }

    let src = &input.plan.source_capture_pixel_size;
    let ver = &input.verification_capture_size;
    if src.width > 0
        && src.height > 0
        && (src.width != ver.width || src.height != ver.height)
    {
        return Err(GuardError::Blocked(ReasonCode::FRAME_SIZE_MISMATCH.into()));
    }

    if !input.verification_is_new_capture {
        if input.verification_frame_id != input.plan.source_frame_id() {
            return Err(GuardError::Blocked(ReasonCode::NO_VERIFICATION_FRAME.into()));
        }
        let age = input
            .now_monotonic_ms
            .saturating_sub(input.plan.created_monotonic_ms);
        if age > input.same_frame_max_age_ms {
            return Err(GuardError::Blocked(ReasonCode::STALE_FRAME.into()));
        }
    }

    if input.fresh_snap.body_fingerprint != input.plan.page_body_fingerprint {
        return Err(GuardError::Blocked(ReasonCode::FINGERPRINT_MISMATCH.into()));
    }

    if let Some(box_) = &input.plan.anchor_box {
        if box_.x1 <= box_.x0 || box_.y1 <= box_.y0 {
            return Err(GuardError::Blocked(ReasonCode::ANCHOR_MOVED.into()));
        }
        if let Some(observed) = input.verification_anchor {
            if observed.x1 <= observed.x0 || observed.y1 <= observed.y0 {
                return Err(GuardError::Blocked(ReasonCode::ANCHOR_MOVED.into()));
            }
            let drift = box_.center_drift_px(observed);
            if drift > input.max_anchor_drift_px {
                return Err(GuardError::Blocked(ReasonCode::ANCHOR_MOVED.into()));
            }
        }
    }

    if matches!(input.plan.idempotency, Idempotency::NonIdempotentSubmit) {
        // Replay is blocked via `already_consumed` above; verify owns post-submit.
    }

    Ok(GuardedAction {
        source_frame_id: input.plan.source_frame_id().to_string(),
        verification_frame_id: input.verification_frame_id.to_string(),
        verified_generation: input.verification_generation,
        verified_fingerprint: input.fresh_snap.body_fingerprint.clone(),
        plan: input.plan.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vision_contracts::{ActionType, OwnerIdentity, PlatformKind, RectF};

    fn lease() -> TargetSurfaceLease {
        TargetSurfaceLease {
            lease_id: "L1".into(),
            generation: 1,
            platform: PlatformKind::Fake,
            owner: OwnerIdentity {
                pid: 1,
                app_identity: "fake".into(),
                launch_identity: "boot".into(),
            },
            surface_fingerprint: "fp".into(),
            title_hint: "t".into(),
            bounds_in_screen_points: RectF {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            capture_pixel_size: SizeI {
                width: 100,
                height: 200,
            },
            scale_x: 1.0,
            scale_y: 1.0,
            issued_at: "t0".into(),
            expires_at: "t1".into(),
        }
    }

    fn plan(fp: &str) -> ActionPlan {
        ActionPlan {
            action_plan_id: "ap1".into(),
            run_id: "r1".into(),
            lease_id: "L1".into(),
            generation: 1,
            frame_id: "f0".into(),
            page_body_fingerprint: fp.into(),
            source_capture_pixel_size: SizeI {
                width: 100,
                height: 200,
            },
            anchor_box: Some(AnchorBox {
                x0: 10,
                y0: 10,
                x1: 30,
                y1: 30,
            }),
            action_type: ActionType::Click,
            confidence: 0.9,
            expected_transition: "next".into(),
            created_monotonic_ms: 1_000,
            ttl_ms: 500,
            idempotency: Idempotency::Idempotent,
            text: None,
        }
    }

    fn ok_input<'a>(
        plan: &'a ActionPlan,
        lease: &'a TargetSurfaceLease,
        snap: &'a UiSnapshot,
        anchor: &'a AnchorBox,
    ) -> GuardInput<'a> {
        GuardInput {
            plan,
            lease,
            validate: &ValidateOutcome::Valid,
            session_run_id: "r1",
            fresh_snap: snap,
            verification_frame_id: "f1",
            verification_generation: 1,
            verification_capture_size: SizeI {
                width: 100,
                height: 200,
            },
            verification_anchor: Some(anchor),
            now_monotonic_ms: 1_100,
            verification_is_new_capture: true,
            same_frame_max_age_ms: 100,
            input_enabled: true,
            min_confidence: 0.5,
            max_anchor_drift_px: 8,
            already_consumed: false,
        }
    }

    #[test]
    fn blocks_when_input_disabled() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let mut input = ok_input(&plan, &lease, &snap, &anchor);
        input.input_enabled = false;
        let err = guard_action(input).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::INPUT_DISABLED));
    }

    #[test]
    fn blocks_fingerprint_mismatch() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpB".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let err = guard_action(ok_input(&plan, &lease, &snap, &anchor)).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::FINGERPRINT_MISMATCH));
    }

    #[test]
    fn blocks_invalid_lease() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let mut input = ok_input(&plan, &lease, &snap, &anchor);
        let invalid = ValidateOutcome::Invalid {
            reason: "gone".into(),
        };
        input.validate = &invalid;
        let err = guard_action(input).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::LEASE_INVALID));
    }

    #[test]
    fn blocks_expired_plan() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let mut input = ok_input(&plan, &lease, &snap, &anchor);
        input.now_monotonic_ms = 2_000;
        let err = guard_action(input).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::PLAN_EXPIRED));
    }

    #[test]
    fn blocks_run_id_mismatch() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let mut input = ok_input(&plan, &lease, &snap, &anchor);
        input.session_run_id = "other-run";
        let err = guard_action(input).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::RUN_ID_MISMATCH));
    }

    #[test]
    fn blocks_lease_id_mismatch() {
        let mut lease = lease();
        lease.lease_id = "L-other".into();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let err = guard_action(ok_input(&plan, &lease, &snap, &anchor)).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::LEASE_ID_MISMATCH));
    }

    #[test]
    fn blocks_frame_size_mismatch_resize_or_dpi() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let mut input = ok_input(&plan, &lease, &snap, &anchor);
        input.verification_capture_size = SizeI {
            width: 125,
            height: 250,
        };
        let err = guard_action(input).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::FRAME_SIZE_MISMATCH));
    }

    #[test]
    fn blocks_anchor_drift() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let drifted = AnchorBox {
            x0: 40,
            y0: 40,
            x1: 60,
            y1: 60,
        };
        let err = guard_action(ok_input(&plan, &lease, &snap, &drifted)).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::ANCHOR_MOVED));
    }

    #[test]
    fn allows_same_frame_within_short_ttl() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let mut input = ok_input(&plan, &lease, &snap, &anchor);
        input.verification_frame_id = "f0";
        input.verification_is_new_capture = false;
        input.same_frame_max_age_ms = 200;
        input.now_monotonic_ms = 1_150;
        let g = guard_action(input).unwrap();
        assert_eq!(g.source_frame_id, "f0");
        assert_eq!(g.verification_frame_id, "f0");
    }

    #[test]
    fn blocks_same_frame_after_short_ttl() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let mut input = ok_input(&plan, &lease, &snap, &anchor);
        input.verification_frame_id = "f0";
        input.verification_is_new_capture = false;
        input.same_frame_max_age_ms = 50;
        input.now_monotonic_ms = 1_200;
        let err = guard_action(input).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::STALE_FRAME));
    }

    #[test]
    fn blocks_no_new_frame_with_foreign_id() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let mut input = ok_input(&plan, &lease, &snap, &anchor);
        input.verification_is_new_capture = false;
        input.verification_frame_id = "f-other";
        let err = guard_action(input).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::NO_VERIFICATION_FRAME));
    }

    #[test]
    fn blocks_submit_once_replay() {
        let lease = lease();
        let mut plan = plan("fpA");
        plan.action_type = ActionType::SubmitOnce;
        plan.idempotency = Idempotency::NonIdempotentSubmit;
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let mut input = ok_input(&plan, &lease, &snap, &anchor);
        input.already_consumed = true;
        let err = guard_action(input).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::PLAN_ALREADY_CONSUMED));
    }

    #[test]
    fn allows_matching_records_source_and_verification() {
        let lease = lease();
        let plan = plan("fpA");
        let snap = UiSnapshot {
            body_fingerprint: "fpA".into(),
            ..UiSnapshot::default()
        };
        let anchor = plan.anchor_box.clone().unwrap();
        let g = guard_action(ok_input(&plan, &lease, &snap, &anchor)).unwrap();
        assert_eq!(g.source_frame_id, "f0");
        assert_eq!(g.verification_frame_id, "f1");
        assert_eq!(g.verified_frame_id(), "f1");
    }
}

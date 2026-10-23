use vision_contracts::{ActionPlan, ActionType, AnchorBox, Idempotency, SizeI};

use crate::state::UiSnapshot;

/// Default plan TTL when caller does not override (ms, process-monotonic).
/// Must exceed typical observe→inject gap; fragile-nav confirm rebuilds the plan
/// after streak (see inject.rs) so confirm budget is not charged against this.
pub const DEFAULT_PLAN_TTL_MS: u64 = 2_500;

pub fn build_click_plan(
    run_id: &str,
    lease_id: &str,
    generation: u64,
    frame_id: &str,
    source_capture_pixel_size: SizeI,
    snap: &UiSnapshot,
    anchor: AnchorBox,
    action_type: ActionType,
    expected_transition: &str,
    confidence: f32,
    created_monotonic_ms: u64,
    ttl_ms: u64,
) -> ActionPlan {
    let idempotency = if matches!(action_type, ActionType::SubmitOnce) {
        Idempotency::NonIdempotentSubmit
    } else {
        Idempotency::Idempotent
    };
    ActionPlan {
        action_plan_id: format!("ap-{}-{}", frame_id, expected_transition),
        run_id: run_id.to_string(),
        lease_id: lease_id.to_string(),
        generation,
        frame_id: frame_id.to_string(),
        page_body_fingerprint: snap.body_fingerprint.clone(),
        source_capture_pixel_size,
        anchor_box: Some(anchor),
        action_type,
        confidence,
        expected_transition: expected_transition.to_string(),
        created_monotonic_ms,
        ttl_ms,
        idempotency,
        text: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vision_contracts::monotonic_now_ms;

    #[test]
    fn plan_expiry_uses_monotonic_ttl() {
        let snap = UiSnapshot::default();
        let created = monotonic_now_ms();
        let plan = build_click_plan(
            "run-1",
            "lease-1",
            1,
            "frame-src",
            SizeI {
                width: 400,
                height: 800,
            },
            &snap,
            AnchorBox {
                x0: 10,
                y0: 10,
                x1: 40,
                y1: 40,
            },
            ActionType::Click,
            "next",
            0.9,
            created,
            50,
        );
        assert_eq!(plan.source_frame_id(), "frame-src");
        assert!(!plan.is_expired(created));
        assert!(!plan.is_expired(created + 50));
        assert!(plan.is_expired(created + 51));
    }

    #[test]
    fn submit_once_is_non_idempotent() {
        let snap = UiSnapshot::default();
        let plan = build_click_plan(
            "run-1",
            "lease-1",
            1,
            "f",
            SizeI {
                width: 1,
                height: 1,
            },
            &snap,
            AnchorBox {
                x0: 0,
                y0: 0,
                x1: 2,
                y1: 2,
            },
            ActionType::SubmitOnce,
            "submit",
            1.0,
            0,
            800,
        );
        assert!(matches!(
            plan.idempotency,
            Idempotency::NonIdempotentSubmit
        ));
    }
}

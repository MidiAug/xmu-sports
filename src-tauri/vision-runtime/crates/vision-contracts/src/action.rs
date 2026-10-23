use serde::{Deserialize, Serialize};

use crate::lease::SizeI;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Click,
    TypeText,
    Scroll,
    Wait,
    Rebind,
    Stop,
    SubmitOnce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Idempotency {
    Idempotent,
    NonIdempotentSubmit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorBox {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl AnchorBox {
    pub fn center(&self) -> (i32, i32) {
        ((self.x0 + self.x1) / 2, (self.y0 + self.y1) / 2)
    }

    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    /// Chebyshev distance between centers (pixels).
    pub fn center_drift_px(&self, other: &AnchorBox) -> i32 {
        let (ax, ay) = self.center();
        let (bx, by) = other.center();
        (ax - bx).abs().max((ay - by).abs())
    }
}

/// Proposed action from the recognizer — never executed directly.
///
/// `frame_id` is the **source** frame that produced this plan (serde also accepts
/// `source_frame_id`). Expiry is process-monotonic: `created_monotonic_ms + ttl_ms`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionPlan {
    pub action_plan_id: String,
    pub run_id: String,
    pub lease_id: String,
    pub generation: u64,
    /// Source frame that produced this plan. Alias: `source_frame_id`.
    #[serde(alias = "source_frame_id")]
    pub frame_id: String,
    pub page_body_fingerprint: String,
    /// Capture pixel size of the source frame (resize/DPI guard).
    #[serde(default = "default_capture_size")]
    pub source_capture_pixel_size: SizeI,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_box: Option<AnchorBox>,
    pub action_type: ActionType,
    pub confidence: f32,
    pub expected_transition: String,
    /// Process-local monotonic ms when the plan was created.
    #[serde(default)]
    pub created_monotonic_ms: u64,
    /// Time-to-live after `created_monotonic_ms`. Replaces unverifiable `deadline_ms`.
    #[serde(default = "default_plan_ttl_ms", alias = "deadline_ms")]
    pub ttl_ms: u64,
    pub idempotency: Idempotency,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

fn default_capture_size() -> SizeI {
    SizeI {
        width: 0,
        height: 0,
    }
}

fn default_plan_ttl_ms() -> u64 {
    2_500
}

impl ActionPlan {
    pub fn source_frame_id(&self) -> &str {
        &self.frame_id
    }

    pub fn expires_at_monotonic_ms(&self) -> u64 {
        self.created_monotonic_ms.saturating_add(self.ttl_ms)
    }

    pub fn is_expired(&self, now_monotonic_ms: u64) -> bool {
        now_monotonic_ms > self.expires_at_monotonic_ms()
    }
}

/// Action that passed Guard — the only type inject_input accepts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardedAction {
    pub plan: ActionPlan,
    /// Same as `plan.frame_id` (source). Explicit for traces.
    #[serde(default, alias = "plan_frame_id")]
    pub source_frame_id: String,
    /// Frame used for cheap verification immediately before inject.
    #[serde(alias = "verified_frame_id")]
    pub verification_frame_id: String,
    pub verified_generation: u64,
    pub verified_fingerprint: String,
}

impl GuardedAction {
    /// Backward-compat accessor used by older call sites / tests.
    pub fn verified_frame_id(&self) -> &str {
        &self.verification_frame_id
    }
}

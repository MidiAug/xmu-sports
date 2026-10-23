use serde::{Deserialize, Serialize};
use std::fmt;

/// Machine-readable fault / guard reasons (non-exhaustive string form for IPC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReasonCode(pub String);

impl ReasonCode {
    pub const SCREEN_RECORDING_DENIED: &'static str = "SCREEN_RECORDING_DENIED";
    pub const ACCESSIBILITY_DENIED: &'static str = "ACCESSIBILITY_DENIED";
    pub const TARGET_MINIMIZED: &'static str = "TARGET_MINIMIZED";
    pub const FOREGROUND_LOST: &'static str = "FOREGROUND_LOST";
    pub const CAPTURE_DEVICE_LOST: &'static str = "CAPTURE_DEVICE_LOST";
    pub const LEASE_INVALID: &'static str = "LEASE_INVALID";
    pub const LEASE_REBIND_REQUIRED: &'static str = "LEASE_REBIND_REQUIRED";
    pub const LEASE_EXPIRED: &'static str = "LEASE_EXPIRED";
    pub const PACK_NOT_INSTALLED: &'static str = "PACK_NOT_INSTALLED";
    pub const PACK_HEALTH_FAILED: &'static str = "PACK_HEALTH_FAILED";
    pub const LOW_CONFIDENCE: &'static str = "LOW_CONFIDENCE";
    pub const STALE_FRAME: &'static str = "STALE_FRAME";
    pub const PLAN_EXPIRED: &'static str = "PLAN_EXPIRED";
    pub const RUN_ID_MISMATCH: &'static str = "RUN_ID_MISMATCH";
    pub const LEASE_ID_MISMATCH: &'static str = "LEASE_ID_MISMATCH";
    pub const FRAME_SIZE_MISMATCH: &'static str = "FRAME_SIZE_MISMATCH";
    pub const NO_VERIFICATION_FRAME: &'static str = "NO_VERIFICATION_FRAME";
    pub const PLAN_ALREADY_CONSUMED: &'static str = "PLAN_ALREADY_CONSUMED";
    pub const FINGERPRINT_MISMATCH: &'static str = "FINGERPRINT_MISMATCH";
    pub const ANCHOR_MOVED: &'static str = "ANCHOR_MOVED";
    pub const USER_BUSY: &'static str = "USER_BUSY";
    pub const INPUT_DISABLED: &'static str = "INPUT_DISABLED";
    pub const UNKNOWN_AFTER_SUBMIT: &'static str = "UNKNOWN_AFTER_SUBMIT";
    pub const PROTOCOL_MISMATCH: &'static str = "PROTOCOL_MISMATCH";
    pub const WORKER_EXITED: &'static str = "WORKER_EXITED";
    pub const MACOS_PATH_UNVERIFIED: &'static str = "MACOS_PATH_UNVERIFIED";
    pub const WGC_UNAVAILABLE: &'static str = "WGC_UNAVAILABLE";
    pub const CONTENT_UNUSABLE: &'static str = "CONTENT_UNUSABLE";
    pub const CAPTURE_BACKEND_MISMATCH: &'static str = "CAPTURE_BACKEND_MISMATCH";
    pub const FULLSCREEN_FALLBACK_FORBIDDEN: &'static str = "FULLSCREEN_FALLBACK_FORBIDDEN";
    /// Date strip OCR incomplete / multi-frame consensus not reached (retryable Wait).
    pub const DATE_EVIDENCE_WEAK: &'static str = "DATE_EVIDENCE_WEAK";
    /// Recovery budget exhausted without ConfirmedAbsent (terminal, not DATE_MISMATCH).
    pub const DATE_EVIDENCE_TIMEOUT: &'static str = "DATE_EVIDENCE_TIMEOUT";
    pub const DATE_MISMATCH: &'static str = "DATE_MISMATCH";
    pub const DATE_UNAVAILABLE: &'static str = "DATE_UNAVAILABLE";

    pub fn new(code: impl Into<String>) -> Self {
        Self(code.into())
    }
}

impl fmt::Display for ReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

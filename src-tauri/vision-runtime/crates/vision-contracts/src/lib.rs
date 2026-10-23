//! Cross-platform vision contracts (no HWND / CGWindowID / Tauri).

pub mod action;
pub mod capability;
pub mod frame;
pub mod ipc;
pub mod lease;
pub mod reason;
pub mod time;

pub use action::{ActionPlan, ActionType, AnchorBox, GuardedAction, Idempotency};
pub use capability::{CapabilityReport, CapabilityStatus};
pub use frame::{
    CaptureBackend, Frame, FramePixels, FrameStatus, PixelFormat, RectI, ScreenTransform,
};
pub use ipc::{IpcEnvelope, IpcKind, PROTOCOL_VERSION};
pub use lease::{
    OwnerIdentity, PlatformKind, RectF, SizeI, SurfaceCandidate, TargetSurfaceLease,
    ValidateOutcome,
};
pub use reason::ReasonCode;
pub use time::{
    is_fresher, lease_expired, lease_issue_and_expiry, monotonic_now_ms, parse_rfc3339,
    wall_now_rfc3339, DEFAULT_LEASE_TTL_SECS,
};

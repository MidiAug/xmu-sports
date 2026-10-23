use thiserror::Error;
use vision_contracts::{
    CapabilityReport, Frame, GuardedAction, SurfaceCandidate, TargetSurfaceLease, ValidateOutcome,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationStatus {
    Foreground,
    CanActivate,
    BusyUser,
    Denied,
}

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Error)]
pub enum InputError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone)]
pub struct InputReceipt {
    pub accepted: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct PlatformDiagnostics {
    pub redacted_lines: Vec<String>,
}

/// Capability contract for adapters. Core never sees HWND/CGWindowID.
pub trait PlatformPort: Send {
    fn check_capabilities(&mut self) -> Result<CapabilityReport, PlatformError>;
    fn discover_surfaces(
        &mut self,
        app_selector: &str,
    ) -> Result<Vec<SurfaceCandidate>, PlatformError>;
    fn bind_surface(&mut self, candidate_id: &str) -> Result<TargetSurfaceLease, PlatformError>;
    fn validate_surface(
        &mut self,
        lease: &TargetSurfaceLease,
    ) -> Result<ValidateOutcome, PlatformError>;
    fn capture_frame(
        &mut self,
        lease: &TargetSurfaceLease,
        deadline_ms: u64,
    ) -> Result<Frame, CaptureError>;
    fn activation_status(
        &mut self,
        lease: &TargetSurfaceLease,
    ) -> Result<ActivationStatus, PlatformError>;
    fn activate_surface(
        &mut self,
        lease: &TargetSurfaceLease,
        explicit_authority: bool,
    ) -> Result<(), PlatformError>;
    fn inject_input(
        &mut self,
        action: &GuardedAction,
        lease: &TargetSurfaceLease,
        fresh_frame: &Frame,
    ) -> Result<InputReceipt, InputError>;
    fn collect_diagnostics(&mut self) -> Result<PlatformDiagnostics, PlatformError>;
    fn release_surface(&mut self, lease: &TargetSurfaceLease) -> Result<(), PlatformError>;
}

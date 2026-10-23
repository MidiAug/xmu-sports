//! macOS PlatformPort — returns unsupported / path-unverified until Mac 真机验收.
//! Compiles on all hosts so the workspace builds on Windows CI.

use vision_contracts::{
    CapabilityReport, CapabilityStatus, Frame, GuardedAction, PlatformKind, ReasonCode,
    SurfaceCandidate, TargetSurfaceLease, ValidateOutcome,
};
use vision_platform_api::{
    ActivationStatus, CaptureError, InputError, InputReceipt, PlatformDiagnostics, PlatformError,
    PlatformPort,
};

pub struct MacosPlatform {
    pub pack_ready: bool,
}

impl MacosPlatform {
    pub fn new() -> Self {
        Self { pack_ready: false }
    }
}

impl Default for MacosPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformPort for MacosPlatform {
    fn check_capabilities(&mut self) -> Result<CapabilityReport, PlatformError> {
        Ok(CapabilityReport {
            status: CapabilityStatus::Unsupported,
            reason_code: ReasonCode::MACOS_PATH_UNVERIFIED.into(),
            platform: PlatformKind::Macos,
            pack_ready: self.pack_ready,
            input_enabled: false,
            details: Some(serde_json::json!({
                "note": "macOS WeChat miniprogram path not verified on real hardware yet",
                "report": "docs/research/macos-vision-path-feasibility.md"
            })),
        })
    }

    fn discover_surfaces(
        &mut self,
        _app_selector: &str,
    ) -> Result<Vec<SurfaceCandidate>, PlatformError> {
        Err(PlatformError::Message(
            ReasonCode::MACOS_PATH_UNVERIFIED.into(),
        ))
    }

    fn bind_surface(&mut self, _candidate_id: &str) -> Result<TargetSurfaceLease, PlatformError> {
        Err(PlatformError::Message(
            ReasonCode::MACOS_PATH_UNVERIFIED.into(),
        ))
    }

    fn validate_surface(
        &mut self,
        _lease: &TargetSurfaceLease,
    ) -> Result<ValidateOutcome, PlatformError> {
        Ok(ValidateOutcome::Invalid {
            reason: ReasonCode::MACOS_PATH_UNVERIFIED.into(),
        })
    }

    fn capture_frame(
        &mut self,
        _lease: &TargetSurfaceLease,
        _deadline_ms: u64,
    ) -> Result<Frame, CaptureError> {
        Err(CaptureError::Message(
            ReasonCode::MACOS_PATH_UNVERIFIED.into(),
        ))
    }

    fn activation_status(
        &mut self,
        _lease: &TargetSurfaceLease,
    ) -> Result<ActivationStatus, PlatformError> {
        Ok(ActivationStatus::Denied)
    }

    fn activate_surface(
        &mut self,
        _lease: &TargetSurfaceLease,
        _explicit_authority: bool,
    ) -> Result<(), PlatformError> {
        Err(PlatformError::Message(
            ReasonCode::MACOS_PATH_UNVERIFIED.into(),
        ))
    }

    fn inject_input(
        &mut self,
        _action: &GuardedAction,
        _lease: &TargetSurfaceLease,
        _fresh_frame: &Frame,
    ) -> Result<InputReceipt, InputError> {
        Err(InputError::Message(ReasonCode::INPUT_DISABLED.into()))
    }

    fn collect_diagnostics(&mut self) -> Result<PlatformDiagnostics, PlatformError> {
        Ok(PlatformDiagnostics {
            redacted_lines: vec!["macos: path unverified".into()],
        })
    }

    fn release_surface(&mut self, _lease: &TargetSurfaceLease) -> Result<(), PlatformError> {
        Ok(())
    }
}

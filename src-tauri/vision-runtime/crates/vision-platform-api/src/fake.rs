use crate::port::{
    ActivationStatus, CaptureError, InputError, InputReceipt, PlatformDiagnostics, PlatformError,
    PlatformPort,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use vision_contracts::{
    CapabilityReport, CapabilityStatus, CaptureBackend, Frame, FramePixels, FrameStatus,
    GuardedAction, OwnerIdentity, PixelFormat, PlatformKind, ReasonCode, RectF, RectI,
    ScreenTransform, SizeI, SurfaceCandidate, TargetSurfaceLease, ValidateOutcome,
};
#[derive(Debug, Clone)]
pub enum FakeScript {
    /// Next validate returns Invalid.
    InvalidateLease,
    /// Next validate returns RebindRequired.
    RebindRequired,
    /// Next capture returns black frame.
    BlackFrame,
    /// Next activation is BusyUser.
    BusyUser,
    /// Drop the bound surface entirely.
    CloseSurface,
}
#[derive(Debug, Default)]
pub struct FakeInputSink {
    pub actions: Vec<GuardedAction>,
}
impl FakeInputSink {
    pub fn clear(&mut self) {
        self.actions.clear();
    }
}
pub struct FakePlatform {
    pub input_enabled: bool,
    pub pack_ready: bool,
    candidates: Vec<SurfaceCandidate>,
    lease: Option<TargetSurfaceLease>,
    generation: u64,
    frames: VecDeque<Frame>,
    script: VecDeque<FakeScript>,
    pub sink: Arc<Mutex<FakeInputSink>>,
    frame_seq: u64,
}
impl FakePlatform {
    pub fn new() -> Self {
        let cand = SurfaceCandidate {
            candidate_id: "cand-1".into(),
            title_hint: "厦大体育".into(),
            owner: OwnerIdentity {
                pid: 4242,
                app_identity: "fake.wechat".into(),
                launch_identity: "boot-1".into(),
            },
            bounds_in_screen_points: RectF {
                x: 100.0,
                y: 80.0,
                width: 420.0,
                height: 760.0,
            },
            surface_fingerprint: "fake:1".into(),
        };
        Self {
            input_enabled: false,
            pack_ready: true,
            candidates: vec![cand],
            lease: None,
            generation: 0,
            frames: VecDeque::new(),
            script: VecDeque::new(),
            sink: Arc::new(Mutex::new(FakeInputSink::default())),
            frame_seq: 0,
        }
    }
    pub fn push_script(&mut self, s: FakeScript) {
        self.script.push_back(s);
    }
    pub fn enqueue_frame_bgra(&mut self, width: u32, height: u32, fill: u8) {
        let lease = self.lease.clone().unwrap_or_else(|| self.demo_lease());
        self.frame_seq += 1;
        let bytes = vec![fill; (width * height * 4) as usize];
        self.frames.push_back(Frame {
            frame_id: format!("f-{}", self.frame_seq),
            lease_id: lease.lease_id.clone(),
            generation: lease.generation,
            monotonic_capture_time_ms: vision_contracts::monotonic_now_ms(),
            pixel_width: width,
            pixel_height: height,
            pixel_format: PixelFormat::Bgra8,
            content_rect: RectI {
                x: 0,
                y: 0,
                width,
                height,
            },
            screen_transform: ScreenTransform {
                origin_x: lease.bounds_in_screen_points.x,
                origin_y: lease.bounds_in_screen_points.y,
                scale_x: lease.scale_x,
                scale_y: lease.scale_y,
            },
            status: FrameStatus::Ok,
            pixels: Some(FramePixels {
                bytes,
                stride: (width * 4) as usize,
            }),
            capture_backend: CaptureBackend::Fake,
        });
    }
    fn demo_lease(&self) -> TargetSurfaceLease {
        let c = &self.candidates[0];
        TargetSurfaceLease {
            lease_id: "lease-fake-1".into(),
            generation: self.generation,
            platform: PlatformKind::Fake,
            owner: c.owner.clone(),
            surface_fingerprint: c.surface_fingerprint.clone(),
            title_hint: c.title_hint.clone(),
            bounds_in_screen_points: c.bounds_in_screen_points.clone(),
            capture_pixel_size: SizeI {
                width: 420,
                height: 760,
            },
            scale_x: 1.0,
            scale_y: 1.0,
            issued_at: String::new(),
            expires_at: String::new(),
        }
        .with_default_ttl()
    }
}
impl Default for FakePlatform {
    fn default() -> Self {
        Self::new()
    }
}
impl PlatformPort for FakePlatform {
    fn check_capabilities(&mut self) -> Result<CapabilityReport, PlatformError> {
        let status = if !self.pack_ready {
            CapabilityStatus::NotInstalled
        } else if !self.input_enabled {
            CapabilityStatus::Ready
        } else {
            CapabilityStatus::Ready
        };
        let reason = if !self.pack_ready {
            ReasonCode::PACK_NOT_INSTALLED
        } else {
            "OK"
        };
        Ok(CapabilityReport {
            status,
            reason_code: reason.into(),
            platform: PlatformKind::Fake,
            pack_ready: self.pack_ready,
            input_enabled: self.input_enabled,
            details: None,
        })
    }
    fn discover_surfaces(
        &mut self,
        _app_selector: &str,
    ) -> Result<Vec<SurfaceCandidate>, PlatformError> {
        Ok(self.candidates.clone())
    }
    fn bind_surface(&mut self, candidate_id: &str) -> Result<TargetSurfaceLease, PlatformError> {
        if !self
            .candidates
            .iter()
            .any(|c| c.candidate_id == candidate_id)
        {
            return Err(PlatformError::Message(format!(
                "unknown candidate {candidate_id}"
            )));
        }
        self.generation += 1;
        let mut lease = self.demo_lease();
        lease.generation = self.generation;
        lease.lease_id = format!("lease-fake-{}", self.generation);
        self.lease = Some(lease.clone());
        Ok(lease)
    }
    fn validate_surface(
        &mut self,
        lease: &TargetSurfaceLease,
    ) -> Result<ValidateOutcome, PlatformError> {
        if let Some(script) = self.script.pop_front() {
            match script {
                FakeScript::InvalidateLease | FakeScript::CloseSurface => {
                    self.lease = None;
                    return Ok(ValidateOutcome::Invalid {
                        reason: "scripted_close".into(),
                    });
                }
                FakeScript::RebindRequired => {
                    return Ok(ValidateOutcome::RebindRequired {
                        reason: "scripted_resize".into(),
                    });
                }
                FakeScript::BusyUser => {}
                FakeScript::BlackFrame => {
                    self.script.push_front(FakeScript::BlackFrame);
                }
            }
        }
        match &self.lease {
            Some(current) if current.lease_id == lease.lease_id => {
                if current.generation != lease.generation {
                    Ok(ValidateOutcome::RebindRequired {
                        reason: "generation_mismatch".into(),
                    })
                } else {
                    Ok(ValidateOutcome::Valid)
                }
            }
            _ => Ok(ValidateOutcome::Invalid {
                reason: "no_bound_lease".into(),
            }),
        }
    }
    fn capture_frame(
        &mut self,
        lease: &TargetSurfaceLease,
        _deadline_ms: u64,
    ) -> Result<Frame, CaptureError> {
        match self.validate_surface(lease) {
            Ok(ValidateOutcome::Valid) => {}
            Ok(ValidateOutcome::RebindRequired { reason }) => {
                return Err(CaptureError::Message(format!(
                    "{}:{reason}",
                    ReasonCode::LEASE_REBIND_REQUIRED
                )));
            }
            Ok(ValidateOutcome::Invalid { reason }) => {
                return Err(CaptureError::Message(format!(
                    "{}:{reason}",
                    ReasonCode::LEASE_INVALID
                )));
            }
            Err(e) => return Err(CaptureError::Message(e.to_string())),
        }
        if let Some(FakeScript::BlackFrame) = self.script.front() {
            self.script.pop_front();
            return Ok(Frame {
                frame_id: "black".into(),
                lease_id: lease.lease_id.clone(),
                generation: lease.generation,
                monotonic_capture_time_ms: vision_contracts::monotonic_now_ms(),
                pixel_width: 10,
                pixel_height: 10,
                pixel_format: PixelFormat::Bgra8,
                content_rect: RectI {
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 10,
                },
                screen_transform: ScreenTransform {
                    origin_x: 0.0,
                    origin_y: 0.0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                },
                status: FrameStatus::Black,
                pixels: Some(FramePixels {
                    bytes: vec![0; 400],
                    stride: 40,
                }),
                capture_backend: CaptureBackend::Fake,
            });
        }
        if let Some(mut frame) = self.frames.pop_front() {
            frame.lease_id = lease.lease_id.clone();
            frame.generation = lease.generation;
            return Ok(frame);
        }
        // Default solid frame
        self.enqueue_frame_bgra(420, 760, 40);
        self.capture_frame(lease, _deadline_ms)
    }
    fn activation_status(
        &mut self,
        _lease: &TargetSurfaceLease,
    ) -> Result<ActivationStatus, PlatformError> {
        if let Some(FakeScript::BusyUser) = self.script.front() {
            self.script.pop_front();
            return Ok(ActivationStatus::BusyUser);
        }
        Ok(ActivationStatus::Foreground)
    }
    fn activate_surface(
        &mut self,
        _lease: &TargetSurfaceLease,
        explicit_authority: bool,
    ) -> Result<(), PlatformError> {
        if !explicit_authority {
            return Err(PlatformError::Message("authority required".into()));
        }
        Ok(())
    }
    fn inject_input(
        &mut self,
        action: &GuardedAction,
        lease: &TargetSurfaceLease,
        fresh_frame: &Frame,
    ) -> Result<InputReceipt, InputError> {
        if !self.input_enabled {
            return Err(InputError::Message(ReasonCode::INPUT_DISABLED.into()));
        }
        if fresh_frame.lease_id != lease.lease_id || fresh_frame.generation != lease.generation {
            return Err(InputError::Message(ReasonCode::LEASE_INVALID.into()));
        }
        if action.verified_generation != lease.generation {
            return Err(InputError::Message(
                ReasonCode::LEASE_REBIND_REQUIRED.into(),
            ));
        }
        self.sink
            .lock()
            .map_err(|e| InputError::Message(e.to_string()))?
            .actions
            .push(action.clone());
        Ok(InputReceipt {
            accepted: true,
            detail: "fake_recorded".into(),
        })
    }
    fn collect_diagnostics(&mut self) -> Result<PlatformDiagnostics, PlatformError> {
        Ok(PlatformDiagnostics {
            redacted_lines: vec![format!(
                "fake lease={:?} frames_queued={}",
                self.lease.as_ref().map(|l| &l.lease_id),
                self.frames.len()
            )],
        })
    }
    fn release_surface(&mut self, _lease: &TargetSurfaceLease) -> Result<(), PlatformError> {
        self.lease = None;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use vision_contracts::{ActionPlan, ActionType, Idempotency};
    #[test]
    fn inject_blocked_when_disabled() {
        let mut p = FakePlatform::new();
        let lease = p.bind_surface("cand-1").unwrap();
        let frame = p.capture_frame(&lease, 100).unwrap();
        let plan = ActionPlan {
            action_plan_id: "a".into(),
            run_id: "r".into(),
            lease_id: lease.lease_id.clone(),
            generation: lease.generation,
            frame_id: frame.frame_id.clone(),
            page_body_fingerprint: "fp".into(),
            source_capture_pixel_size: vision_contracts::SizeI {
                width: frame.content_rect.width,
                height: frame.content_rect.height,
            },
            anchor_box: None,
            action_type: ActionType::Click,
            confidence: 1.0,
            expected_transition: "x".into(),
            created_monotonic_ms: 0,
            ttl_ms: 1,
            idempotency: Idempotency::Idempotent,
            text: None,
        };
        let guarded = GuardedAction {
            source_frame_id: frame.frame_id.clone(),
            verification_frame_id: frame.frame_id.clone(),
            verified_generation: lease.generation,
            verified_fingerprint: "fp".into(),
            plan,
        };
        let err = p.inject_input(&guarded, &lease, &frame).unwrap_err();
        assert!(err.to_string().contains(ReasonCode::INPUT_DISABLED));
        assert!(p.sink.lock().unwrap().actions.is_empty());
    }
    #[test]
    fn invalidate_stops_validate() {
        let mut p = FakePlatform::new();
        let lease = p.bind_surface("cand-1").unwrap();
        p.push_script(FakeScript::InvalidateLease);
        match p.validate_surface(&lease).unwrap() {
            ValidateOutcome::Invalid { .. } => {}
            other => panic!("expected invalid, got {other:?}"),
        }
    }
    #[test]
    fn rebind_required_blocks_old_lease_capture_until_explicit_rebind() {
        let mut p = FakePlatform::new();
        let old = p.bind_surface("cand-1").unwrap();
        p.push_script(FakeScript::RebindRequired);
        match p.validate_surface(&old).unwrap() {
            ValidateOutcome::RebindRequired { .. } => {}
            other => panic!("expected rebind_required, got {other:?}"),
        }
        // Capture with the original lease must fail; success would mask the reject.
        p.push_script(FakeScript::RebindRequired);
        let err = p.capture_frame(&old, 100).unwrap_err();
        assert!(
            err.to_string().contains("rebind") || err.to_string().contains("LEASE"),
            "unexpected: {err}"
        );
        let fresh = p.bind_surface("cand-1").unwrap();
        assert_ne!(fresh.lease_id, old.lease_id);
        assert!(fresh.generation > old.generation);
        assert!(p.capture_frame(&fresh, 100).is_ok());
    }
}

//! Windows platform adapter — identity + single-window capture.
//! Real SendInput is gated behind `input_enabled`; diagnostics never inject input.

mod capture;
mod discover;
mod glance;
mod input;
mod launch;
mod locator_gate;
mod ocr_bundle;
mod rapid_ocr;
mod captcha_dddd;
mod session;
mod device_diagnostics;
pub use device_diagnostics::device_diagnostics;

pub mod evidence;
pub mod ocr;

pub use evidence::{draw_boxes_bgra, encode_bgra_bmp};

use std::collections::HashMap;

use chrono::Local;
use vision_contracts::{
    lease_expired, CapabilityReport, CapabilityStatus, CaptureBackend, Frame, GuardedAction,
    PlatformKind, ReasonCode, SurfaceCandidate, TargetSurfaceLease, ValidateOutcome,
};
use vision_platform_api::{
    ActivationStatus, CaptureError, InputError, InputReceipt, PlatformDiagnostics, PlatformError,
    PlatformPort,
};

use capture::capture_bound_window;

pub use capture::{CapturePreference, CaptureResult, WgcCaptureSession};
pub use discover::{
    discover_wechat_candidates, title_is_miniprogram as is_miniprogram_title,
    title_is_wechat as is_wechat_title, NativeSurface,
};
pub use glance::{
    click_wechat_step, click_wechat_step_run, glance_wechat, glance_wechat_run, DateCardSummary,
    DateEvidenceSummary, GlanceMode, GlanceReport,
};
pub use session::{PendingEvidenceFrame, RunSession, SessionObservation};
pub use launch::{
    foreground_miniprogram_window, launch_and_wait, launch_and_wait_n, launch_fail_code, launch_url,
    launch_xmu_miniprogram, LaunchResult, DEFAULT_APP_ID, DEFAULT_LAUNCH_ATTEMPTS,
    DEFAULT_LAUNCH_WAIT_MS, LAUNCH_MP_NEED_DESKTOP, LAUNCH_MP_NEED_DESKTOP_MSG, LAUNCH_RETRY_GAP_MS,
};
pub use captcha_dddd::recognize_captcha_bgra;
pub use ocr_bundle::{
    find_bundle_ocr_root, seed_ocr_cache, seed_ocr_cache_from_resource_dir, SeedReport,
};

/// The action was rejected before any input was sent. The host may discard the
/// pending token, rebind, and observe again; it must never replay the old plan.
pub const PRE_INPUT_REBIND_PREFIX: &str = "PRE_INPUT_REBIND_REQUIRED:";
/// Guard rejected before SendInput; the caller may take a new observation and plan.
pub const PRE_INPUT_REOBSERVE_PREFIX: &str = "PRE_INPUT_REOBSERVE_REQUIRED:";

use discover::{title_is_miniprogram, title_is_wechat};

pub struct WindowsPlatform {
    pub input_enabled: bool,
    pub pack_ready: bool,
    pub capture_preference: CapturePreference,
    natives: HashMap<String, NativeSurface>,
    bound: Option<(String, TargetSurfaceLease)>,
    last_capture_notes: Vec<String>,
    last_backend: Option<CaptureBackend>,
}

impl WindowsPlatform {
    pub fn new() -> Self {
        Self {
            input_enabled: false,
            pack_ready: false,
            capture_preference: CapturePreference::GdiOnly,
            natives: HashMap::new(),
            bound: None,
            last_capture_notes: Vec::new(),
            last_backend: None,
        }
    }

    pub fn last_capture_backend(&self) -> Option<CaptureBackend> {
        self.last_backend
    }

    pub fn last_capture_notes(&self) -> &[String] {
        &self.last_capture_notes
    }

    pub fn bound_lease(&self) -> Option<&TargetSurfaceLease> {
        self.bound.as_ref().map(|(_, l)| l)
    }

    /// Preserve a monotonic generation when a RunSession rebuilds this adapter.
    /// The fresh adapter would otherwise restart at g1 and could reuse a lease ID.
    pub(crate) fn set_bound_generation(&mut self, generation: u64) -> Result<TargetSurfaceLease, String> {
        let (candidate_id, lease) = self.bound.as_mut().ok_or("no bound lease")?;
        let native = self.natives.get(candidate_id).ok_or("native missing")?;
        lease.generation = generation;
        lease.lease_id = format!("lease-win-{}-g{generation}", native.hwnd_u64);
        Ok(lease.clone())
    }
}

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformPort for WindowsPlatform {
    fn check_capabilities(&mut self) -> Result<CapabilityReport, PlatformError> {
        if !self.pack_ready {
            return Ok(CapabilityReport {
                status: CapabilityStatus::NotInstalled,
                reason_code: ReasonCode::PACK_NOT_INSTALLED.into(),
                platform: PlatformKind::Windows,
                pack_ready: false,
                input_enabled: self.input_enabled,
                details: None,
            });
        }
        Ok(CapabilityReport {
            status: CapabilityStatus::Ready,
            reason_code: "OK".into(),
            platform: PlatformKind::Windows,
            pack_ready: true,
            input_enabled: self.input_enabled,
            details: Some(serde_json::json!({
                "capture_preference": format!("{:?}", self.capture_preference),
                "last_backend": self.last_backend,
                "note": "single-window only; fullscreen fallback forbidden"
            })),
        })
    }

    fn discover_surfaces(
        &mut self,
        app_selector: &str,
    ) -> Result<Vec<SurfaceCandidate>, PlatformError> {
        let want_mini = app_selector.contains("miniprogram") || app_selector.contains("厦大");
        let found = discover_wechat_candidates().map_err(PlatformError::Message)?;
        self.natives.clear();
        let mut out = Vec::new();
        for n in found {
            let is_mini = title_is_miniprogram(&n.title);
            let is_wx = title_is_wechat(&n.title);
            if want_mini && !is_mini && !is_wx {
                continue;
            }
            if !want_mini && !is_wx && !is_mini {
                continue;
            }
            let cand = n.to_candidate();
            self.natives.insert(cand.candidate_id.clone(), n);
            out.push(cand);
        }
        Ok(out)
    }

    fn bind_surface(&mut self, candidate_id: &str) -> Result<TargetSurfaceLease, PlatformError> {
        let native = self
            .natives
            .get(candidate_id)
            .ok_or_else(|| PlatformError::Message(format!("unknown candidate {candidate_id}")))?;
        let refreshed = discover::refresh_native(native.hwnd()).map_err(PlatformError::Message)?;
        if refreshed.surface_fingerprint != native.surface_fingerprint
            || refreshed.owner.pid != native.owner.pid
            || refreshed.owner.launch_identity != native.owner.launch_identity
        {
            return Err(PlatformError::Message(
                "identity changed between discover and bind".into(),
            ));
        }
        // Explicit rebind bumps generation; first bind starts at 1.
        let next_gen = self
            .bound
            .as_ref()
            .filter(|(cid, _)| cid.as_str() == candidate_id)
            .map(|(_, l)| l.generation + 1)
            .unwrap_or(1);
        let lease = refreshed.to_lease(next_gen);
        self.natives.insert(candidate_id.to_string(), refreshed);
        self.bound = Some((candidate_id.to_string(), lease.clone()));
        Ok(lease)
    }

    fn validate_surface(
        &mut self,
        lease: &TargetSurfaceLease,
    ) -> Result<ValidateOutcome, PlatformError> {
        if lease_expired(&lease.expires_at, Local::now()).unwrap_or(true) {
            return Ok(ValidateOutcome::Invalid {
                reason: ReasonCode::LEASE_EXPIRED.into(),
            });
        }
        let Some((cid, bound)) = &self.bound else {
            return Ok(ValidateOutcome::Invalid {
                reason: "no_bound_lease".into(),
            });
        };
        if bound.lease_id != lease.lease_id {
            return Ok(ValidateOutcome::Invalid {
                reason: "lease_id_mismatch".into(),
            });
        }
        let native = self
            .natives
            .get(cid)
            .ok_or_else(|| PlatformError::Message("native missing".into()))?;
        let refreshed = match discover::refresh_native(native.hwnd()) {
            Ok(r) => r,
            Err(e) => {
                return Ok(ValidateOutcome::Invalid { reason: e });
            }
        };
        if refreshed.owner.pid != lease.owner.pid
            || refreshed.owner.launch_identity != lease.owner.launch_identity
            || refreshed.owner.app_identity != lease.owner.app_identity
        {
            return Ok(ValidateOutcome::Invalid {
                reason: "owner_or_launch_identity_changed".into(),
            });
        }
        if !refreshed.visible {
            return Ok(ValidateOutcome::Invalid {
                reason: ReasonCode::TARGET_MINIMIZED.into(),
            });
        }
        let geom_changed = (refreshed.bounds.width - lease.bounds_in_screen_points.width).abs()
            > 2.0
            || (refreshed.bounds.height - lease.bounds_in_screen_points.height).abs() > 2.0
            || (refreshed.scale_x - lease.scale_x).abs() > 0.01
            || (refreshed.bounds.x - lease.bounds_in_screen_points.x).abs() > 2.0
            || (refreshed.bounds.y - lease.bounds_in_screen_points.y).abs() > 2.0;
        if geom_changed || refreshed.surface_fingerprint != lease.surface_fingerprint {
            log::warn!(
                target: "vision_surface",
                "[lease_validate] outcome=rebind_required reason=geometry_or_fingerprint_changed lease_id={} generation={} hwnd={} pid={} geometry_changed={} fingerprint_changed={} old_class={} new_class={} old_bounds={:?} new_bounds={:?} old_scale=({:.3},{:.3}) new_scale=({:.3},{:.3})",
                lease.lease_id,
                lease.generation,
                refreshed.hwnd_u64,
                refreshed.owner.pid,
                geom_changed,
                refreshed.surface_fingerprint != lease.surface_fingerprint,
                native.class_name,
                refreshed.class_name,
                lease.bounds_in_screen_points,
                refreshed.bounds,
                lease.scale_x,
                lease.scale_y,
                refreshed.scale_x,
                refreshed.scale_y,
            );
            // Keep the bound lease unchanged. RebindRequired means the caller must
            // call bind_surface explicitly; silent lease swap would hide rejection.
            self.natives.insert(cid.clone(), refreshed);
            return Ok(ValidateOutcome::RebindRequired {
                reason: "geometry_or_fingerprint_changed".into(),
            });
        }
        // Refresh title/bounds without bumping generation when still valid.
        let mut updated = lease.clone();
        updated.title_hint = refreshed.title.clone();
        updated.bounds_in_screen_points = refreshed.bounds.clone();
        self.natives.insert(cid.clone(), refreshed);
        self.bound = Some((cid.clone(), updated));
        Ok(ValidateOutcome::Valid)
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
        let (cid, current) = self
            .bound
            .as_ref()
            .ok_or_else(|| CaptureError::Message("no bound lease".into()))?;
        let native = self
            .natives
            .get(cid)
            .ok_or_else(|| CaptureError::Message("native missing".into()))?
            .clone();
        let current = current.clone();
        let result = capture_bound_window(&native, &current, self.capture_preference)
            .map_err(CaptureError::Message)?;
        self.last_backend = Some(result.backend_used);
        self.last_capture_notes = result.notes;
        Ok(result.frame)
    }

    fn activation_status(
        &mut self,
        lease: &TargetSurfaceLease,
    ) -> Result<ActivationStatus, PlatformError> {
        let Some((cid, _)) = &self.bound else {
            return Ok(ActivationStatus::Denied);
        };
        if self.bound.as_ref().map(|(_, l)| l.lease_id.as_str()) != Some(lease.lease_id.as_str()) {
            return Ok(ActivationStatus::Denied);
        }
        let native = self
            .natives
            .get(cid)
            .ok_or_else(|| PlatformError::Message("native missing".into()))?;
        Ok(discover::activation_status(native.hwnd()))
    }

    fn activate_surface(
        &mut self,
        lease: &TargetSurfaceLease,
        explicit_authority: bool,
    ) -> Result<(), PlatformError> {
        if !explicit_authority {
            return Err(PlatformError::Message("authority required".into()));
        }
        let Some((cid, bound)) = &self.bound else {
            return Err(PlatformError::Message("no bound lease".into()));
        };
        if bound.lease_id != lease.lease_id {
            return Err(PlatformError::Message("lease mismatch".into()));
        }
        let native = self
            .natives
            .get(cid)
            .ok_or_else(|| PlatformError::Message("native missing".into()))?;
        discover::activate(native.hwnd()).map_err(PlatformError::Message)
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
        input::inject_guarded(action, lease, fresh_frame)
    }

    fn collect_diagnostics(&mut self) -> Result<PlatformDiagnostics, PlatformError> {
        let mut lines: Vec<String> = self
            .natives
            .values()
            .map(|n| {
                format!(
                    "hwnd={} pid={} title={} scale={:.2} fp={}",
                    n.hwnd_u64, n.owner.pid, n.title, n.scale_x, n.surface_fingerprint
                )
            })
            .collect();
        if let Some(b) = self.last_backend {
            lines.push(format!("last_capture_backend={b:?}"));
        }
        lines.extend(self.last_capture_notes.iter().cloned());
        Ok(PlatformDiagnostics {
            redacted_lines: lines,
        })
    }

    fn release_surface(&mut self, _lease: &TargetSurfaceLease) -> Result<(), PlatformError> {
        self.bound = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vision_contracts::lease_issue_and_expiry;

    #[test]
    fn expired_lease_invalid() {
        let p = WindowsPlatform::new();
        let (_issued, expires) = lease_issue_and_expiry(1800);
        let expired = "2000-01-01T00:00:00+00:00";
        assert!(lease_expired(expired, Local::now()).unwrap());
        assert!(!lease_expired(&expires, Local::now()).unwrap());
        let _ = p;
    }

    #[cfg(windows)]
    #[test]
    fn rebound_adapter_keeps_generation_in_lease_and_platform_state() {
        let native = NativeSurface {
            hwnd_usize: 9,
            hwnd_u64: 9,
            title: "test".into(),
            class_name: "test".into(),
            owner: vision_contracts::OwnerIdentity {
                pid: 1,
                app_identity: "app".into(),
                launch_identity: "launch".into(),
            },
            bounds: vision_contracts::RectF {
                x: 0.0, y: 0.0, width: 100.0, height: 100.0,
            },
            scale_x: 1.0,
            scale_y: 1.0,
            surface_fingerprint: "fp".into(),
            visible: true,
            area: 10_000,
        };
        let mut platform = WindowsPlatform::new();
        platform.natives.insert("win-9".into(), native.clone());
        platform.bound = Some(("win-9".into(), native.to_lease(1)));
        let lease = platform.set_bound_generation(2).unwrap();
        assert_eq!(lease.generation, 2);
        assert_eq!(lease.lease_id, "lease-win-9-g2");
        assert_eq!(platform.bound_lease(), Some(&lease));
    }
}

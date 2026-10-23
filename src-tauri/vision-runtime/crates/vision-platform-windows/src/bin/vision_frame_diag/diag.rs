//! Single-frame diagnostic helpers — no input injection; write under LOCALAPPDATA only.
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use vision_contracts::{
    wall_now_rfc3339, CaptureBackend, Frame, FrameStatus, TargetSurfaceLease, ValidateOutcome,
};
use vision_platform_api::PlatformPort;
use vision_platform_windows::{CapturePreference, WindowsPlatform};
#[derive(Debug, Serialize)]
pub struct DiagCandidate {
    pub index: usize,
    pub candidate_id: String,
    pub title_hint: String,
    pub pid: u32,
    pub app_identity_leaf: String,
    pub bounds: serde_json::Value,
    pub surface_fingerprint_hash: String,
}
#[derive(Debug, Serialize)]
pub struct FrameDiagReport {
    pub ts: String,
    pub scenario: String,
    pub backend_requested: String,
    pub backend_used: Option<CaptureBackend>,
    pub notes: Vec<String>,
    pub validate: String,
    pub reason_code: Option<String>,
    pub lease: Option<serde_json::Value>,
    pub frame_meta: Option<serde_json::Value>,
    pub bmp_path: Option<String>,
    pub meta_path: String,
    pub input_injected: bool,
}
/// Evidence that an *old* lease was checked after a window change, separate from rebind.
#[derive(Debug, Clone, Serialize)]
pub struct ValidateAfterEvidence {
    pub outcome: String,
    pub reason: Option<String>,
    pub original_generation: u64,
    pub original_lease_id: String,
    /// After validate, bound lease_id must still match the original (no silent swap).
    pub bound_lease_id_unchanged: bool,
    pub bound_generation_unchanged: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct CaptureAttemptEvidence {
    pub attempted_with: String,
    pub ok: bool,
    pub reason_code: Option<String>,
    pub error: Option<String>,
    pub bmp_path: Option<String>,
    pub frame_meta: Option<serde_json::Value>,
}
/// Scenario after-change report: old-lease rejection must not be masked by rebind success.
#[derive(Debug, Serialize)]
pub struct ScenarioAfterReport {
    pub ts: String,
    pub scenario: String,
    pub backend_requested: String,
    pub input_injected: bool,
    pub original_lease: serde_json::Value,
    pub validate_after: ValidateAfterEvidence,
    pub old_lease_capture: CaptureAttemptEvidence,
    pub rebind_capture: Option<FrameDiagReport>,
    pub rebind_error: Option<String>,
    pub meta_path: String,
    pub evidence_ok_hint: String,
}
fn leaf_path(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string())
}
fn fp_hash(fp: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    fp.hash(&mut h);
    format!("{:016x}", h.finish())
}
pub fn default_diag_root() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base)
        .join("xmu-sports")
        .join("diag")
        .join("frames")
}
pub fn list_candidates(plat: &mut WindowsPlatform) -> Result<Vec<DiagCandidate>, String> {
    let cands = plat
        .discover_surfaces("wechat")
        .map_err(|e| e.to_string())?;
    Ok(cands
        .into_iter()
        .enumerate()
        .map(|(i, c)| DiagCandidate {
            index: i,
            candidate_id: c.candidate_id,
            title_hint: c.title_hint,
            pid: c.owner.pid,
            app_identity_leaf: leaf_path(&c.owner.app_identity),
            bounds: serde_json::json!({
                "x": c.bounds_in_screen_points.x,
                "y": c.bounds_in_screen_points.y,
                "width": c.bounds_in_screen_points.width,
                "height": c.bounds_in_screen_points.height,
            }),
            surface_fingerprint_hash: fp_hash(&c.surface_fingerprint),
        })
        .collect())
}
fn write_bgra_bmp(path: &Path, width: u32, height: u32, bgra: &[u8]) -> Result<(), String> {
    let row_stride = width * 4;
    let pixel_bytes = (row_stride * height) as usize;
    if bgra.len() < pixel_bytes {
        return Err("pixel buffer too small".into());
    }
    // BMP wants BGR bottom-up without alpha typically; write 32-bit BI_BITFIELDS-ish BI_RGB.
    let file_header_size = 14u32;
    let info_size = 40u32;
    let off = file_header_size + info_size;
    let size = off + pixel_bytes as u32;
    let mut f = fs::File::create(path).map_err(|e| e.to_string())?;
    f.write_all(b"BM").map_err(|e| e.to_string())?;
    f.write_all(&size.to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&0u16.to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&0u16.to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&off.to_le_bytes()).map_err(|e| e.to_string())?;
    f.write_all(&info_size.to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&(width as i32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    // negative height = top-down
    f.write_all(&(-(height as i32)).to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&1u16.to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&32u16.to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&0u32.to_le_bytes())
        .map_err(|e| e.to_string())?; // BI_RGB
    f.write_all(&(pixel_bytes as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&0i32.to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&0i32.to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&0u32.to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&0u32.to_le_bytes())
        .map_err(|e| e.to_string())?;
    f.write_all(&bgra[..pixel_bytes])
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn redact_lease(lease: &TargetSurfaceLease) -> serde_json::Value {
    serde_json::json!({
        "lease_id": lease.lease_id,
        "generation": lease.generation,
        "title_hint": lease.title_hint,
        "pid": lease.owner.pid,
        "app_identity_leaf": leaf_path(&lease.owner.app_identity),
        "launch_identity": lease.owner.launch_identity,
        "surface_fingerprint_hash": fp_hash(&lease.surface_fingerprint),
        "bounds_in_screen_points": {
            "x": lease.bounds_in_screen_points.x,
            "y": lease.bounds_in_screen_points.y,
            "width": lease.bounds_in_screen_points.width,
            "height": lease.bounds_in_screen_points.height,
        },
        "capture_pixel_size": {
            "width": lease.capture_pixel_size.width,
            "height": lease.capture_pixel_size.height,
        },
        "scale_x": lease.scale_x,
        "scale_y": lease.scale_y,
        "issued_at": lease.issued_at,
        "expires_at": lease.expires_at,
    })
}
fn frame_meta(frame: &Frame) -> serde_json::Value {
    let incomplete = matches!(frame.status, FrameStatus::Incomplete);
    serde_json::json!({
        "frame_id": frame.frame_id,
        "lease_id": frame.lease_id,
        "generation": frame.generation,
        "monotonic_capture_time_ms": frame.monotonic_capture_time_ms,
        "pixel_width": frame.pixel_width,
        "pixel_height": frame.pixel_height,
        "status": format!("{:?}", frame.status).to_lowercase(),
        "capture_backend": frame.capture_backend,
        "content_rect": {
            "x": frame.content_rect.x,
            "y": frame.content_rect.y,
            "width": frame.content_rect.width,
            "height": frame.content_rect.height,
        },
        "screen_transform": {
            "origin_x": frame.screen_transform.origin_x,
            "origin_y": frame.screen_transform.origin_y,
            "scale_x": frame.screen_transform.scale_x,
            "scale_y": frame.screen_transform.scale_y,
        },
        "usable": frame.is_usable(),
        "vision_usable": frame.is_usable(),
        "black": matches!(frame.status, FrameStatus::Black),
        "incomplete_or_low_info": incomplete,
        "content_gate": if frame.is_usable() { "pass_heuristic" } else { "fail_not_vision_usable" },
    })
}
pub fn format_validate_outcome(v: &ValidateOutcome) -> (String, Option<String>) {
    match v {
        ValidateOutcome::Valid => ("valid".into(), None),
        ValidateOutcome::RebindRequired { reason } => {
            ("rebind_required".into(), Some(reason.clone()))
        }
        ValidateOutcome::Invalid { reason } => ("invalid".into(), Some(reason.clone())),
    }
}
fn capture_error_code(msg: &str) -> String {
    msg.split(':')
        .next()
        .map(|s| s.to_string())
        .unwrap_or_else(|| msg.to_string())
}
fn evidence_ok_hint(
    validate: &ValidateAfterEvidence,
    old_cap: &CaptureAttemptEvidence,
    rebind_ok: bool,
) -> String {
    let old_rejected = !old_cap.ok;
    let validate_blocks = matches!(validate.outcome.as_str(), "rebind_required" | "invalid");
    if validate_blocks
        && old_rejected
        && validate.bound_lease_id_unchanged
        && validate.bound_generation_unchanged
    {
        if rebind_ok {
            "PASS_SHAPE: old lease blocked; explicit rebind captured".into()
        } else {
            "PASS_SHAPE: old lease blocked; rebind also failed (ok for close/minimize/gone)".into()
        }
    } else if !validate_blocks && old_cap.ok {
        "CHECK: no geometry/identity change detected — confirm scenario was applied".into()
    } else {
        "FAIL_SHAPE: expected validate reject + old_lease_capture fail without silent rebind".into()
    }
}
/// Capture one diagnostic frame for a confirmed candidate. Never injects input.
pub fn capture_one(
    plat: &mut WindowsPlatform,
    candidate_id: &str,
    scenario: &str,
    out_dir: &Path,
) -> Result<FrameDiagReport, String> {
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    // Hard enforce: diagnostics never enable live input.
    plat.input_enabled = false;
    let lease = plat.bind_surface(candidate_id).map_err(|e| e.to_string())?;
    let validate = plat.validate_surface(&lease).map_err(|e| e.to_string())?;
    let (outcome, reason) = format_validate_outcome(&validate);
    let validate_s = match reason {
        Some(r) => format!("{outcome}:{r}"),
        None => outcome,
    };
    let stamp = wall_now_rfc3339().replace(':', "").replace('+', "p");
    let meta_path = out_dir.join(format!("{scenario}_{stamp}.json"));
    let (frame_meta_v, bmp_path, reason, backend, extra_notes) =
        match plat.capture_frame(&lease, 2000) {
            Ok(frame) => {
                let bmp = out_dir.join(format!("{scenario}_{stamp}.bmp"));
                if let Some(px) = &frame.pixels {
                    write_bgra_bmp(&bmp, frame.pixel_width, frame.pixel_height, &px.bytes)?;
                }
                (
                    Some(frame_meta(&frame)),
                    Some(bmp.display().to_string()),
                    None,
                    Some(frame.capture_backend),
                    Vec::new(),
                )
            }
            Err(e) => {
                let msg = e.to_string();
                let code = capture_error_code(&msg);
                (
                    None,
                    None,
                    Some(code),
                    plat.last_capture_backend(),
                    vec![format!("capture_error:{msg}")],
                )
            }
        };
    let mut notes = plat.last_capture_notes().to_vec();
    notes.extend(extra_notes);
    let report = FrameDiagReport {
        ts: wall_now_rfc3339(),
        scenario: scenario.into(),
        backend_requested: format!("{:?}", plat.capture_preference),
        backend_used: backend,
        notes,
        validate: validate_s,
        reason_code: reason,
        lease: Some(redact_lease(plat.bound_lease().unwrap_or(&lease))),
        frame_meta: frame_meta_v,
        bmp_path,
        meta_path: meta_path.display().to_string(),
        input_injected: false,
    };
    fs::write(
        &meta_path,
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(report)
}
/// After the operator changes the bound window: validate + capture with the *original*
/// lease (must persist rejection), then separately explicit-rebind capture.
pub fn scenario_after(
    plat: &mut WindowsPlatform,
    candidate_id: &str,
    original_lease: &TargetSurfaceLease,
    scenario_name: &str,
    out_dir: &Path,
) -> Result<ScenarioAfterReport, String> {
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    plat.input_enabled = false;
    let stamp = wall_now_rfc3339().replace(':', "").replace('+', "p");
    let scenario = format!("{scenario_name}_after");
    let meta_path = out_dir.join(format!("{scenario}_{stamp}.json"));
    let validate = plat
        .validate_surface(original_lease)
        .map_err(|e| e.to_string())?;
    let (outcome, reason) = format_validate_outcome(&validate);
    let bound_after = plat.bound_lease();
    let validate_after = ValidateAfterEvidence {
        outcome,
        reason,
        original_generation: original_lease.generation,
        original_lease_id: original_lease.lease_id.clone(),
        bound_lease_id_unchanged: bound_after
            .map(|l| l.lease_id == original_lease.lease_id)
            .unwrap_or(false),
        bound_generation_unchanged: bound_after
            .map(|l| l.generation == original_lease.generation)
            .unwrap_or(false),
    };
    let old_lease_capture = match plat.capture_frame(original_lease, 2000) {
        Ok(frame) => {
            // Unexpected success — still write BMP so the operator can see the leak.
            let bmp = out_dir.join(format!("{scenario}_old_lease_{stamp}.bmp"));
            if let Some(px) = &frame.pixels {
                let _ = write_bgra_bmp(&bmp, frame.pixel_width, frame.pixel_height, &px.bytes);
            }
            CaptureAttemptEvidence {
                attempted_with: "original_lease".into(),
                ok: true,
                reason_code: None,
                error: Some(
                    "UNEXPECTED: original lease captured after scenario; expected reject".into(),
                ),
                bmp_path: Some(bmp.display().to_string()),
                frame_meta: Some(frame_meta(&frame)),
            }
        }
        Err(e) => {
            let msg = e.to_string();
            CaptureAttemptEvidence {
                attempted_with: "original_lease".into(),
                ok: false,
                reason_code: Some(capture_error_code(&msg)),
                error: Some(msg),
                bmp_path: None,
                frame_meta: None,
            }
        }
    };
    let (rebind_capture, rebind_error) = match capture_one(
        plat,
        candidate_id,
        &format!("{scenario_name}_after_rebind"),
        out_dir,
    ) {
        Ok(r) => (Some(r), None),
        Err(e) => (None, Some(e)),
    };
    let hint = evidence_ok_hint(
        &validate_after,
        &old_lease_capture,
        rebind_capture.is_some(),
    );
    let report = ScenarioAfterReport {
        ts: wall_now_rfc3339(),
        scenario,
        backend_requested: format!("{:?}", plat.capture_preference),
        input_injected: false,
        original_lease: redact_lease(original_lease),
        validate_after,
        old_lease_capture,
        rebind_capture,
        rebind_error,
        meta_path: meta_path.display().to_string(),
        evidence_ok_hint: hint,
    };
    fs::write(
        &meta_path,
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(report)
}
pub fn parse_backend(s: &str) -> Result<CapturePreference, String> {
    match s.to_ascii_lowercase().as_str() {
        "gdi" | "gdi_only" => Ok(CapturePreference::GdiOnly),
        "wgc" | "wgc_only" => Ok(CapturePreference::WgcOnly),
        "auto" => Ok(CapturePreference::Auto),
        other => Err(format!("unknown backend {other} (gdi|wgc|auto)")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use vision_contracts::{OwnerIdentity, PlatformKind, ReasonCode, RectF, SizeI};
    fn sample_lease(generation: u64) -> TargetSurfaceLease {
        TargetSurfaceLease {
            lease_id: "lease-diag-1".into(),
            generation,
            platform: PlatformKind::Windows,
            owner: OwnerIdentity {
                pid: 42,
                app_identity: r"C:\WeChat\WeChat.exe".into(),
                launch_identity: "launch-1".into(),
            },
            surface_fingerprint: "fp".into(),
            title_hint: "微信".into(),
            bounds_in_screen_points: RectF {
                x: 10.0,
                y: 20.0,
                width: 800.0,
                height: 600.0,
            },
            capture_pixel_size: SizeI {
                width: 800,
                height: 600,
            },
            scale_x: 1.0,
            scale_y: 1.0,
            issued_at: "2026-09-18T00:00:00+08:00".into(),
            expires_at: "2026-09-18T00:30:00+08:00".into(),
        }
    }
    #[test]
    fn bmp_header_roundtrip_tiny() {
        let dir = std::env::temp_dir().join("xmu-sports-diag-test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("t.bmp");
        let pixels = vec![
            0u8, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255,
        ];
        write_bgra_bmp(&path, 2, 2, &pixels).unwrap();
        assert!(path.is_file());
        let _ = fs::remove_file(path);
    }
    #[test]
    fn format_validate_splits_outcome_and_reason() {
        let (o, r) = format_validate_outcome(&ValidateOutcome::RebindRequired {
            reason: "geometry_or_fingerprint_changed".into(),
        });
        assert_eq!(o, "rebind_required");
        assert_eq!(r.as_deref(), Some("geometry_or_fingerprint_changed"));
    }
    #[test]
    fn evidence_hint_requires_old_lease_reject_before_rebind_success() {
        let lease = sample_lease(1);
        let validate = ValidateAfterEvidence {
            outcome: "rebind_required".into(),
            reason: Some("geometry_or_fingerprint_changed".into()),
            original_generation: lease.generation,
            original_lease_id: lease.lease_id.clone(),
            bound_lease_id_unchanged: true,
            bound_generation_unchanged: true,
        };
        let old_cap = CaptureAttemptEvidence {
            attempted_with: "original_lease".into(),
            ok: false,
            reason_code: Some(ReasonCode::LEASE_REBIND_REQUIRED.into()),
            error: Some("LEASE_REBIND_REQUIRED:geometry_or_fingerprint_changed".into()),
            bmp_path: None,
            frame_meta: None,
        };
        let hint = evidence_ok_hint(&validate, &old_cap, true);
        assert!(hint.starts_with("PASS_SHAPE"), "{hint}");
        // Rebind success alone must not pass if old lease was not rejected.
        let bad_old = CaptureAttemptEvidence {
            ok: true,
            error: None,
            reason_code: None,
            ..old_cap.clone()
        };
        let fail = evidence_ok_hint(&validate, &bad_old, true);
        assert!(fail.starts_with("FAIL_SHAPE"), "{fail}");
    }
    #[test]
    fn scenario_after_report_json_keeps_sections_separate() {
        let lease = sample_lease(1);
        let report = ScenarioAfterReport {
            ts: "t".into(),
            scenario: "move_after".into(),
            backend_requested: "GdiOnly".into(),
            input_injected: false,
            original_lease: redact_lease(&lease),
            validate_after: ValidateAfterEvidence {
                outcome: "rebind_required".into(),
                reason: Some("geometry_or_fingerprint_changed".into()),
                original_generation: 1,
                original_lease_id: lease.lease_id.clone(),
                bound_lease_id_unchanged: true,
                bound_generation_unchanged: true,
            },
            old_lease_capture: CaptureAttemptEvidence {
                attempted_with: "original_lease".into(),
                ok: false,
                reason_code: Some(ReasonCode::LEASE_REBIND_REQUIRED.into()),
                error: Some("LEASE_REBIND_REQUIRED:geometry_or_fingerprint_changed".into()),
                bmp_path: None,
                frame_meta: None,
            },
            rebind_capture: None,
            rebind_error: Some("window_gone".into()),
            meta_path: "x.json".into(),
            evidence_ok_hint:
                "PASS_SHAPE: old lease blocked; rebind also failed (ok for close/minimize/gone)"
                    .into(),
        };
        let v = serde_json::to_value(&report).unwrap();
        assert!(v.get("validate_after").is_some());
        assert!(v.get("old_lease_capture").is_some());
        assert!(v.get("rebind_capture").is_some() || v.get("rebind_error").is_some());
        assert_eq!(v["old_lease_capture"]["ok"], false);
        assert_eq!(v["validate_after"]["original_generation"], 1);
        assert_eq!(v["input_injected"], false);
        // Must not collapse into a single successful capture-shaped report.
        assert!(v.get("bmp_path").is_none());
        assert!(v.get("usable").is_none());
    }
}

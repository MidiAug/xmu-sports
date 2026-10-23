//! Capture backends: GDI (diagnostic) and WGC (HWND-bound). Never fullscreen crop.

mod content;
mod gdi;
mod wgc;

pub use wgc::WgcCaptureSession;

use vision_contracts::{CaptureBackend, Frame, ReasonCode, TargetSurfaceLease};

use crate::discover::NativeSurface;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturePreference {
    /// Prefer WGC; on failure return structured error (no silent fullscreen).
    WgcOnly,
    /// GDI BitBlt / PrintWindow path only.
    GdiOnly,
    /// Try WGC first; if unavailable, use GDI only when GDI content is vision-usable.
    Auto,
}

pub struct CaptureResult {
    pub frame: Frame,
    pub backend_requested: CapturePreference,
    pub backend_used: CaptureBackend,
    pub notes: Vec<String>,
}

pub fn capture_bound_window(
    native: &NativeSurface,
    lease: &TargetSurfaceLease,
    pref: CapturePreference,
) -> Result<CaptureResult, String> {
    match pref {
        CapturePreference::GdiOnly => {
            let (frame, mut notes) = gdi::capture_window_gdi_with_notes(native, lease)?;
            notes.insert(0, "gdi_only".into());
            Ok(CaptureResult {
                frame,
                backend_requested: pref,
                backend_used: CaptureBackend::GdiWindow,
                notes,
            })
        }
        CapturePreference::WgcOnly => {
            let (frame, mut notes) = wgc::capture_window_wgc_with_notes(native, lease)
                .map_err(|e| format!("{}:{e}", ReasonCode::WGC_UNAVAILABLE))?;
            notes.insert(0, "wgc_only".into());
            Ok(CaptureResult {
                frame,
                backend_requested: pref,
                backend_used: CaptureBackend::WgcHwnd,
                notes,
            })
        }
        CapturePreference::Auto => match wgc::capture_window_wgc_with_notes(native, lease) {
            Ok((frame, mut notes)) => {
                notes.insert(0, "auto_chose_wgc".into());
                if !frame.is_usable() {
                    notes.push(
                        "auto_wgc_content_not_vision_usable:returned_for_diag_no_gdi_mask".into(),
                    );
                }
                Ok(CaptureResult {
                    frame,
                    backend_requested: pref,
                    backend_used: CaptureBackend::WgcHwnd,
                    notes,
                })
            }
            Err(wgc_err) => {
                let (frame, mut notes) = gdi::capture_window_gdi_with_notes(native, lease)?;
                notes.insert(0, format!("auto_wgc_failed:{wgc_err}"));
                if !frame.is_usable() {
                    return Err(format!(
                        "{}:auto_refused_unusable_gdi_fallback;wgc={wgc_err};gdi_status={:?};{}",
                        ReasonCode::CONTENT_UNUSABLE,
                        frame.status,
                        notes.join("|")
                    ));
                }
                notes.push("auto_fell_back_to_gdi_window_not_fullscreen".into());
                Ok(CaptureResult {
                    frame,
                    backend_requested: pref,
                    backend_used: CaptureBackend::GdiWindow,
                    notes,
                })
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_names_stable() {
        assert_eq!(format!("{:?}", CapturePreference::GdiOnly), "GdiOnly");
    }
}

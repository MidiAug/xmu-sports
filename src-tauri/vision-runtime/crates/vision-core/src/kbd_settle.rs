//! Soft-keyboard settle gate: wait until bounce/overshoot stops before key mapping.
//!
//! WeChat IME sheet slides up, overshoots, then drops to rest. Mapping on the
//! peak frame puts「完成」~30px too high (see release logs y≈447 vs settled ≈481).
//! Industry pattern: consecutive identical frames / landmark cy (UiAutomator
//! `requireStableScreenshot`, Spectre `waitForVisualIdle`).

use crate::candidates::{CandidateKind, CandidateSet};
use crate::ocr::labels_match_exact;

/// Consecutive identical bottom-band fingerprints required.
pub const KBD_BAND_STREAK_NEED: u32 = 3;
/// 「完成」cy samples within this many px count as the same.
pub const KBD_CHROME_TOL_PX: i32 = 2;
/// Consecutive chrome cy hits within tol before accepting.
pub const KBD_CHROME_STREAK_NEED: u32 = 2;
/// Lower band starts at this fraction of frame height.
///
/// Must sit **below** the captcha input / blinking caret (~y 376–442 on 719px
/// frames). Chrome「完成」rests near y≈481 → use ~0.64 so only the IME sheet
/// is hashed (Playwright/Capybara: exclude caret from stability ROI).
pub const KBD_BAND_Y0_FRAC: f32 = 0.64;

/// Tracks bottom-band pixel fingerprints until the sheet stops moving.
#[derive(Debug, Clone)]
pub struct BandSettleTracker {
    last_fp: Option<u64>,
    streak: u32,
    need: u32,
}

impl Default for BandSettleTracker {
    fn default() -> Self {
        Self::new(KBD_BAND_STREAK_NEED)
    }
}

impl BandSettleTracker {
    pub fn new(need: u32) -> Self {
        Self {
            last_fp: None,
            streak: 0,
            need: need.max(1),
        }
    }

    pub fn streak(&self) -> u32 {
        self.streak
    }

    pub fn last_fp(&self) -> Option<u64> {
        self.last_fp
    }

    /// Feed one band fingerprint. Returns true when `need` consecutive matches.
    pub fn push(&mut self, fp: u64) -> bool {
        if self.last_fp == Some(fp) {
            self.streak = self.streak.saturating_add(1);
        } else {
            self.last_fp = Some(fp);
            self.streak = 1;
        }
        self.streak >= self.need
    }
}

/// Tracks OCR「完成」center-y until it stops drifting (bounce).
#[derive(Debug, Clone)]
pub struct ChromeCyTracker {
    last_cy: Option<i32>,
    streak: u32,
    need: u32,
    tol: i32,
}

impl Default for ChromeCyTracker {
    fn default() -> Self {
        Self::new(KBD_CHROME_STREAK_NEED, KBD_CHROME_TOL_PX)
    }
}

impl ChromeCyTracker {
    pub fn new(need: u32, tol: i32) -> Self {
        Self {
            last_cy: None,
            streak: 0,
            need: need.max(1),
            tol: tol.max(0),
        }
    }

    pub fn streak(&self) -> u32 {
        self.streak
    }

    pub fn last_cy(&self) -> Option<i32> {
        self.last_cy
    }

    /// Feed one「完成」cy. `None` resets the streak (chrome not visible yet).
    pub fn push(&mut self, cy: Option<i32>) -> bool {
        let Some(cy) = cy else {
            self.last_cy = None;
            self.streak = 0;
            return false;
        };
        if let Some(prev) = self.last_cy {
            if (cy - prev).abs() <= self.tol {
                self.streak = self.streak.saturating_add(1);
                // Keep last_cy as the running sample (prefer latest).
                self.last_cy = Some(cy);
            } else {
                self.last_cy = Some(cy);
                self.streak = 1;
            }
        } else {
            self.last_cy = Some(cy);
            self.streak = 1;
        }
        self.streak >= self.need
    }
}

/// Center-y of soft-key「完成」, if present.
pub fn keyboard_done_cy(candidates: &CandidateSet) -> Option<i32> {
    let hit = candidates.items.iter().find(|c| {
        (c.kind == CandidateKind::KeyboardKey || c.kind == CandidateKind::Text)
            && labels_match_exact(&c.label, "完成")
    })?;
    let b = hit.visual_bbox.as_ref().unwrap_or(&hit.bbox);
    Some((b.y0 + b.y1) / 2)
}

/// FNV-1a over the lower band of a BGRA frame (keyboard sheet + chrome).
pub fn bottom_band_fingerprint(pixels: &[u8], frame_w: u32, frame_h: u32, y0_frac: f32) -> u64 {
    if frame_w == 0 || frame_h == 0 {
        return 0;
    }
    let y0 = ((frame_h as f32) * y0_frac.clamp(0.0, 0.95)).floor() as u32;
    let y0 = y0.min(frame_h.saturating_sub(1));
    let mut hash = 0xcbf29ce484222325u64;
    let row_bytes = (frame_w as usize).saturating_mul(4);
    let start = (y0 as usize).saturating_mul(row_bytes);
    let slice = if start < pixels.len() {
        &pixels[start..]
    } else {
        &[]
    };
    // Stride sample: every 4th pixel keeps cost down on 517×719 WGC frames.
    let mut i = 0usize;
    while i + 4 <= slice.len() {
        let b = u32::from_le_bytes([slice[i], slice[i + 1], slice[i + 2], slice[i + 3]]);
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        i += 16; // skip 3 pixels
    }
    hash ^ ((frame_w as u64) << 32) ^ (frame_h as u64) ^ (y0 as u64)
}

/// Combined gate used by the host: band stable AND chrome cy stable (when seen).
#[derive(Debug, Clone, Default)]
pub struct KbdSettleGate {
    pub band: BandSettleTracker,
    pub chrome: ChromeCyTracker,
    pub saw_chrome: bool,
}

impl KbdSettleGate {
    pub fn new() -> Self {
        Self {
            band: BandSettleTracker::default(),
            chrome: ChromeCyTracker::default(),
            saw_chrome: false,
        }
    }

    /// Returns true when the keyboard sheet is settled enough to map keys.
    ///
    /// - Band fingerprint must be stable (`KBD_BAND_STREAK_NEED`).
    /// - Once「完成」has been seen at least once, its cy must also be stable.
    ///   (If OCR never sees chrome, band-only settle still arms — rare.)
    pub fn push(&mut self, band_fp: u64, done_cy: Option<i32>) -> bool {
        let band_ok = self.band.push(band_fp);
        if done_cy.is_some() {
            self.saw_chrome = true;
        }
        let chrome_ok = if self.saw_chrome {
            self.chrome.push(done_cy)
        } else {
            true
        };
        band_ok && chrome_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidates::{BBox, Candidate, CandidateKind, CandidateSet, CandidateSource};

    fn done_at(cy: i32) -> CandidateSet {
        let h = 24;
        let y0 = cy - h / 2;
        CandidateSet {
            items: vec![Candidate {
                id: "done".into(),
                label: "完成".into(),
                kind: CandidateKind::KeyboardKey,
                score: 0.9,
                bbox: BBox {
                    x0: 450,
                    y0,
                    x1: 510,
                    y1: y0 + h,
                },
                sources: vec![CandidateSource::Ocr],
                actionable: true,
                template_id: None,
                slot_meta: None,
                evidence: vec![],
                element_id: None,
                element_state: None,
                theme: None,
                visual_bbox: None,
                hit_bbox: None,
            }],
            dedup_iou: 0.5,
        }
    }

    #[test]
    fn band_streak_requires_consecutive_matches() {
        let mut t = BandSettleTracker::new(3);
        assert!(!t.push(1));
        assert!(!t.push(1));
        assert!(t.push(1));
        assert!(!t.push(2)); // reset
        assert!(!t.push(2));
        assert!(t.push(2));
    }

    #[test]
    fn chrome_cy_tolerates_one_px_jitter() {
        let mut t = ChromeCyTracker::new(2, 2);
        assert!(!t.push(Some(447)));
        assert!(t.push(Some(448))); // within tol → streak 2
    }

    #[test]
    fn chrome_cy_resets_on_bounce() {
        let mut t = ChromeCyTracker::new(3, 2);
        assert!(!t.push(Some(447))); // peak
        assert!(!t.push(Some(447)));
        assert!(!t.push(Some(481))); // drop resets
        assert!(!t.push(Some(481)));
        assert!(t.push(Some(482)));
    }

    #[test]
    fn chrome_none_resets() {
        let mut t = ChromeCyTracker::new(2, 2);
        assert!(!t.push(Some(480)));
        assert!(!t.push(None));
        assert_eq!(t.streak(), 0);
    }

    #[test]
    fn keyboard_done_cy_reads_center() {
        let set = done_at(481);
        assert_eq!(keyboard_done_cy(&set), Some(481));
    }

    #[test]
    fn gate_peak_band_differs_from_rest() {
        let mut g = KbdSettleGate::new();
        // Two peak frames — not enough band streak; then bounce changes band.
        assert!(!g.push(0xAAAA, Some(447)));
        assert!(!g.push(0xAAAA, Some(447)));
        assert!(!g.push(0xBBBB, Some(460)));
        assert!(!g.push(0xCCCC, Some(481)));
        assert!(!g.push(0xCCCC, Some(481)));
        assert!(g.push(0xCCCC, Some(481)));
    }

    #[test]
    fn bottom_band_fingerprint_stable_on_same_buffer() {
        let w = 40u32;
        let h = 80u32;
        let mut px = vec![200u8; (w * h * 4) as usize];
        // Paint lower band differently
        let y0 = ((h as f32) * KBD_BAND_Y0_FRAC) as u32;
        for y in y0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                px[i] = 250;
                px[i + 1] = 250;
                px[i + 2] = 250;
            }
        }
        let a = bottom_band_fingerprint(&px, w, h, KBD_BAND_Y0_FRAC);
        let b = bottom_band_fingerprint(&px, w, h, KBD_BAND_Y0_FRAC);
        assert_eq!(a, b);
        // Shift lower band content
        for y in y0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                px[i] = 240;
            }
        }
        let c = bottom_band_fingerprint(&px, w, h, KBD_BAND_Y0_FRAC);
        assert_ne!(a, c);
    }
}

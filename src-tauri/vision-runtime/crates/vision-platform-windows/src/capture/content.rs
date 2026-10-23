//! Heuristic content assessment for captured BGRA frames.
//!
//! Near-monochrome / low-variance frames are diagnostic warnings and must not be
//! treated as vision-usable. Pixel stats alone do not prove *why* capture failed
//! (wrong HWND vs empty GDI surface vs real dark UI) — humans still confirm.

use vision_contracts::FrameStatus;

#[derive(Debug, Clone, PartialEq)]
pub struct ContentAssessment {
    pub status: FrameStatus,
    pub near_monochrome: bool,
    pub all_zero: bool,
    pub sample_count: usize,
    pub unique_quantized: usize,
    pub max_channel: u8,
    pub min_channel: u8,
    pub notes: Vec<String>,
}

/// Assess BGRA8 pixels. Sampling keeps diag cheap on large frames.
pub fn assess_bgra(width: u32, height: u32, bgra: &[u8]) -> ContentAssessment {
    let w = width as usize;
    let h = height as usize;
    let need = w.saturating_mul(h).saturating_mul(4);
    if w == 0 || h == 0 || bgra.len() < need {
        return ContentAssessment {
            status: FrameStatus::Incomplete,
            near_monochrome: true,
            all_zero: true,
            sample_count: 0,
            unique_quantized: 0,
            max_channel: 0,
            min_channel: 0,
            notes: vec!["content:buffer_too_small_or_empty".into()],
        };
    }

    let total_px = w * h;
    // ~up to 64k samples; always include corners + center via step covering full range.
    let step = ((total_px / 64_000) + 1).max(1);
    let mut min_c = 255u8;
    let mut max_c = 0u8;
    let mut all_zero = true;
    let mut sample_count = 0usize;
    let mut buckets = [false; 512]; // 8³ coarse RGB buckets
    let mut unique = 0usize;

    let mut i = 0usize;
    while i < total_px {
        let o = i * 4;
        let b = bgra[o];
        let g = bgra[o + 1];
        let r = bgra[o + 2];
        if b | g | r != 0 {
            all_zero = false;
        }
        min_c = min_c.min(b).min(g).min(r);
        max_c = max_c.max(b).max(g).max(r);
        let qi = ((r as usize >> 5) << 6) | ((g as usize >> 5) << 3) | (b as usize >> 5);
        if !buckets[qi] {
            buckets[qi] = true;
            unique += 1;
        }
        sample_count += 1;
        i += step;
    }

    let range = max_c.saturating_sub(min_c);
    // Uniform BitBlt blanks (e.g. WeChat dark-gray fill) have tiny range and ≤2 quantized hues.
    // Do not treat high-contrast 2-color checkers as monochrome — require both signals.
    let near_monochrome = unique <= 2 && range <= 8;

    let mut notes = Vec::new();
    notes.push(format!(
        "content:samples={sample_count};unique_q={unique};min={min_c};max={max_c};range={range}"
    ));

    let status = if all_zero {
        notes.push("content:all_zero_black".into());
        FrameStatus::Black
    } else if near_monochrome {
        notes.push("content:near_monochrome_warning:not_vision_usable;human_confirm_cause".into());
        FrameStatus::Incomplete
    } else {
        FrameStatus::Ok
    };

    ContentAssessment {
        status,
        near_monochrome,
        all_zero,
        sample_count,
        unique_quantized: unique,
        max_channel: max_c,
        min_channel: min_c,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, bgr: [u8; 3]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            v.extend_from_slice(&[bgr[0], bgr[1], bgr[2], 0]);
        }
        v
    }

    #[test]
    fn uniform_dark_gray_is_incomplete_not_ok() {
        let px = solid(64, 64, [38, 37, 37]);
        let a = assess_bgra(64, 64, &px);
        assert!(a.near_monochrome);
        assert_eq!(a.status, FrameStatus::Incomplete);
        assert!(!a.all_zero);
    }

    #[test]
    fn all_zero_is_black() {
        let px = solid(16, 16, [0, 0, 0]);
        let a = assess_bgra(16, 16, &px);
        assert_eq!(a.status, FrameStatus::Black);
        assert!(a.all_zero);
    }

    #[test]
    fn varied_pixels_are_ok() {
        let mut px = solid(32, 32, [10, 10, 10]);
        // Several distinct colors so quantized uniqueness rises.
        for y in 0..32 {
            for x in 0..32 {
                let o = ((y * 32 + x) * 4) as usize;
                px[o] = (x * 7) as u8;
                px[o + 1] = (y * 9) as u8;
                px[o + 2] = ((x + y) * 5) as u8;
            }
        }
        let a = assess_bgra(32, 32, &px);
        assert!(!a.near_monochrome, "{a:?}");
        assert_eq!(a.status, FrameStatus::Ok);
    }
}

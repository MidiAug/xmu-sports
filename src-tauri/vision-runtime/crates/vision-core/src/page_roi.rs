//! Page-relative OCR bands for region-priority recognition (P4).
//!
//! Full-page OCR is the upgrade path; stable booking pages prefer these bands.

use crate::ocr::OcrHit;
use crate::state::Page;

/// Normalized ROI band in frame space (fractions of width/height).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageRoiBand {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    /// Upscale factor hint for small UI text (1.0 = native).
    pub scale: f32,
}

impl PageRoiBand {
    pub fn clamp(self) -> Self {
        Self {
            x0: self.x0.clamp(0.0, 1.0),
            y0: self.y0.clamp(0.0, 1.0),
            x1: self.x1.clamp(0.0, 1.0).max(self.x0),
            y1: self.y1.clamp(0.0, 1.0).max(self.y0),
            scale: self.scale.max(1.0),
        }
    }

    /// Pixel rect (inclusive-exclusive style: x1/y1 are exclusive caps).
    pub fn pixel_rect(self, frame_w: u32, frame_h: u32) -> Option<(u32, u32, u32, u32)> {
        let b = self.clamp();
        let x0 = (b.x0 * frame_w as f32).floor() as u32;
        let y0 = (b.y0 * frame_h as f32).floor() as u32;
        let x1 = (b.x1 * frame_w as f32).ceil() as u32;
        let y1 = (b.y1 * frame_h as f32).ceil() as u32;
        let x1 = x1.min(frame_w).max(x0.saturating_add(8));
        let y1 = y1.min(frame_h).max(y0.saturating_add(8));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some((x0, y0, x1, y1))
    }
}

/// Default ROI bands for pages that benefit from region-priority OCR.
pub fn page_roi_bands(page: Page) -> Vec<PageRoiBand> {
    match page {
        Page::VenueList => vec![PageRoiBand {
            x0: 0.04,
            y0: 0.12,
            x1: 0.96,
            y1: 0.88,
            scale: 1.0,
        }],
        Page::VenueDetail => vec![PageRoiBand {
            // Date strip: weekday + MD + 暂无|预定
            x0: 0.02,
            y0: 0.10,
            x1: 0.98,
            y1: 0.42,
            scale: 1.5,
        }],
        Page::SelectSlot => vec![PageRoiBand {
            x0: 0.02,
            y0: 0.14,
            x1: 0.98,
            y1: 0.82,
            scale: 1.25,
        }],
        Page::ConfirmOrder => vec![PageRoiBand {
            x0: 0.04,
            y0: 0.18,
            x1: 0.96,
            y1: 0.78,
            scale: 1.25,
        }],
        _ => Vec::new(),
    }
}

/// Pages that may skip full OCR when cache + ROI quality are good.
pub fn page_allows_roi_priority(page: Page) -> bool {
    !page_roi_bands(page).is_empty()
}

/// Whether ROI-only OCR produced enough usable hits.
pub fn roi_quality_ok(hits: &[OcrHit], min_hits: usize, min_avg_score: f32) -> bool {
    if hits.len() < min_hits {
        return false;
    }
    let avg = hits.iter().map(|h| h.score).sum::<f32>() / hits.len() as f32;
    avg >= min_avg_score
}

/// Merge base (usually full-page) with ROI hits. ROI wins on overlapping boxes.
///
/// Prefer [`merge_ocr_hits_replacing_bands`] when ROI bands were re-scanned so
/// stale cache text inside those bands is dropped even without a new hit.
pub fn merge_ocr_hits(base: &[OcrHit], roi: &[OcrHit], iou_thresh: f32) -> Vec<OcrHit> {
    let mut out: Vec<OcrHit> = Vec::with_capacity(base.len() + roi.len());
    for h in base {
        let overlaps_roi = roi.iter().any(|r| hit_iou(h, r) >= iou_thresh);
        if !overlaps_roi {
            out.push(h.clone());
        }
    }
    out.extend(roi.iter().cloned());
    out
}

fn hit_center_in_band(hit: &OcrHit, band: PageRoiBand, frame_w: u32, frame_h: u32) -> bool {
    let Some((x0, y0, x1, y1)) = band.pixel_rect(frame_w, frame_h) else {
        return false;
    };
    let cx = hit.x + hit.w / 2;
    let cy = hit.y + hit.h / 2;
    cx >= x0 as i32 && cx < x1 as i32 && cy >= y0 as i32 && cy < y1 as i32
}

/// Merge cache + ROI: drop any base hit whose center lies in a re-scanned band,
/// then add all ROI hits (ROI also wins on IoU overlap outside bands).
pub fn merge_ocr_hits_replacing_bands(
    base: &[OcrHit],
    roi: &[OcrHit],
    bands: &[PageRoiBand],
    frame_w: u32,
    frame_h: u32,
    iou_thresh: f32,
) -> Vec<OcrHit> {
    let mut out: Vec<OcrHit> = Vec::with_capacity(base.len() + roi.len());
    for h in base {
        if bands
            .iter()
            .any(|b| hit_center_in_band(h, *b, frame_w, frame_h))
        {
            continue;
        }
        let overlaps_roi = roi.iter().any(|r| hit_iou(h, r) >= iou_thresh);
        if !overlaps_roi {
            out.push(h.clone());
        }
    }
    out.extend(roi.iter().cloned());
    out
}

fn hit_iou(a: &OcrHit, b: &OcrHit) -> f32 {
    let ax1 = a.x + a.w;
    let ay1 = a.y + a.h;
    let bx1 = b.x + b.w;
    let by1 = b.y + b.h;
    let ix0 = a.x.max(b.x);
    let iy0 = a.y.max(b.y);
    let ix1 = ax1.min(bx1);
    let iy1 = ay1.min(by1);
    let iw = (ix1 - ix0).max(0);
    let ih = (iy1 - iy0).max(0);
    let inter = (iw * ih) as f32;
    if inter <= 0.0 {
        return 0.0;
    }
    let uni = (a.w * a.h + b.w * b.h) as f32 - inter;
    if uni <= 0.0 {
        0.0
    } else {
        inter / uni
    }
}

/// Soft age after which a cached full-page OCR should be refreshed (ms).
pub const FULL_OCR_STALE_MS: u64 = 4_000;

/// Decide whether this observe must run full-page OCR.
pub fn need_full_ocr(
    cached_page: Option<Page>,
    cache_age_ms: Option<u64>,
    cache_quality: Option<f32>,
    force_date_roi_zoom: bool,
    has_cache_hits: bool,
) -> bool {
    if force_date_roi_zoom {
        // Still want a full baseline when zooming date strip into a weak frame.
        return !has_cache_hits;
    }
    if !has_cache_hits {
        return true;
    }
    let Some(page) = cached_page else {
        return true;
    };
    if !page_allows_roi_priority(page) {
        return true;
    }
    if cache_quality.unwrap_or(0.0) < 0.4 {
        return true;
    }
    if cache_age_ms.unwrap_or(FULL_OCR_STALE_MS + 1) >= FULL_OCR_STALE_MS {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(text: &str, x: i32, y: i32) -> OcrHit {
        OcrHit {
            text: text.into(),
            x,
            y,
            w: 40,
            h: 16,
            score: 0.9,
        }
    }

    #[test]
    fn venue_detail_has_date_band() {
        let bands = page_roi_bands(Page::VenueDetail);
        assert_eq!(bands.len(), 1);
        assert!(bands[0].y1 <= 0.5);
        assert!(bands[0].scale >= 1.4);
    }

    #[test]
    fn unknown_page_no_roi() {
        assert!(page_roi_bands(Page::Unknown).is_empty());
        assert!(!page_allows_roi_priority(Page::Home));
    }

    #[test]
    fn merge_roi_wins_overlap() {
        let base = vec![hit("旧", 10, 10), hit("场馆", 200, 200)];
        let roi = vec![hit("新", 12, 12)];
        let merged = merge_ocr_hits(&base, &roi, 0.3);
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|h| h.text.contains("新")));
        assert!(merged.iter().any(|h| h.text.contains("场馆")));
        assert!(!merged.iter().any(|h| h.text.contains("旧")));
    }

    #[test]
    fn merge_replacing_bands_drops_stale_in_band() {
        // VenueDetail date band is roughly y 0.08–0.42 of the frame.
        let band = page_roi_bands(Page::VenueDetail)[0];
        let base = vec![
            hit("09月27日", 40, 100), // inside date band, stale
            hit("羽毛球", 40, 500),   // outside band
        ];
        let roi = vec![hit("09月29日", 80, 110)];
        let merged = merge_ocr_hits_replacing_bands(&base, &roi, &[band], 400, 800, 0.3);
        assert!(merged.iter().any(|h| h.text.contains("09月29日")));
        assert!(merged.iter().any(|h| h.text.contains("羽毛球")));
        assert!(
            !merged.iter().any(|h| h.text.contains("09月27日")),
            "stale date inside re-scanned band must be dropped"
        );
    }

    #[test]
    fn roi_quality_requires_hits_and_score() {
        assert!(!roi_quality_ok(&[], 3, 0.5));
        let weak = vec![
            OcrHit {
                text: "a".into(),
                x: 0,
                y: 0,
                w: 10,
                h: 10,
                score: 0.2,
            };
            5
        ];
        assert!(!roi_quality_ok(&weak, 3, 0.5));
        let ok = vec![hit("一", 0, 0), hit("二", 50, 0), hit("三", 100, 0)];
        assert!(roi_quality_ok(&ok, 3, 0.5));
    }

    #[test]
    fn need_full_when_no_cache_or_stale() {
        assert!(need_full_ocr(None, None, None, false, false));
        assert!(need_full_ocr(
            Some(Page::VenueDetail),
            Some(FULL_OCR_STALE_MS + 1),
            Some(0.8),
            false,
            true
        ));
        assert!(!need_full_ocr(
            Some(Page::VenueDetail),
            Some(500),
            Some(0.8),
            false,
            true
        ));
    }

    #[test]
    fn pixel_rect_clamps() {
        let b = PageRoiBand {
            x0: 0.0,
            y0: 0.1,
            x1: 1.0,
            y1: 0.4,
            scale: 1.5,
        };
        let (x0, y0, x1, y1) = b.pixel_rect(1000, 1000).unwrap();
        assert_eq!(x0, 0);
        assert_eq!(y0, 100);
        assert_eq!(x1, 1000);
        assert_eq!(y1, 400);
    }
}

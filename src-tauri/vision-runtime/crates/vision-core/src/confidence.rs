//! Layered perception confidence for GlanceReport / click gating (P4).

use vision_contracts::AnchorBox;

use crate::date_cards::DateStripCoverage;
use crate::shadow::DEFAULT_MIN_CONFIDENCE;
use crate::state::{Page, UiSnapshot};

/// Minimum click-safety score before inject (aligned with page min confidence).
pub const CLICK_SAFETY_MIN: f32 = DEFAULT_MIN_CONFIDENCE;

#[derive(Debug, Clone, PartialEq)]
pub struct LayeredConfidence {
    pub page: Option<f32>,
    pub date: Option<f32>,
    pub click_safety: Option<f32>,
}

/// Build layered scores from the current observation.
pub fn layered_confidence(
    snap: &UiSnapshot,
    coverage: Option<&DateStripCoverage>,
    date_ocr_quality: Option<f32>,
    plan_confidence: f32,
    anchor: Option<&AnchorBox>,
    verification_anchor: Option<&AnchorBox>,
    max_anchor_drift_px: i32,
) -> LayeredConfidence {
    let page = if snap.page == Page::Unknown {
        Some(0.0)
    } else {
        Some(snap.confidence.clamp(0.0, 1.0))
    };

    let date = if snap.page == Page::VenueDetail {
        let q = coverage.map(|c| c.quality).unwrap_or(0.0);
        let ocr_q = date_ocr_quality.unwrap_or(q);
        Some(q.min(ocr_q).clamp(0.0, 1.0))
    } else {
        None
    };

    let click_safety = Some(click_safety_score(
        plan_confidence,
        anchor,
        verification_anchor,
        max_anchor_drift_px,
    ));

    LayeredConfidence {
        page,
        date,
        click_safety,
    }
}

/// Anchor geometry + plan confidence → click safety in 0..1.
pub fn click_safety_score(
    plan_confidence: f32,
    anchor: Option<&AnchorBox>,
    verification_anchor: Option<&AnchorBox>,
    max_anchor_drift_px: i32,
) -> f32 {
    let conf = plan_confidence.clamp(0.0, 1.0);
    let Some(box_) = anchor else {
        // TypeText / non-box actions: confidence alone.
        return conf;
    };
    if box_.x1 <= box_.x0 || box_.y1 <= box_.y0 {
        return 0.0;
    }
    let geom = 1.0_f32;
    let drift_factor = match verification_anchor {
        None => 1.0,
        Some(obs) => {
            if obs.x1 <= obs.x0 || obs.y1 <= obs.y0 {
                return 0.0;
            }
            let drift = box_.center_drift_px(obs) as f32;
            let max = max_anchor_drift_px.max(1) as f32;
            if drift > max {
                0.0
            } else {
                (1.0 - drift / max).clamp(0.0, 1.0)
            }
        }
    };
    (conf * geom * drift_factor).clamp(0.0, 1.0)
}

pub fn click_safety_ok(score: f32) -> bool {
    score >= CLICK_SAFETY_MIN
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::UiSnapshot;

    fn snap(page: Page, conf: f32) -> UiSnapshot {
        UiSnapshot {
            page,
            confidence: conf,
            ..UiSnapshot::default()
        }
    }

    #[test]
    fn date_confidence_only_on_venue_detail() {
        let c = layered_confidence(&snap(Page::Home, 0.9), None, None, 0.9, None, None, 24);
        assert!(c.date.is_none());
        let cov = DateStripCoverage {
            quality: 0.7,
            ..DateStripCoverage::empty()
        };
        let c2 = layered_confidence(
            &snap(Page::VenueDetail, 0.85),
            Some(&cov),
            Some(0.6),
            0.8,
            None,
            None,
            24,
        );
        assert_eq!(c2.date, Some(0.6));
        assert_eq!(c2.page, Some(0.85));
    }

    #[test]
    fn low_plan_confidence_fails_safety() {
        let s = click_safety_score(0.3, None, None, 24);
        assert!(!click_safety_ok(s));
    }

    #[test]
    fn drifted_anchor_zeroes_safety() {
        let a = AnchorBox {
            x0: 10,
            y0: 10,
            x1: 50,
            y1: 40,
        };
        let v = AnchorBox {
            x0: 100,
            y0: 100,
            x1: 140,
            y1: 130,
        };
        let s = click_safety_score(0.95, Some(&a), Some(&v), 24);
        assert_eq!(s, 0.0);
        assert!(!click_safety_ok(s));
    }

    #[test]
    fn nearby_verification_keeps_safety() {
        let a = AnchorBox {
            x0: 10,
            y0: 10,
            x1: 50,
            y1: 40,
        };
        let v = AnchorBox {
            x0: 12,
            y0: 11,
            x1: 52,
            y1: 41,
        };
        let s = click_safety_score(0.9, Some(&a), Some(&v), 24);
        assert!(click_safety_ok(s));
    }
}

//! Multi-frame scene fusion for date evidence.
//!
//! Single frames produce [`FrameDateEvidence`]; only this module emits
//! [`FusedDateEvidence::ConfirmedAbsent`] (→ DATE_MISMATCH).

use std::collections::VecDeque;

use crate::date_cards::{
    date_cards_fingerprint, DateCard, DateStripCoverage, FrameDateEvidence,
};
use crate::ocr::OcrHit;
use crate::recovery::DateRecoveryBudget;
use crate::state::Page;

/// Keep last N date samples (Task 1.3).
pub const SCENE_DATE_HISTORY: usize = 5;
/// Independent stable frames required for ConfirmedAbsent / ConfirmedUnavailable.
pub const DATE_CONFIRM_STREAK: u32 = 3;

#[derive(Debug, Clone, PartialEq)]
pub enum FusedDateEvidence {
    ConfirmedBookable(DateCard),
    ConfirmedUnavailable(DateCard),
    ConfirmedAbsent,
    Insufficient { why: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DateFrameRecord {
    pub page: Page,
    pub frame_id: String,
    pub capture_monotonic_ms: u64,
    pub fingerprint: u64,
    pub evidence: FrameDateEvidence,
    pub coverage: DateStripCoverage,
    pub ocr_quality: f32,
}

/// Cached full-page OCR for region-priority observes (P4).
#[derive(Debug, Clone)]
pub struct OcrObserveCache {
    pub page: Page,
    pub hits: Vec<OcrHit>,
    pub full_monotonic_ms: u64,
    pub quality: f32,
}

#[derive(Debug, Clone, Default)]
pub struct SceneState {
    date_frames: VecDeque<DateFrameRecord>,
    /// Recovery ladder while fused evidence stays Insufficient.
    pub date_recovery: Option<DateRecoveryBudget>,
    /// Last full-page OCR cache for ROI-priority path.
    pub ocr_cache: Option<OcrObserveCache>,
    /// Next observe should upscale date-strip ROI OCR (recovery RoiZoom).
    pub pending_date_roi_zoom: bool,
}

impl SceneState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear_date(&mut self) {
        self.date_frames.clear();
        self.date_recovery = None;
        self.pending_date_roi_zoom = false;
    }

    pub fn clear_ocr_cache(&mut self) {
        self.ocr_cache = None;
    }

    pub fn remember_full_ocr(
        &mut self,
        page: Page,
        hits: Vec<OcrHit>,
        monotonic_ms: u64,
        quality: f32,
    ) {
        self.ocr_cache = Some(OcrObserveCache {
            page,
            hits,
            full_monotonic_ms: monotonic_ms,
            quality,
        });
    }

    pub fn date_len(&self) -> usize {
        self.date_frames.len()
    }

    /// Latest capture monotonic ms in the date history (for recovery budget).
    pub fn last_capture_ms(&self) -> Option<u64> {
        self.date_frames.back().map(|r| r.capture_monotonic_ms)
    }

    /// Push one independent capture. Duplicate frame_id / same capture time+fp are ignored.
    pub fn push_date_frame(&mut self, rec: DateFrameRecord) -> FusedDateEvidence {
        if let Some(prev) = self.date_frames.back() {
            if prev.frame_id == rec.frame_id
                || (prev.capture_monotonic_ms == rec.capture_monotonic_ms
                    && prev.fingerprint == rec.fingerprint)
            {
                return fuse_date_history(self.date_frames.make_contiguous());
            }
        }
        self.date_frames.push_back(rec);
        while self.date_frames.len() > SCENE_DATE_HISTORY {
            self.date_frames.pop_front();
        }
        fuse_date_history(self.date_frames.make_contiguous())
    }
}

fn is_stable_date_frame(rec: &DateFrameRecord) -> bool {
    rec.page == Page::VenueDetail
        && !rec.coverage.animating
        && rec.coverage.is_sufficient()
        && rec.ocr_quality >= 0.4
}

/// Pure fusion over recent independent frames.
pub fn fuse_date_history(frames: &[DateFrameRecord]) -> FusedDateEvidence {
    if frames.is_empty() {
        return FusedDateEvidence::Insufficient {
            why: "no_frames".into(),
        };
    }
    let last = frames.last().unwrap();
    match &last.evidence {
        FrameDateEvidence::Bookable(card) => {
            return FusedDateEvidence::ConfirmedBookable(card.clone());
        }
        FrameDateEvidence::Incomplete { why } => {
            return FusedDateEvidence::Insufficient {
                why: why.clone(),
            };
        }
        // Same-frame dual source (text + color) → confirm without streak.
        FrameDateEvidence::UnavailableCandidate(card)
            if card.sources.dual_status_confirmed() == Some(crate::date_cards::DateStatus::Empty) =>
        {
            return FusedDateEvidence::ConfirmedUnavailable(card.clone());
        }
        _ => {}
    }

    // ConfirmedUnavailable: last N stable UnavailableCandidate with same md+label.
    let unavail: Vec<&DateFrameRecord> = frames
        .iter()
        .rev()
        .take_while(|r| {
            is_stable_date_frame(r)
                && matches!(r.evidence, FrameDateEvidence::UnavailableCandidate(_))
        })
        .collect();
    if unavail.len() as u32 >= DATE_CONFIRM_STREAK {
        let first = match &unavail[0].evidence {
            FrameDateEvidence::UnavailableCandidate(c) => c.clone(),
            _ => unreachable!(),
        };
        let same = unavail.iter().all(|r| match &r.evidence {
            FrameDateEvidence::UnavailableCandidate(c) => {
                c.month == first.month && c.day == first.day && c.status_label == first.status_label
            }
            _ => false,
        });
        if same {
            return FusedDateEvidence::ConfirmedUnavailable(first);
        }
    }

    // ConfirmedAbsent: last N stable AbsentWithCoverage, coverage range agrees.
    let absents: Vec<&DateFrameRecord> = frames
        .iter()
        .rev()
        .take_while(|r| {
            is_stable_date_frame(r)
                && matches!(r.evidence, FrameDateEvidence::AbsentWithCoverage(_))
        })
        .collect();
    if absents.len() as u32 >= DATE_CONFIRM_STREAK {
        let ranges_agree = absents.windows(2).all(|w| {
            w[0].coverage.first == w[1].coverage.first
                && w[0].coverage.last == w[1].coverage.last
                && w[0].coverage.goal_in_visible_range
                && w[1].coverage.goal_in_visible_range
        });
        if ranges_agree
            && absents
                .iter()
                .all(|r| r.coverage.goal_in_visible_range && r.coverage.is_sufficient())
        {
            return FusedDateEvidence::ConfirmedAbsent;
        }
    }

    FusedDateEvidence::Insufficient {
        why: match &last.evidence {
            FrameDateEvidence::UnavailableCandidate(_) => "unavailable_awaiting_streak".into(),
            FrameDateEvidence::AbsentWithCoverage(_) => "absent_awaiting_streak".into(),
            FrameDateEvidence::Incomplete { why } => why.clone(),
            FrameDateEvidence::Bookable(_) => "bookable".into(),
        },
    }
}

/// Helper for tests / callers building a record from cards.
pub fn make_date_record(
    page: Page,
    frame_id: &str,
    capture_ms: u64,
    cards: &[DateCard],
    evidence: FrameDateEvidence,
    coverage: DateStripCoverage,
    ocr_quality: f32,
) -> DateFrameRecord {
    DateFrameRecord {
        page,
        frame_id: frame_id.into(),
        capture_monotonic_ms: capture_ms,
        fingerprint: date_cards_fingerprint(cards),
        evidence,
        coverage,
        ocr_quality,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidates::BBox;
    use crate::date_cards::{DateStatus, DateStripCoverage};

    fn card(day: u32, status: DateStatus, label: &str) -> DateCard {
        let status_text = match status {
            DateStatus::Unknown => None,
            s => Some(s),
        };
        DateCard {
            weekday: "周二".into(),
            date_label: format!("09月{day:02}日"),
            month: 9,
            day,
            status,
            click_bbox: BBox {
                x0: 100,
                y0: 200,
                x1: 140,
                y1: 240,
            },
            status_label: label.into(),
            status_color: None,
            sources: crate::date_cards::DateEvidenceSources {
                structure_ok: true,
                md_ok: true,
                status_text,
                status_color: None,
            },
        }
    }

    fn card_dual(day: u32, status: DateStatus, label: &str) -> DateCard {
        let mut c = card(day, status, label);
        c.status_color = Some(status);
        c.sources.status_color = Some(status);
        c
    }

    fn cov_in_range() -> DateStripCoverage {
        DateStripCoverage {
            roi: Some(BBox {
                x0: 50,
                y0: 100,
                x1: 400,
                y1: 280,
            }),
            visible_cols: 3,
            parsed_dates: vec![(9, 21), (9, 22), (9, 23)],
            ordered: true,
            continuous: true,
            first: Some((9, 21)),
            last: Some((9, 23)),
            animating: false,
            occluded: false,
            quality: 0.9,
            goal_in_visible_range: true,
        }
    }

    #[test]
    fn incomplete_stays_insufficient() {
        let mut scene = SceneState::new();
        let fused = scene.push_date_frame(make_date_record(
            Page::VenueDetail,
            "f1",
            10,
            &[],
            FrameDateEvidence::Incomplete {
                why: "no_date_cards".into(),
            },
            DateStripCoverage::empty(),
            0.9,
        ));
        assert!(matches!(fused, FusedDateEvidence::Insufficient { .. }));
    }

    #[test]
    fn duplicate_frame_id_does_not_count() {
        let mut scene = SceneState::new();
        let cov = cov_in_range();
        let c = card(21, DateStatus::Empty, "暂无");
        let rec = make_date_record(
            Page::VenueDetail,
            "same",
            10,
            &[c.clone()],
            FrameDateEvidence::UnavailableCandidate(c),
            cov,
            0.9,
        );
        let _ = scene.push_date_frame(rec.clone());
        let mut r2 = rec.clone();
        r2.capture_monotonic_ms = 11;
        let _ = scene.push_date_frame(r2);
        assert_eq!(scene.date_len(), 1);
    }

    #[test]
    fn three_absent_with_coverage_confirms() {
        let mut scene = SceneState::new();
        let cov = cov_in_range();
        for i in 0..3 {
            let fused = scene.push_date_frame(make_date_record(
                Page::VenueDetail,
                &format!("f{i}"),
                100 + i as u64,
                &[],
                FrameDateEvidence::AbsentWithCoverage(cov.clone()),
                cov.clone(),
                0.9,
            ));
            if i < 2 {
                assert!(
                    matches!(fused, FusedDateEvidence::Insufficient { .. }),
                    "i={i}"
                );
            } else {
                assert_eq!(fused, FusedDateEvidence::ConfirmedAbsent);
            }
        }
    }

    #[test]
    fn offscreen_incomplete_never_confirms_absent() {
        let mut scene = SceneState::new();
        for i in 0..3 {
            let fused = scene.push_date_frame(make_date_record(
                Page::VenueDetail,
                &format!("f{i}"),
                100 + i as u64,
                &[],
                FrameDateEvidence::Incomplete {
                    why: "goal_offscreen_or_range_unknown".into(),
                },
                DateStripCoverage::empty(),
                0.9,
            ));
            assert!(matches!(fused, FusedDateEvidence::Insufficient { .. }));
        }
    }

    #[test]
    fn three_unavailable_confirms() {
        let mut scene = SceneState::new();
        let cov = cov_in_range();
        let c = card(21, DateStatus::Empty, "暂无");
        for i in 0..3 {
            let fused = scene.push_date_frame(make_date_record(
                Page::VenueDetail,
                &format!("u{i}"),
                200 + i as u64,
                &[c.clone()],
                FrameDateEvidence::UnavailableCandidate(c.clone()),
                cov.clone(),
                0.9,
            ));
            if i < 2 {
                assert!(matches!(fused, FusedDateEvidence::Insufficient { .. }));
            } else {
                assert!(matches!(
                    fused,
                    FusedDateEvidence::ConfirmedUnavailable(_)
                ));
            }
        }
    }

    #[test]
    fn bookable_confirms_immediately() {
        let mut scene = SceneState::new();
        let cov = cov_in_range();
        let c = card(21, DateStatus::Bookable, "预定");
        let fused = scene.push_date_frame(make_date_record(
            Page::VenueDetail,
            "b1",
            1,
            &[c.clone()],
            FrameDateEvidence::Bookable(c),
            cov,
            0.9,
        ));
        assert!(matches!(fused, FusedDateEvidence::ConfirmedBookable(_)));
    }

    #[test]
    fn dual_source_unavailable_confirms_without_streak() {
        let mut scene = SceneState::new();
        let cov = cov_in_range();
        let c = card_dual(21, DateStatus::Empty, "暂无");
        let fused = scene.push_date_frame(make_date_record(
            Page::VenueDetail,
            "d1",
            1,
            &[c.clone()],
            FrameDateEvidence::UnavailableCandidate(c),
            cov,
            0.9,
        ));
        assert!(matches!(
            fused,
            FusedDateEvidence::ConfirmedUnavailable(_)
        ));
    }

    #[test]
    fn single_source_unavailable_awaits_streak() {
        let mut scene = SceneState::new();
        let cov = cov_in_range();
        let c = card(21, DateStatus::Empty, "暂无");
        let fused = scene.push_date_frame(make_date_record(
            Page::VenueDetail,
            "s1",
            1,
            &[c.clone()],
            FrameDateEvidence::UnavailableCandidate(c),
            cov,
            0.9,
        ));
        assert!(matches!(
            fused,
            FusedDateEvidence::Insufficient { why } if why == "unavailable_awaiting_streak"
        ));
    }
}

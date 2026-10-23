//! Venue-detail date evidence fusion + book CTA policy.

use vision_contracts::AnchorBox;

use crate::candidates::CandidateSet;
use crate::state::{Page, UiSnapshot};

use super::super::{finish, stop, FlowDomain, NextIntent, ObservationDecision, ObserveFrameMeta, TaskGoal};

pub(crate) fn venue_detail(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    candidates: &CandidateSet,
    mut scene: Option<&mut crate::scene::SceneState>,
    frame: Option<&ObserveFrameMeta>,
) -> ObservationDecision {
    let Some(venue) = goal.venue() else {
        return stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "GOAL_UNSPECIFIED",
            "暂停，没有目标场馆",
        );
    };
    if candidates.find_click(venue).is_none() && !candidates.has_label(venue) {
        return stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "VENUE_MISMATCH",
            "暂停，画面场馆不是目标",
        );
    }
    let Some(date) = goal.date() else {
        return stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "GOAL_UNSPECIFIED",
            "暂停，没有目标日期",
        );
    };
    let frame_w = frame.map(|f| f.frame_w).unwrap_or_else(|| {
        candidates
            .items
            .iter()
            .map(|c| c.bbox.x1)
            .max()
            .unwrap_or(900)
            .max(200)
    });
    let frame_h = frame.map(|f| f.frame_h).unwrap_or(900);
    let mut cards = crate::date_cards::locate_date_cards_ex(candidates, frame_w);
    crate::date_cards::enrich_date_cards_color(
        &mut cards,
        frame.and_then(|f| f.pixels_bgra),
        frame_w.max(0) as u32,
        frame_h.max(0) as u32,
    );
    let coverage =
        crate::date_cards::build_date_strip_coverage(&cards, date, frame_w, frame_h);
    let frame_ev = crate::date_cards::assess_frame_date(&cards, date, &coverage);

    let fused = match scene.as_mut() {
        Some(scene) => {
            if let Some(meta) = frame {
                let rec = crate::scene::make_date_record(
                    snap.page,
                    &meta.frame_id,
                    meta.capture_monotonic_ms,
                    &cards,
                    frame_ev.clone(),
                    coverage.clone(),
                    snap.confidence.max(0.0),
                );
                scene.push_date_frame(rec)
            } else {
                crate::scene::fuse_date_history(&[crate::scene::make_date_record(
                    snap.page,
                    "anon",
                    0,
                    &cards,
                    frame_ev.clone(),
                    coverage.clone(),
                    snap.confidence.max(0.0),
                )])
            }
        }
        None => match &frame_ev {
            // Stateless path (UI glance / tests without scene): Bookable acts;
            // never single-frame DATE_MISMATCH / DATE_UNAVAILABLE.
            crate::date_cards::FrameDateEvidence::Bookable(c) => {
                crate::scene::FusedDateEvidence::ConfirmedBookable(c.clone())
            }
            other => crate::scene::FusedDateEvidence::Insufficient {
                why: match other {
                    crate::date_cards::FrameDateEvidence::Incomplete { why } => why.clone(),
                    crate::date_cards::FrameDateEvidence::UnavailableCandidate(_) => {
                        "unavailable_need_multiframe".into()
                    }
                    crate::date_cards::FrameDateEvidence::AbsentWithCoverage(_) => {
                        "absent_need_multiframe".into()
                    }
                    crate::date_cards::FrameDateEvidence::Bookable(_) => unreachable!(),
                },
            },
        },
    };

    match fused {
        crate::scene::FusedDateEvidence::ConfirmedBookable(card) => {
            if let Some(s) = scene.as_mut() {
                s.date_recovery = None;
            }
            let anchor = AnchorBox {
                x0: card.click_bbox.x0,
                y0: card.click_bbox.y0,
                x1: card.click_bbox.x1,
                y1: card.click_bbox.y1,
            };
            finish(
                FlowDomain::XmuBooking,
                snap.page,
                snap,
                goal,
                "现在：场馆详情",
                "下一步：选择目标日期",
                NextIntent::Navigate {
                    label: "预定".into(),
                    anchor: anchor.clone(),
                    expected_page: Page::SelectSlot,
                },
                vec![anchor],
                "select_slot",
                None,
            )
        }
        crate::scene::FusedDateEvidence::ConfirmedUnavailable(_) => {
            if let Some(s) = scene.as_mut() {
                s.date_recovery = None;
            }
            stop(
                snap,
                goal,
                FlowDomain::XmuBooking,
                "DATE_UNAVAILABLE",
                "暂停，目标日暂无",
            )
        }
        crate::scene::FusedDateEvidence::ConfirmedAbsent => {
            if let Some(s) = scene.as_mut() {
                s.date_recovery = None;
            }
            stop(
                snap,
                goal,
                FlowDomain::XmuBooking,
                "DATE_MISMATCH",
                "暂停，画面上没有目标日期",
            )
        }
        crate::scene::FusedDateEvidence::Insufficient { why } => {
            date_evidence_insufficient(snap, goal, scene, &why, frame)
        }
    }
}

pub(crate) fn date_evidence_insufficient(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    mut scene: Option<&mut crate::scene::SceneState>,
    why: &str,
    frame: Option<&ObserveFrameMeta>,
) -> ObservationDecision {
    use crate::recovery::{next_date_recovery, DateRecoveryAction, DateRecoveryBudget};

    let now_ms = frame
        .map(|f| f.capture_monotonic_ms)
        .or_else(|| scene.as_ref().and_then(|s| s.last_capture_ms()))
        .unwrap_or(0);

    let action = if let Some(s) = scene.as_deref_mut() {
        if s.date_recovery.is_none() {
            s.date_recovery = Some(DateRecoveryBudget::for_live_run(now_ms));
        }
        let budget = s.date_recovery.as_mut().unwrap();
        next_date_recovery(budget, now_ms)
    } else {
        DateRecoveryAction::Wait {
            stage: crate::recovery::DateRecoveryStage::WaitStabilize,
            approx_ms: crate::date_cards::DATE_EVIDENCE_WAIT_MS,
            reason: "DATE_EVIDENCE_WEAK",
        }
    };

    if matches!(
        &action,
        DateRecoveryAction::Wait {
            stage: crate::recovery::DateRecoveryStage::RoiZoomOcr,
            ..
        }
    ) {
        if let Some(s) = scene.as_deref_mut() {
            s.pending_date_roi_zoom = true;
        }
    }

    match action {
        DateRecoveryAction::Wait {
            approx_ms,
            reason,
            stage,
        } => finish(
            FlowDomain::XmuBooking,
            snap.page,
            snap,
            goal,
            "现在：场馆详情",
            &format!("下一步：等待日期证据（{why}/{})", stage.as_str()),
            NextIntent::Wait {
                reason: reason.into(),
                approx_ms,
            },
            Vec::new(),
            "venue_detail",
            Some(reason.into()),
        ),
        DateRecoveryAction::Rebind { reason } => finish(
            FlowDomain::XmuBooking,
            snap.page,
            snap,
            goal,
            "现在：场馆详情",
            "下一步：重新绑定窗口以刷新日期条",
            NextIntent::Rebind {
                reason: reason.into(),
            },
            Vec::new(),
            "venue_detail",
            Some(reason.into()),
        ),
        DateRecoveryAction::Timeout => stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "DATE_EVIDENCE_TIMEOUT",
            "暂停，日期证据不足且已超时",
        ),
    }
}


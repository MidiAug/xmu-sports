//! Select-slot page next-step policy.

use crate::candidates::{Candidate, CandidateKind, CandidateSet};
use crate::state::{Page, UiSnapshot};

use super::super::{navigate, stop, FlowDomain, ObservationDecision, TaskGoal};

pub(crate) fn select_slot(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    candidates: &CandidateSet,
) -> ObservationDecision {
    // Bottom CTA armed (同授权登录灰→蓝)：格已选中，下一步点「确认预约」，不要再点格。
    if crate::state::can_confirm_slot(snap) {
        if let Some(hit) = find_confirm_slot_cta(candidates) {
            return navigate(
                snap,
                goal,
                FlowDomain::XmuBooking,
                "现在：场次已选",
                "下一步：点确认预约",
                "确认预约",
                hit,
                Page::ConfirmOrder,
                "confirm_order",
            );
        }
        return stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "CONFIRM_CTA_NOT_LOCALIZED",
            "暂停，底部确认预约未定位",
        );
    }

    let cells: Vec<&Candidate> = candidates
        .items
        .iter()
        .filter(|c| c.kind == CandidateKind::SlotCell)
        .collect();
    let available: Vec<&Candidate> = cells
        .iter()
        .copied()
        .filter(|c| {
            c.actionable
                && c.slot_meta
                    .as_ref()
                    .is_some_and(|m| m.state == crate::slots::SlotState::Available)
        })
        .collect();

    if cells.is_empty() {
        return stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "SLOT_GRID_NOT_LOCALIZED",
            "暂停，无法确定可约格",
        );
    }
    if available.is_empty() {
        return stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "SLOT_ALL_GREY",
            "暂停，选择场次全灰/不可约",
        );
    }

    let mut pool = available;
    if let Some(t) = goal.slot_time() {
        let needle = crate::slots::parse_time_label(t).unwrap_or_else(|| t.to_string());
        let visible_times = visible_slot_times(&cells);
        let sequence = if goal.allow_time_fallback {
            crate::slots::nearest_time_order(
                &visible_times,
                &needle,
                goal.time_fallback_order(),
            )
        } else {
            vec![needle.clone()]
        };
        let mut matched: Vec<&Candidate> = Vec::new();
        for cand_time in &sequence {
            matched = pool
                .iter()
                .copied()
                .filter(|c| {
                    c.slot_meta
                        .as_ref()
                        .is_some_and(|m| m.start_time == *cand_time)
                })
                .collect();
            if !matched.is_empty() {
                break;
            }
        }
        if matched.is_empty() {
            return stop(
                snap,
                goal,
                FlowDomain::XmuBooking,
                "SLOT_TIME_MISMATCH",
                if goal.allow_time_fallback {
                    "暂停，目标与邻近时段都没有可约格"
                } else {
                    "暂停，没有指定时段的可约格"
                },
            );
        }
        pool = matched;
    }
    if let Some(court) = goal.prefer_court() {
        let want = normalize_court(court);
        let preferred: Vec<&Candidate> = pool
            .iter()
            .copied()
            .filter(|c| {
                c.slot_meta.as_ref().is_some_and(|m| {
                    let by_label = m
                        .court_label
                        .as_deref()
                        .is_some_and(|l| normalize_court(l) == want);
                    let by_index = m
                        .court_index
                        .is_some_and(|i| format!("{i:02}") == want || format!("{i}") == want);
                    by_label || by_index
                })
            })
            .collect();
        // Prefer matching court when present; otherwise keep the time-row pool (同 Python).
        if !preferred.is_empty() {
            pool = preferred;
        }
    }

    pool.sort_by_key(|c| (c.bbox.cy(), c.bbox.cx()));
    let ordered = crate::slots::apply_court_offset(pool, goal.court_offset);
    let cell = ordered[0];
    let label = cell
        .slot_meta
        .as_ref()
        .map(|m| m.start_time.clone())
        .unwrap_or_else(|| "slot".into());
    // 选格不换页：底部「请先选择场次」→「确认预约」。expected=同页 → SamePageNextControl。
    navigate(
        snap,
        goal,
        FlowDomain::XmuBooking,
        "现在：场次页",
        "下一步：点可约格",
        &label,
        cell,
        Page::SelectSlot,
        "select_slot_armed",
    )
}

pub(crate) fn visible_slot_times(cells: &[&Candidate]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in cells {
        let Some(t) = c.slot_meta.as_ref().map(|m| m.start_time.as_str()) else {
            continue;
        };
        if t.is_empty() {
            continue;
        }
        let norm = crate::slots::parse_time_label(t).unwrap_or_else(|| t.to_string());
        if !out.contains(&norm) {
            out.push(norm);
        }
    }
    out.sort_by_key(|t| time_sort_key(t));
    out
}

pub(crate) fn time_sort_key(t: &str) -> i32 {
    let parts: Vec<_> = t.split(':').collect();
    if parts.len() != 2 {
        return 0;
    }
    let h: i32 = parts[0].parse().unwrap_or(0);
    let m: i32 = parts[1].parse().unwrap_or(0);
    h * 60 + m
}

/// Bottom 「确认预约」— prefer lowest on screen (skip top chrome if OCR doubles it).
pub(crate) fn find_confirm_slot_cta(candidates: &CandidateSet) -> Option<&Candidate> {
    candidates
        .items
        .iter()
        .filter(|c| {
            c.actionable
                && c.kind != CandidateKind::SlotCell
                && crate::ocr::labels_match_exact(&c.label, "确认预约")
        })
        .max_by_key(|c| c.bbox.y0)
}

pub(crate) fn normalize_court(s: &str) -> String {
    let t = s.trim();
    if let Ok(n) = t.parse::<u32>() {
        return format!("{n:02}");
    }
    t.to_string()
}


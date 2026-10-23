//! Shared label / sport / goal navigation helpers for page policies.

use crate::candidates::{Candidate, CandidateSet};
use crate::state::{Page, UiSnapshot};

use super::super::{navigate, stop, FlowDomain, ObservationDecision, TaskGoal};

pub(crate) fn notice_ack<'a>(candidates: &'a CandidateSet) -> Option<&'a Candidate> {
    candidates
        .items
        .iter()
        .find(|c| c.actionable && c.element_id.as_deref() == Some(crate::candidates::ELEMENT_NOTICE_ACK))
}

pub(crate) fn nav_sport_icon(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    candidates: &CandidateSet,
) -> ObservationDecision {
    let Some(sport) = goal.sport() else {
        return stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "GOAL_UNSPECIFIED",
            "暂停，没有任务目标",
        );
    };
    match candidates.find_sport_tile(sport) {
        Some(hit) => navigate(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "现在：运动列表",
            &format!("下一步：点{sport}"),
            sport,
            hit,
            Page::VenueList,
            "venue_list",
        ),
        None => stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "SPORT_NOT_ON_SCREEN",
            "暂停，画面上没有这个目标",
        ),
    }
}

pub(crate) fn nav_goal_label(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    candidates: &CandidateSet,
    label: Option<&str>,
    missing_reason: &str,
    mismatch_reason: &str,
    now: &str,
    next: &str,
    expected: Page,
    transition: &str,
) -> ObservationDecision {
    let Some(label) = label else {
        return stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            missing_reason,
            "暂停，没有任务目标",
        );
    };
    match candidates.find_click(label) {
        Some(hit) => navigate(
            snap,
            goal,
            FlowDomain::XmuBooking,
            now,
            next,
            label,
            hit,
            expected,
            transition,
        ),
        None => stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            mismatch_reason,
            "暂停，画面上没有这个目标",
        ),
    }
}

pub(crate) fn nav_text(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    candidates: &CandidateSet,
    domain: FlowDomain,
    now: &str,
    needle: &str,
    next: &str,
    expected: Page,
    transition: &str,
) -> ObservationDecision {
    match candidates.find_click(needle) {
        Some(hit) => navigate(
            snap, goal, domain, now, next, needle, hit, expected, transition,
        ),
        None => stop(
            snap,
            goal,
            domain,
            "ANCHOR_NOT_FOUND",
            "暂停，找不到要点的入口",
        ),
    }
}


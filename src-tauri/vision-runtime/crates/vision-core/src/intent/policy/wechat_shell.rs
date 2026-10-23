//! WeChat shell / mini-program panel next-step policy.

use crate::candidates::{Candidate, CandidateKind, CandidateSet, ShellVisualState};
use crate::state::{Page, UiSnapshot};

use super::super::{
    box_of, finish, navigate, stop, AtomicStep, FlowDomain, NextIntent, ObservationDecision,
    TaskGoal,
};

pub(crate) fn mp_panel(snap: &UiSnapshot, goal: &TaskGoal, candidates: &CandidateSet) -> ObservationDecision {
    // Prefer the crest tile over an OCR caption that also appears inside the logo.
    if let Some(hit) = find_xmu_tile(candidates) {
        return navigate(
            snap,
            goal,
            FlowDomain::WechatShell,
            "现在：小程序面板",
            "下一步：打开厦大体育",
            "厦大体育",
            hit,
            Page::Home,
            "home",
        );
    }
    if snap.page == Page::WechatMpSearch {
        if let Some(hit) = find_xmu_search_result(candidates) {
            return navigate(
                snap,
                goal,
                FlowDomain::WechatShell,
                "现在：小程序搜索",
                "下一步：打开厦大体育",
                "厦大体育",
                hit,
                Page::Home,
                "home",
            );
        }
        return finish(
            FlowDomain::WechatShell,
            Page::WechatMpSearch,
            snap,
            goal,
            "现在：小程序搜索",
            "下一步：暂停，搜索结果里没有厦大体育",
            NextIntent::Stop {
                reason: "ANCHOR_NOT_FOUND".into(),
            },
            Vec::new(),
            "wechat_mp_search",
            Some("ANCHOR_NOT_FOUND".into()),
        );
    }
    // Discover's 「搜一搜」must never be treated as the panel search button.
    // Magnifier → type → result must stay in one authorization: the search
    // layer collapses as soon as WeChat loses focus.
    if let Some(search) = find_mp_search(candidates) {
        let search_anchor = box_of(search);
        return finish(
            FlowDomain::WechatShell,
            Page::WechatMpPanel,
            snap,
            goal,
            "现在：小程序面板",
            "下一步：搜索并打开厦大体育",
            NextIntent::AtomicComposite {
                label: "搜索厦大体育".into(),
                steps: vec![
                    AtomicStep::Click {
                        label: "搜索".into(),
                        anchor: search_anchor.clone(),
                    },
                    AtomicStep::TypeText {
                        text: "厦大体育".into(),
                    },
                ],
                expected_page: Page::Home,
            },
            vec![search_anchor],
            "home",
            None,
        );
    }
    finish(
        FlowDomain::WechatShell,
        Page::WechatMpPanel,
        snap,
        goal,
        "现在：小程序面板",
        "下一步：暂停，找不到小程序搜索入口",
        NextIntent::Stop {
            reason: "ANCHOR_NOT_FOUND".into(),
        },
        Vec::new(),
        "wechat_mp_panel",
        Some("ANCHOR_NOT_FOUND".into()),
    )
}

pub(crate) fn find_xmu_tile(candidates: &CandidateSet) -> Option<&Candidate> {
    candidates
        .items
        .iter()
        .filter(|c| {
            c.actionable
                && (c.element_id.as_deref() == Some(crate::candidates::ELEMENT_XMU_SPORTS)
                    || c.template_id
                        .as_deref()
                        .is_some_and(|id| id.starts_with("xmu_sports_icon")))
        })
        .max_by(|a, b| {
            b.bbox.y0.cmp(&a.bbox.y0).then_with(|| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        })
        .or_else(|| candidates.find_click("厦大体育"))
}

pub(crate) fn find_xmu_search_result(candidates: &CandidateSet) -> Option<&Candidate> {
    find_xmu_tile(candidates).or_else(|| {
        candidates
            .items
            .iter()
            .filter(|c| {
                c.actionable
                    && c.kind != CandidateKind::SlotCell
                    && crate::ocr::labels_match_exact(&c.label, "厦大体育")
                    && !c.label.contains("小程序")
                    && !c.label.contains("场馆")
            })
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    })
}

pub(crate) fn find_mp_search(candidates: &CandidateSet) -> Option<&Candidate> {
    candidates
        .items
        .iter()
        .filter(|c| c.actionable && is_mp_search_button(c))
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

pub(crate) fn is_mp_search_button(c: &Candidate) -> bool {
    if c.element_id.as_deref() == Some(crate::candidates::ELEMENT_MP_SEARCH)
        || c.template_id
            .as_deref()
            .is_some_and(|id| id.starts_with("mp_search"))
    {
        return true;
    }
    let label = c.label.trim();
    if label == "搜一搜" || label.contains("搜一搜") {
        return false;
    }
    label == "搜索" || c.shell_alias() == Some("搜索")
}

pub(crate) fn wechat_shell_or_unknown(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    candidates: &CandidateSet,
) -> ObservationDecision {
    if snap.page != Page::Unknown && snap.page != Page::WechatMain {
        return stop(
            snap,
            goal,
            FlowDomain::Unknown,
            "UNKNOWN_PAGE",
            "暂停，认不出当前页",
        );
    }
    // The shell is a two-level state machine, not a word search.  The
    // compass's red dot is a notification overlay and is intentionally not
    // evidence here: filled vs outlined is the state signal.
    match candidates.shell_state("发现") {
        Some(ShellVisualState::Inactive) => {
            if let Some(hit) = candidates.find_click("发现") {
                return navigate(
                    snap,
                    goal,
                    FlowDomain::WechatShell,
                    "现在：微信，发现未打开",
                    "下一步：打开发现",
                    "发现",
                    hit,
                    Page::WechatDiscover,
                    "wechat_discover",
                );
            }
        }
        Some(ShellVisualState::Active) => return wechat_discover(snap, goal, candidates),
        None => {}
    }
    if candidates.has_label("厦大体育") && candidates.find_click("小程序").is_some() {
        return mp_panel(snap, goal, candidates);
    }
    if let Some(hit) = candidates.find_click("小程序") {
        return navigate(
            snap,
            goal,
            FlowDomain::WechatShell,
            "现在：微信",
            "下一步：打开小程序",
            "小程序",
            hit,
            Page::WechatMpPanel,
            "wechat_mp_panel",
        );
    }
    if let Some(hit) = candidates.find_click("发现") {
        return navigate(
            snap,
            goal,
            FlowDomain::WechatShell,
            "现在：微信",
            "下一步：打开发现",
            "发现",
            hit,
            Page::WechatDiscover,
            "wechat_discover",
        );
    }
    let now = if snap.page == Page::WechatMain {
        "现在：微信"
    } else {
        "现在：不确定"
    };
    finish(
        if snap.page == Page::WechatMain {
            FlowDomain::WechatShell
        } else {
            FlowDomain::Unknown
        },
        snap.page,
        snap,
        goal,
        now,
        "下一步：暂停，找不到发现入口",
        NextIntent::Stop {
            reason: "ANCHOR_NOT_FOUND".into(),
        },
        Vec::new(),
        snap.page.as_str(),
        Some("ANCHOR_NOT_FOUND".into()),
    )
}

pub(crate) fn wechat_discover(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    candidates: &CandidateSet,
) -> ObservationDecision {
    match candidates.shell_state("小程序") {
        Some(ShellVisualState::Active) => mp_panel(snap, goal, candidates),
        Some(ShellVisualState::Inactive) => {
            if let Some(hit) = candidates.find_click("小程序") {
                navigate(
                    snap,
                    goal,
                    FlowDomain::WechatShell,
                    "现在：微信，发现已打开",
                    "下一步：打开小程序",
                    "小程序",
                    hit,
                    Page::WechatMpPanel,
                    "wechat_mp_panel",
                )
            } else {
                stop(
                    snap,
                    goal,
                    FlowDomain::WechatShell,
                    "ANCHOR_NOT_FOUND",
                    "暂停，找不到小程序入口",
                )
            }
        }
        None => stop(
            snap,
            goal,
            FlowDomain::WechatShell,
            "ANCHOR_NOT_FOUND",
            "暂停，找不到小程序入口",
        ),
    }
}


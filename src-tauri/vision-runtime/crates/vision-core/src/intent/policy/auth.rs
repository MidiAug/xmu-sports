//! XMU WeChat authorize gate policy.

use crate::candidates::{Candidate, CandidateKind, CandidateSet};
use crate::state::{Page, UiSnapshot};

use super::super::{navigate, stop, FlowDomain, ObservationDecision, TaskGoal};

pub(crate) fn xmu_wx_authorize(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    candidates: &CandidateSet,
) -> ObservationDecision {
    // The button stays gray until the agree row is checked, then the fill turns dark blue.
    if let Some(hit) = enabled_auth_login(candidates) {
        return navigate(
            snap,
            goal,
            FlowDomain::XmuAuth,
            "现在：微信授权",
            "下一步：授权登录",
            "授权登录",
            hit,
            Page::Home,
            "home",
        );
    }
    if let Some(hit) = find_agree_checkbox(candidates) {
        return navigate(
            snap,
            goal,
            FlowDomain::XmuAuth,
            "现在：微信授权",
            "下一步：勾选协议",
            "阅读并同意",
            &hit,
            Page::XmuWxAuthorize,
            "xmu_wx_authorize",
        );
    }
    if let Some(hit) = candidates.find_click("授权登录") {
        return navigate(
            snap,
            goal,
            FlowDomain::XmuAuth,
            "现在：微信授权",
            "下一步：授权登录",
            "授权登录",
            hit,
            Page::Home,
            "home",
        );
    }
    stop(
        snap,
        goal,
        FlowDomain::XmuAuth,
        "CHECKBOX_NOT_LOCALIZED",
        "暂停，协议勾选框没有定位",
    )
}

pub(crate) fn enabled_auth_login(candidates: &CandidateSet) -> Option<&Candidate> {
    candidates.items.iter().find(|c| {
        c.actionable
            && c.element_id.as_deref() == Some(crate::candidates::ELEMENT_AUTH_LOGIN)
            && c.evidence.iter().any(|e| e == "auth_login_enabled")
    })
}

/// Click left of 「阅读并同意…」 row: radio + gray caption.
/// Full-line OCR center lands on blue 《用户协议》 and opens the doc (device 2026-09-23).
pub(crate) fn find_agree_checkbox(candidates: &CandidateSet) -> Option<Candidate> {
    let hit = candidates.items.iter().find(|c| {
        c.actionable
            && (c.label.contains("阅读并同意")
                || c.label.contains("同意") && c.label.contains("协议"))
    })?;
    let src = hit.click_bbox();
    let w = (src.x1 - src.x0).max(1);
    // Left ~38% of the caption strip → center stays on gray text / radio, not 《…》 links.
    let left_w = ((w as f32) * 0.38).round() as i32;
    let left_w = left_w.clamp(1, w);
    let b = crate::candidates::BBox {
        x0: src.x0,
        y0: src.y0,
        x1: src.x0 + left_w,
        y1: src.y1,
    };
    Some(Candidate {
        id: "agree_caption".into(),
        bbox: b.clone(),
        kind: CandidateKind::Button,
        label: "阅读并同意".into(),
        score: hit.score,
        sources: hit.sources.clone(),
        actionable: true,
        template_id: None,
        slot_meta: None,
        evidence: vec!["agree_click_left_of_links".into()],
        element_id: Some("xmu.agree_checkbox".into()),
        element_state: None,
        theme: None,
        visual_bbox: Some(b.clone()),
        hit_bbox: Some(b),
    })
}


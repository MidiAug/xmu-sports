//! One next step from the current frame. Suggestion only — never a GuardedAction.
//!
//! Page policies live in [`policy`]; [`decide_next`] / [`decide_next_with_scene`]
//! remain the only dispatchers.

use serde::{Deserialize, Serialize};
use vision_contracts::AnchorBox;

use crate::candidates::{Candidate, CandidateSet};
use crate::state::{Overlay, Page, UiSnapshot};

/// Host re-glances this often while 「请求中」/ auth waiting — not a fixed 5s sleep.
/// Aligned with soft-poll cadence; see Playwright/Airtest condition waits.
pub const AUTH_WAIT_POLL_MS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowDomain {
    WechatShell,
    XmuAuth,
    XmuBooking,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskGoal {
    pub sport: Option<String>,
    pub venue: Option<String>,
    pub date: Option<String>,
    pub slot_time: Option<String>,
    /// Multi-court venues only (e.g. 「01」). Empty = leftmost available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer_court: Option<String>,
    /// Skip this many L→R available courts before clicking (0=leftmost). Default 2.
    #[serde(default = "default_court_offset")]
    pub court_offset: u32,
    /// When target time has no available cell, try neighboring rows.
    #[serde(default)]
    pub allow_time_fallback: bool,
    /// `up_then_down` (先更早) | `down_then_up` (先更晚).
    #[serde(default = "default_time_fallback_order")]
    pub time_fallback_order: String,
    /// Filled by glance after captcha-tile OCR. Not a user task field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognized_captcha: Option<String>,
}

fn default_time_fallback_order() -> String {
    "down_then_up".into()
}

fn default_court_offset() -> u32 {
    2
}

impl Default for TaskGoal {
    fn default() -> Self {
        Self {
            sport: None,
            venue: None,
            date: None,
            slot_time: None,
            prefer_court: None,
            court_offset: default_court_offset(),
            allow_time_fallback: false,
            time_fallback_order: default_time_fallback_order(),
            recognized_captcha: None,
        }
    }
}

impl TaskGoal {
    fn sport(&self) -> Option<&str> {
        filled(self.sport.as_deref())
    }
    fn venue(&self) -> Option<&str> {
        filled(self.venue.as_deref())
    }
    fn date(&self) -> Option<&str> {
        filled(self.date.as_deref())
    }
    fn slot_time(&self) -> Option<&str> {
        filled(self.slot_time.as_deref())
    }
    fn prefer_court(&self) -> Option<&str> {
        filled(self.prefer_court.as_deref())
    }
    fn recognized_captcha(&self) -> Option<&str> {
        filled(self.recognized_captcha.as_deref())
    }
    fn time_fallback_order(&self) -> &str {
        let o = self.time_fallback_order.trim();
        if o == "up_then_down" {
            "up_then_down"
        } else {
            "down_then_up"
        }
    }
}

fn filled(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|t| !t.is_empty())
}

/// Suggested next step. Not convertible to input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NextIntent {
    Navigate {
        label: String,
        anchor: AnchorBox,
        expected_page: Page,
    },
    /// Click → type → click again without giving the WeChat search layer focus back.
    /// Used when the mini-program search box collapses on deactivate.
    AtomicComposite {
        label: String,
        steps: Vec<AtomicStep>,
        expected_page: Page,
    },
    Wait {
        reason: String,
        approx_ms: u64,
    },
    Stop {
        reason: String,
    },
    NeedHuman {
        reason: String,
    },
    /// Capture target is the wrong HWND. Not a click.
    Rebind {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AtomicStep {
    Click { label: String, anchor: AnchorBox },
    TypeText { text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationDecision {
    pub domain: FlowDomain,
    pub page: Page,
    pub overlay: Overlay,
    pub goal: TaskGoal,
    pub now_line: String,
    pub next_line: String,
    pub intent: NextIntent,
    pub evidence_boxes: Vec<AnchorBox>,
    pub confidence: f32,
    pub expected_transition: String,
    pub refuse_reason: Option<String>,
}

/// Optional capture identity for multi-frame date fusion (Task 1.3).
#[derive(Debug, Clone)]
pub struct ObserveFrameMeta<'a> {
    pub frame_id: String,
    pub capture_monotonic_ms: u64,
    pub frame_w: i32,
    pub frame_h: i32,
    /// Full-frame BGRA for date status color sampling (P4 multi-source).
    pub pixels_bgra: Option<&'a [u8]>,
}


mod policy;

pub use policy::{plan_captcha_keyboard, CaptchaKeyStroke, CaptchaKeyboardPlan};

use policy::{
    confirm_order, mp_panel, nav_goal_label, nav_sport_icon, nav_text, notice_ack, select_slot,
    venue_detail, wechat_discover, wechat_shell_or_unknown, xmu_wx_authorize,
};

pub fn decide_next(
    snap: &UiSnapshot,
    candidates: &CandidateSet,
    goal: &TaskGoal,
) -> ObservationDecision {
    decide_next_with_scene(snap, candidates, goal, None, None)
}

pub fn decide_next_with_scene(
    snap: &UiSnapshot,
    candidates: &CandidateSet,
    goal: &TaskGoal,
    scene: Option<&mut crate::scene::SceneState>,
    frame: Option<&ObserveFrameMeta<'_>>,
) -> ObservationDecision {
    if candidates.has_label("验证码错误") {
        return finish(
            FlowDomain::XmuBooking,
            Page::ConfirmOrder,
            snap,
            goal,
            "现在：确认预约",
            "下一步：等待",
            NextIntent::Wait {
                reason: "CAPTCHA_ERROR_SETTLE".into(),
                // Desired-state tick (Spec §6); host re-glances — not fixed 5s sleep.
                approx_ms: crate::submit_observe::CAPTCHA_WRONG_TICK_MS,
            },
            Vec::new(),
            "confirm_order",
            Some("CAPTCHA_ERROR_SETTLE".into()),
        );
    }
    if candidates.has_label("请输入验证码") || candidates.has_label("请输入验证") {
        return finish(
            FlowDomain::XmuBooking,
            Page::ConfirmOrder,
            snap,
            goal,
            "现在：确认预约",
            "下一步：点击输入区域唤起键盘",
            NextIntent::Wait {
                reason: "CAPTCHA_NEED_INPUT".into(),
                approx_ms: 1_500,
            },
            Vec::new(),
            "confirm_order",
            Some("CAPTCHA_NEED_INPUT".into()),
        );
    }
    // Mis-tap into「我的课程」— probe backs out before 场馆预约.
    if candidates.has_label("去选课") && candidates.has_label("我的课程") {
        if let Some(hit) = candidates
            .find_click("返回")
            .or_else(|| candidates.find_click("首页"))
        {
            return navigate(
                snap,
                goal,
                FlowDomain::XmuBooking,
                "现在：误入我的课程",
                "下一步：返回首页",
                hit.label.as_str(),
                hit,
                Page::Home,
                "home",
            );
        }
        return stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "COURSES_WRONG_PAGE",
            "暂停，误入我的课程，请手动返回",
        );
    }
    if candidates.has_label("请求中") || snap.page == Page::XmuAuthWaiting {
        return finish(
            FlowDomain::XmuAuth,
            Page::XmuAuthWaiting,
            snap,
            goal,
            "现在：登录等待",
            "下一步：等待",
            NextIntent::Wait {
                reason: "AUTH_WAITING".into(),
                approx_ms: AUTH_WAIT_POLL_MS,
            },
            Vec::new(),
            "home",
            Some("AUTH_WAITING".into()),
        );
    }
    if let Some(hit) = notice_ack(candidates) {
        return navigate(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "现在：预约须知",
            "下一步：点我知道了",
            "我知道了",
            hit,
            Page::VenueSportList,
            "venue_sport_list",
        );
    }
    if matches!(snap.overlay, Overlay::Notice | Overlay::Modal) {
        return stop(
            snap,
            goal,
            domain_of(snap.page),
            "OVERLAY_BLOCKS_ACTION",
            "暂停，有弹窗挡住",
        );
    }

    match snap.page {
        Page::WechatDiscover => wechat_discover(snap, goal, candidates),
        Page::WechatMpPanel | Page::WechatMpSearch => mp_panel(snap, goal, candidates),
        Page::WechatMain | Page::Unknown => {
            if crate::state::title_is_xmu_miniprogram(&snap.title_hint) {
                return stop(
                    snap,
                    goal,
                    FlowDomain::XmuBooking,
                    "ANCHOR_NOT_FOUND",
                    "暂停，找不到场馆预约",
                );
            }
            wechat_shell_or_unknown(snap, goal, candidates)
        }
        Page::XmuLoginGate => nav_text(
            snap,
            goal,
            candidates,
            FlowDomain::XmuAuth,
            "现在：登录门",
            "立即登录",
            "下一步：打开立即登录",
            Page::XmuWxAuthorize,
            "xmu_wx_authorize",
        ),
        Page::XmuWxAuthorize => xmu_wx_authorize(snap, goal, candidates),
        Page::BookingNotice => nav_text(
            snap,
            goal,
            candidates,
            FlowDomain::XmuBooking,
            "现在：预约须知",
            "我知道了",
            "下一步：点我知道了",
            Page::VenueSportList,
            "venue_sport_list",
        ),
        Page::Home => nav_text(
            snap,
            goal,
            candidates,
            FlowDomain::XmuBooking,
            "现在：体育首页",
            "场馆预约",
            "下一步：打开场馆预约",
            Page::VenueSportList,
            "venue_sport_list",
        ),
        Page::VenueSportList => nav_sport_icon(snap, goal, candidates),
        Page::VenueList => {
            let next = goal
                .venue()
                .map(|v| format!("下一步：点{v}"))
                .unwrap_or_else(|| "下一步：选择目标场馆".into());
            nav_goal_label(
                snap,
                goal,
                candidates,
                goal.venue(),
                "GOAL_UNSPECIFIED",
                "VENUE_MISMATCH",
                "现在：场馆列表",
                &next,
                Page::VenueDetail,
                "venue_detail",
            )
        }
        Page::VenueDetail => venue_detail(snap, goal, candidates, scene, frame),
        Page::SelectSlot => select_slot(snap, goal, candidates),
        Page::ConfirmOrder => confirm_order(snap, goal, candidates),
        Page::MyReservations | Page::Success => stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "ALREADY_DONE",
            "停止，已在结果页",
        ),
        Page::XmuAuthWaiting => finish(
            FlowDomain::XmuAuth,
            Page::XmuAuthWaiting,
            snap,
            goal,
            "现在：登录等待",
            "下一步：等待",
            NextIntent::Wait {
                reason: "AUTH_WAITING".into(),
                approx_ms: AUTH_WAIT_POLL_MS,
            },
            Vec::new(),
            "home",
            Some("AUTH_WAITING".into()),
        ),
        Page::OrderDetail | Page::CancelModal => stop(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "NOT_ON_BOOKING_PATH",
            "暂停，不在预约路径上",
        ),
    }
}


pub(super) fn navigate(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    domain: FlowDomain,
    now: &str,
    next: &str,
    label: &str,
    hit: &Candidate,
    expected: Page,
    transition: &str,
) -> ObservationDecision {
    let anchor = box_of(hit);
    finish(
        domain,
        snap.page,
        snap,
        goal,
        now,
        next,
        NextIntent::Navigate {
            label: label.to_string(),
            anchor: anchor.clone(),
            expected_page: expected,
        },
        vec![anchor],
        transition,
        None,
    )
}

pub(super) fn stop(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    domain: FlowDomain,
    reason: &str,
    next: &str,
) -> ObservationDecision {
    let now = match snap.page {
        Page::SelectSlot => "现在：场次页",
        Page::ConfirmOrder => "现在：确认预约",
        Page::VenueDetail => "现在：场馆详情",
        Page::VenueSportList => "现在：运动列表",
        Page::VenueList => "现在：场馆列表",
        Page::XmuWxAuthorize => "现在：微信授权",
        Page::XmuLoginGate => "现在：登录门",
        Page::MyReservations | Page::Success => "现在：我的预约",
        Page::Unknown => "现在：不确定",
        Page::WechatMain => "现在：微信",
        Page::WechatDiscover => "现在：微信，发现已打开",
        Page::WechatMpPanel | Page::WechatMpSearch => "现在：小程序面板",
        _ => {
            return finish(
                domain,
                snap.page,
                snap,
                goal,
                &format!("现在：{}", snap.page.as_str()),
                &format!("下一步：{next}"),
                NextIntent::Stop {
                    reason: reason.to_string(),
                },
                Vec::new(),
                snap.page.as_str(),
                Some(reason.to_string()),
            );
        }
    };
    finish(
        domain,
        snap.page,
        snap,
        goal,
        now,
        &format!("下一步：{next}"),
        NextIntent::Stop {
            reason: reason.to_string(),
        },
        Vec::new(),
        snap.page.as_str(),
        Some(reason.to_string()),
    )
}

pub(super) fn finish(
    domain: FlowDomain,
    page: Page,
    snap: &UiSnapshot,
    goal: &TaskGoal,
    now: &str,
    next: &str,
    intent: NextIntent,
    boxes: Vec<AnchorBox>,
    transition: &str,
    refuse: Option<String>,
) -> ObservationDecision {
    ObservationDecision {
        domain,
        page,
        overlay: snap.overlay,
        goal: goal.clone(),
        now_line: now.to_string(),
        next_line: next.to_string(),
        intent,
        evidence_boxes: boxes,
        confidence: snap.confidence,
        expected_transition: transition.to_string(),
        refuse_reason: refuse,
    }
}

pub(super) fn box_of(hit: &Candidate) -> AnchorBox {
    let b = hit.click_bbox();
    AnchorBox {
        x0: b.x0,
        y0: b.y0,
        x1: b.x1,
        y1: b.y1,
    }
}

pub(super) fn domain_of(page: Page) -> FlowDomain {
    match page {
        Page::WechatMain | Page::WechatDiscover | Page::WechatMpPanel | Page::WechatMpSearch => {
            FlowDomain::WechatShell
        }
        Page::XmuLoginGate | Page::XmuWxAuthorize | Page::XmuAuthWaiting => FlowDomain::XmuAuth,
        Page::Home
        | Page::BookingNotice
        | Page::VenueSportList
        | Page::VenueList
        | Page::VenueDetail
        | Page::SelectSlot
        | Page::ConfirmOrder
        | Page::MyReservations
        | Page::Success => FlowDomain::XmuBooking,
        _ => FlowDomain::Unknown,
    }
}

#[cfg(test)]

mod tests {
    use super::*;
    use crate::candidates::{
        parse_candidates, BBox, Candidate, CandidateKind, CandidateSource, ColorRules, ParseInput,
        ShellVisualState,
    };
    use crate::ocr::{OcrHit, OcrRegionCache};
    use crate::state::{CaptchaPhase, SlotCta};

    fn cache(texts: &[(&str, i32, i32)]) -> CandidateSet {
        let ocr = OcrRegionCache::from_hits(
            texts
                .iter()
                .map(|(t, x, y)| OcrHit {
                    text: (*t).into(),
                    x: *x,
                    y: *y,
                    w: 48,
                    h: 18,
                    score: 0.9,
                })
                .collect(),
        );
        parse_candidates(&ParseInput {
            frame_w: 900,
            frame_h: 900,
            ocr: &ocr,
            pixels_bgra: None,
            detections: &[],
            templates: &[],
            rules: &ColorRules::default(),
        })
    }

    fn snap(page: Page) -> UiSnapshot {
        UiSnapshot {
            page,
            confidence: 0.9,
            captcha: CaptchaPhase::Unknown,
            slot_cta: SlotCta::Unknown,
            ..UiSnapshot::default()
        }
    }

    fn shell(id: &str, label: &str) -> CandidateSet {
        let mut c = Candidate {
            id: "shell".into(),
            bbox: BBox {
                x0: 12,
                y0: 120,
                x1: 48,
                y1: 156,
            },
            kind: CandidateKind::Icon,
            label: label.into(),
            score: 0.95,
            sources: vec![CandidateSource::Template],
            actionable: true,
            template_id: Some(id.into()),
            slot_meta: None,
            evidence: vec!["test:shell_state".into()],
            element_id: None,
            element_state: None,
            theme: None,
            visual_bbox: None,
            hit_bbox: None,
        };
        // Keep test fixtures aligned with production template → identity projection.
        if id.starts_with("discover_compass") {
            c.element_id = Some(crate::candidates::ELEMENT_DISCOVER.into());
        } else if id.starts_with("miniprogram_entry") {
            c.element_id = Some(crate::candidates::ELEMENT_MINIPROGRAM.into());
        }
        if id.contains("_inactive") {
            c.element_state = Some(ShellVisualState::Inactive);
        } else if id.contains("_active") {
            c.element_state = Some(ShellVisualState::Active);
        }
        c.visual_bbox = Some(c.bbox.clone());
        c.hit_bbox = Some(c.bbox.clone());
        CandidateSet {
            items: vec![c],
            dedup_iou: 0.3,
        }
    }

    fn goal_venue() -> TaskGoal {
        TaskGoal {
            sport: Some("乒乓球".into()),
            venue: Some("翔安校区乒乓球室".into()),
            date: Some("2026-09-18".into()),
            slot_time: Some("19:30".into()),
            ..TaskGoal::default()
        }
    }

    #[test]
    fn discover_text_without_glyph_does_not_click() {
        let ocr = cache(&[("发现", 20, 40), ("朋友圈", 80, 80), ("小程序", 80, 200)]);
        let d = decide_next(&snap(Page::WechatDiscover), &ocr, &TaskGoal::default());
        assert!(
            matches!(d.intent, NextIntent::Stop { ref reason } if reason == "ANCHOR_NOT_FOUND")
        );
        assert!(!d.next_line.contains("打开发现"));
    }

    #[test]
    fn unknown_wechat_without_discover_box_stops() {
        let ocr = cache(&[("文件传输助手", 40, 80), ("发送", 400, 600)]);
        let d = decide_next(&snap(Page::Unknown), &ocr, &TaskGoal::default());
        assert!(
            matches!(d.intent, NextIntent::Stop { ref reason } if reason == "ANCHOR_NOT_FOUND")
        );
        assert!(d.evidence_boxes.is_empty());
    }

    #[test]
    fn inactive_discover_is_clicked_even_when_chat_ocr_is_unknown() {
        let candidates = shell("discover_compass_inactive", "发现");
        let d = decide_next(&snap(Page::Unknown), &candidates, &TaskGoal::default());
        assert!(matches!(d.intent, NextIntent::Navigate { ref label, .. } if label == "发现"));
        assert_eq!(d.next_line, "下一步：打开发现");
    }

    #[test]
    fn themed_inactive_discover_is_clicked() {
        let candidates = shell("discover_compass_inactive_dark", "发现");
        let d = decide_next(&snap(Page::Unknown), &candidates, &TaskGoal::default());
        assert!(matches!(d.intent, NextIntent::Navigate { ref label, .. } if label == "发现"));
        assert_eq!(d.next_line, "下一步：打开发现");
    }

    #[test]
    fn themed_active_discover_is_not_clicked_again() {
        let candidates = shell("discover_compass_active_light", "发现");
        let d = decide_next(&snap(Page::Unknown), &candidates, &TaskGoal::default());
        assert!(
            matches!(d.intent, NextIntent::Stop { ref reason } if reason == "ANCHOR_NOT_FOUND")
        );
        assert!(!d.next_line.contains("打开发现"));
    }

    #[test]
    fn active_discover_then_inactive_miniprogram_clicks_miniprogram() {
        let mut candidates = shell("discover_compass_active", "发现");
        candidates.items.push(Candidate {
            template_id: Some("miniprogram_entry_inactive".into()),
            label: "小程序".into(),
            bbox: BBox {
                x0: 20,
                y0: 250,
                x1: 300,
                y1: 300,
            },
            kind: CandidateKind::ListRow,
            element_id: None,
            element_state: None,
            theme: None,
            visual_bbox: None,
            hit_bbox: None,
            ..candidates.items[0].clone()
        });
        let d = decide_next(&snap(Page::Unknown), &candidates, &TaskGoal::default());
        assert!(matches!(d.intent, NextIntent::Navigate { ref label, .. } if label == "小程序"));
        assert_eq!(d.next_line, "下一步：打开小程序");
    }

    #[test]
    fn active_miniprogram_is_not_clicked_a_second_time() {
        let candidates = shell("miniprogram_entry_active", "小程序");
        let d = decide_next(
            &snap(Page::WechatDiscover),
            &candidates,
            &TaskGoal::default(),
        );
        assert!(
            matches!(d.intent, NextIntent::Stop { ref reason } if reason == "ANCHOR_NOT_FOUND")
        );
        assert_eq!(d.now_line, "现在：小程序面板");
    }

    #[test]
    fn panel_opens_xmu_else_search() {
        let with = cache(&[("小程序", 20, 40), ("厦大体育", 80, 160)]);
        let d = decide_next(&snap(Page::WechatMpPanel), &with, &TaskGoal::default());
        assert!(matches!(d.intent, NextIntent::Navigate { ref label, .. } if label == "厦大体育"));

        let missing = cache(&[("小程序", 20, 40), ("搜索", 360, 40), ("最近使用", 40, 100)]);
        let d2 = decide_next(&snap(Page::WechatMpPanel), &missing, &TaskGoal::default());
        assert!(
            matches!(
                d2.intent,
                NextIntent::AtomicComposite { ref label, ref steps, .. }
                    if label == "搜索厦大体育"
                        && steps.len() == 2
                        && matches!(steps[0], AtomicStep::Click { ref label, .. } if label == "搜索")
                        && matches!(steps[1], AtomicStep::TypeText { ref text } if text == "厦大体育")
            ),
            "got {:?}",
            d2.intent
        );
        assert!(d2.next_line.contains("搜索并打开厦大体育"));
    }

    #[test]
    fn panel_prefers_crest_tile_over_ocr_caption() {
        let mut set = cache(&[("厦大体育", 80, 220)]);
        set.items.push(Candidate {
            id: "crest".into(),
            bbox: BBox {
                x0: 200,
                y0: 120,
                x1: 280,
                y1: 220,
            },
            kind: CandidateKind::Icon,
            label: "厦大体育".into(),
            score: 0.91,
            sources: vec![CandidateSource::Template],
            actionable: true,
            template_id: Some("xmu_sports_icon_dark".into()),
            slot_meta: None,
            evidence: vec!["template:xmu_sports_icon_dark".into()],
            element_id: Some(crate::candidates::ELEMENT_XMU_SPORTS.into()),
            element_state: None,
            theme: Some("dark".into()),
            visual_bbox: Some(BBox {
                x0: 210,
                y0: 120,
                x1: 270,
                y1: 180,
            }),
            hit_bbox: Some(BBox {
                x0: 200,
                y0: 120,
                x1: 280,
                y1: 220,
            }),
        });
        let d = decide_next(&snap(Page::WechatMpPanel), &set, &TaskGoal::default());
        match d.intent {
            NextIntent::Navigate { anchor, .. } => {
                assert_eq!(anchor.y0, 120);
                assert_eq!(anchor.x0, 200);
            }
            other => panic!("expected crest navigate, got {other:?}"),
        }
    }

    #[test]
    fn sousousou_is_not_panel_search() {
        let set = cache(&[("小程序", 20, 40), ("搜一搜", 80, 180), ("朋友圈", 80, 80)]);
        let d = decide_next(&snap(Page::WechatDiscover), &set, &TaskGoal::default());
        assert!(
            matches!(d.intent, NextIntent::Stop { ref reason } if reason == "ANCHOR_NOT_FOUND")
        );
        assert!(!d.next_line.contains("搜索"));
    }

    #[test]
    fn auth_wait_does_not_navigate() {
        let ocr = cache(&[("授权登录", 70, 480), ("请求中", 180, 400)]);
        let d = decide_next(&snap(Page::XmuWxAuthorize), &ocr, &TaskGoal::default());
        assert!(matches!(
            d.intent,
            NextIntent::Wait {
                approx_ms: AUTH_WAIT_POLL_MS,
                ..
            }
        ));
        assert_eq!(d.next_line, "下一步：等待");
    }

    #[test]
    fn home_does_not_go_back_to_discover() {
        let ocr = cache(&[("场馆预约", 80, 280), ("首页", 40, 720), ("发现", 8, 8)]);
        let d = decide_next(&snap(Page::Home), &ocr, &TaskGoal::default());
        assert!(matches!(d.intent, NextIntent::Navigate { ref label, .. } if label == "场馆预约"));
        assert_eq!(d.domain, FlowDomain::XmuBooking);
    }

    #[test]
    fn miniprogram_window_does_not_search_discover() {
        let mut snap = snap(Page::Unknown);
        snap.title_hint = "厦大体育".into();
        let set = cache(&[("发现", 20, 400), ("小程序", 20, 460)]);
        let d = decide_next(&snap, &set, &TaskGoal::default());
        assert!(
            !d.next_line.contains("发现"),
            "next must not look for the shell compass: {}",
            d.next_line
        );
        assert!(d.next_line.contains("场馆预约"));
        assert!(matches!(
            d.intent,
            NextIntent::Stop { ref reason } if reason == "ANCHOR_NOT_FOUND"
        ));
    }

    #[test]
    fn venue_detail_requires_name_and_date() {
        let ocr = cache(&[
            ("场馆详情", 160, 40),
            ("翔安校区乒乓球室", 80, 120),
            ("今天", 200, 180),
            ("09月18日", 200, 210),
            ("预定", 200, 250),
        ]);
        let d = decide_next(&snap(Page::VenueDetail), &ocr, &goal_venue());
        match d.intent {
            NextIntent::Navigate {
                ref label,
                ref anchor,
                expected_page,
            } => {
                assert_eq!(label, "预定");
                assert_eq!(expected_page, Page::SelectSlot);
                // Click the status pill, not the MM月DD日 row (y≈210).
                assert!(
                    anchor.y0 >= 240,
                    "anchor should be on 预定 pill, got y0={}",
                    anchor.y0
                );
            }
            other => panic!("expected navigate to 预定, got {other:?}"),
        }
        assert_eq!(d.next_line, "下一步：选择目标日期");

        let wrong = decide_next(
            &snap(Page::VenueDetail),
            &ocr,
            &TaskGoal {
                venue: Some("思明校区健身房".into()),
                date: Some("2026-09-18".into()),
                ..TaskGoal::default()
            },
        );
        assert!(
            matches!(wrong.intent, NextIntent::Stop { ref reason } if reason == "VENUE_MISMATCH")
        );
    }

    #[test]
    fn venue_detail_zhanwu_waits_for_multiframe() {
        let ocr = cache(&[
            ("翔安校区乒乓球室", 80, 120),
            ("今天", 200, 180),
            ("09月18日", 200, 210),
            ("暂无", 200, 250),
        ]);
        let d = decide_next(&snap(Page::VenueDetail), &ocr, &goal_venue());
        assert!(
            matches!(
                d.intent,
                NextIntent::Wait {
                    ref reason,
                    ..
                } if reason == "DATE_EVIDENCE_WEAK"
            ),
            "single-frame 暂无 must Wait, got {:?}",
            d.intent
        );
        assert!(d.evidence_boxes.is_empty());
    }

    #[test]
    fn venue_detail_zhanwu_stops_after_three_stable_frames() {
        let ocr = cache(&[
            ("翔安校区乒乓球室", 80, 120),
            ("今天", 200, 180),
            ("09月18日", 200, 210),
            ("暂无", 200, 250),
        ]);
        let mut scene = crate::scene::SceneState::new();
        let mut last = decide_next_with_scene(
            &snap(Page::VenueDetail),
            &ocr,
            &goal_venue(),
            Some(&mut scene),
            Some(&ObserveFrameMeta {
                frame_id: "u0".into(),
                capture_monotonic_ms: 10,
                frame_w: 900,
                frame_h: 900,
                pixels_bgra: None,
            }),
        );
        for i in 1..3 {
            last = decide_next_with_scene(
                &snap(Page::VenueDetail),
                &ocr,
                &goal_venue(),
                Some(&mut scene),
                Some(&ObserveFrameMeta {
                    frame_id: format!("u{i}"),
                    capture_monotonic_ms: 10 + i as u64,
                    frame_w: 900,
                    frame_h: 900,
                    pixels_bgra: None,
                }),
            );
        }
        assert!(
            matches!(last.intent, NextIntent::Stop { ref reason } if reason == "DATE_UNAVAILABLE"),
            "got {:?}",
            last.intent
        );
    }

    #[test]
    fn venue_detail_wrong_day_waits_not_mismatch() {
        let ocr = cache(&[
            ("翔安校区乒乓球室", 80, 120),
            ("周二", 200, 180),
            ("09月22日", 200, 210),
            ("预定", 200, 250),
        ]);
        let d = decide_next(&snap(Page::VenueDetail), &ocr, &goal_venue());
        assert!(
            matches!(
                d.intent,
                NextIntent::Wait {
                    ref reason,
                    ..
                } if reason == "DATE_EVIDENCE_WEAK"
            ),
            "single-frame missing day must not DATE_MISMATCH, got {:?}",
            d.intent
        );
    }

    #[test]
    fn venue_detail_today_without_md_waits_not_mismatch() {
        let ocr = cache(&[
            ("翔安校区乒乓球室", 80, 120),
            ("今天", 200, 180),
            ("预定", 200, 250),
        ]);
        let d = decide_next(&snap(Page::VenueDetail), &ocr, &goal_venue());
        assert!(
            matches!(
                d.intent,
                NextIntent::Wait {
                    ref reason,
                    ..
                } if reason == "DATE_EVIDENCE_WEAK"
            ),
            "got {:?}",
            d.intent
        );
    }

    #[test]
    fn venue_detail_does_not_click_other_day_yuding() {
        let ocr = cache(&[
            ("翔安校区乒乓球室", 80, 120),
            ("今天", 200, 180),
            ("09月18日", 200, 210),
            ("暂无", 200, 250),
            ("周二", 400, 180),
            ("09月19日", 400, 210),
            ("预定", 400, 250),
        ]);
        let d = decide_next(&snap(Page::VenueDetail), &ocr, &goal_venue());
        assert!(
            matches!(
                d.intent,
                NextIntent::Wait {
                    ref reason,
                    ..
                } if reason == "DATE_EVIDENCE_WEAK"
            ),
            "must not fall back to the other card's 预定: {:?}",
            d.intent
        );
    }

    #[test]
    fn slot_page_stops_without_cell() {
        let ocr = cache(&[
            ("选择场次", 20, 40),
            ("可预约", 20, 90),
            ("不可选", 80, 90),
            ("19:30-21:00", 20, 200),
        ]);
        let d = decide_next(&snap(Page::SelectSlot), &ocr, &goal_venue());
        assert!(
            matches!(d.intent, NextIntent::Stop { ref reason } if reason == "SLOT_GRID_NOT_LOCALIZED")
        );
        assert_eq!(d.now_line, "现在：场次页");
        assert!(d.next_line.contains("无法确定可约格"));
        assert!(d.evidence_boxes.is_empty());
    }

    #[test]
    fn captcha_error_waits_desired_state_tick() {
        let ocr = cache(&[
            ("验证码", 20, 400),
            ("确定预约", 40, 700),
            ("验证码错误", 160, 360),
        ]);
        let d = decide_next(&snap(Page::ConfirmOrder), &ocr, &TaskGoal::default());
        assert!(matches!(
            d.intent,
            NextIntent::Wait {
                approx_ms,
                ..
            } if approx_ms <= 5_000 && approx_ms >= 200
        ));
    }

    #[test]
    fn confirm_without_blank_localization_stops() {
        let ocr = cache(&[("验证码", 20, 400), ("确定预约", 40, 700)]);
        let d = decide_next(&snap(Page::ConfirmOrder), &ocr, &TaskGoal::default());
        assert!(
            matches!(d.intent, NextIntent::Stop { ref reason } if reason == "CAPTCHA_HIT_NOT_LOCALIZED")
        );
    }

    #[test]
    fn full_captcha_goes_to_submit() {
        let mut frame = snap(Page::ConfirmOrder);
        frame.captcha = CaptchaPhase::IdleFull;
        let d = decide_next(
            &frame,
            &cache(&[("确定预约", 40, 700)]),
            &TaskGoal::default(),
        );
        assert!(matches!(
            d.intent,
            NextIntent::Navigate { ref label, expected_page, .. }
                if label == "确定预约" && expected_page == Page::MyReservations
        ));
    }

    #[test]
    fn gym_tile_is_not_the_venue_row_that_contains_gym() {
        let set = cache(&[("健身房", 80, 180), ("思明校区健身房", 80, 640)]);
        let mut goal = TaskGoal::default();
        goal.sport = Some("健身房".into());
        let d = decide_next(&snap(Page::VenueSportList), &set, &goal);
        assert_eq!(d.next_line, "下一步：点健身房");
        match d.intent {
            NextIntent::Navigate { anchor, .. } => {
                assert!(anchor.y0 < 400, "click the top sport caption, y0={}", anchor.y0);
            }
            other => panic!("expected sport tile, got {other:?}"),
        }
    }

    #[test]
    fn agree_checkbox_clicks_left_of_protocol_links() {
        // Full-line OCR: gray caption + blue 《用户协议》— must not click line center.
        let set = cache(&[("阅读并同意以下协议《用户协议》《隐私政策》", 80, 400), ("授权登录", 160, 520)]);
        let caption = set
            .items
            .iter()
            .find(|c| c.label.contains("阅读并同意"))
            .expect("caption");
        let full = caption.click_bbox().clone();
        let d = decide_next(&snap(Page::XmuWxAuthorize), &set, &TaskGoal::default());
        match d.intent {
            NextIntent::Navigate {
                ref label,
                ref anchor,
                ..
            } => {
                assert_eq!(label, "阅读并同意");
                let mid = (full.x0 + full.x1) / 2;
                let click_cx = (anchor.x0 + anchor.x1) / 2;
                assert!(
                    click_cx < mid,
                    "click must be left of caption mid; click_cx={click_cx} mid={mid} anchor={anchor:?} full={full:?}"
                );
                assert!(
                    (anchor.x1 - anchor.x0) as f32 <= (full.x1 - full.x0) as f32 * 0.5 + 1.0,
                    "hit width must be ≤ half caption; anchor={anchor:?} full={full:?}"
                );
            }
            other => panic!("expected agree click, got {other:?}"),
        }
    }

    #[test]
    fn authorize_click_when_checkbox_text_absent() {
        let set = cache(&[("授权登录", 160, 520)]);
        let d = decide_next(&snap(Page::XmuWxAuthorize), &set, &TaskGoal::default());
        assert!(matches!(
            d.intent,
            NextIntent::Navigate { ref label, .. } if label == "授权登录"
        ));
    }

    #[test]
    fn enabled_blue_auth_button_is_the_next_click() {
        let mut set = cache(&[
            ("阅读并同意以下协议", 80, 400),
            ("授权登录", 200, 480),
        ]);
        let btn = set
            .items
            .iter_mut()
            .find(|c| c.label.contains("授权登录"))
            .expect("caption");
        btn.element_id = Some(crate::candidates::ELEMENT_AUTH_LOGIN.into());
        btn.evidence.push("auth_login_enabled".into());
        btn.hit_bbox = Some(BBox {
            x0: 40,
            y0: 450,
            x1: 480,
            y1: 520,
        });
        let d = decide_next(&snap(Page::XmuWxAuthorize), &set, &TaskGoal::default());
        match d.intent {
            NextIntent::Navigate { ref label, ref anchor, .. } => {
                assert_eq!(label, "授权登录");
                assert!(anchor.x1 - anchor.x0 > 200, "click the blue button, not the radio");
            }
            other => panic!("expected authorize click, got {other:?}"),
        }
        assert!(d.next_line.contains("授权登录"));
    }

    fn available_slot_cell(start: &str) -> Candidate {
        let mut cell = shell("slot", start).items.remove(0);
        cell.kind = CandidateKind::SlotCell;
        cell.template_id = None;
        cell.label = start.into();
        cell.slot_meta = Some(crate::candidates::SlotMeta {
            start_time: start.into(),
            court_index: Some(1),
            court_label: None,
            state: crate::slots::SlotState::Available,
        });
        cell
    }

    #[test]
    fn select_slot_ready_clicks_confirm_not_cell() {
        let mut set = cache(&[("确认预约", 160, 680)]);
        set.items.push(available_slot_cell("19:30"));
        let mut s = snap(Page::SelectSlot);
        s.slot_cta = SlotCta::Ready;
        let d = decide_next(&s, &set, &goal_venue());
        match d.intent {
            NextIntent::Navigate {
                ref label,
                expected_page,
                ..
            } => {
                assert_eq!(label, "确认预约");
                assert_eq!(expected_page, Page::ConfirmOrder);
            }
            other => panic!("expected confirm CTA, got {other:?}"),
        }
        assert!(d.next_line.contains("确认预约"), "{}", d.next_line);
    }

    #[test]
    fn select_slot_cell_click_expects_same_page() {
        let d = decide_next(
            &snap(Page::SelectSlot),
            &CandidateSet {
                items: vec![available_slot_cell("19:30")],
                dedup_iou: 0.3,
            },
            &goal_venue(),
        );
        match d.intent {
            NextIntent::Navigate {
                ref label,
                expected_page,
                ..
            } => {
                assert_eq!(label, "19:30");
                assert_eq!(
                    expected_page,
                    Page::SelectSlot,
                    "选格不换页，等底部 CTA 激活"
                );
            }
            other => panic!("expected slot click, got {other:?}"),
        }
        assert!(d.next_line.contains("点可约格"), "{}", d.next_line);
    }

    #[test]
    fn select_slot_ready_without_cta_stops() {
        let mut s = snap(Page::SelectSlot);
        s.slot_cta = SlotCta::Ready;
        let d = decide_next(
            &s,
            &CandidateSet {
                items: vec![available_slot_cell("19:30")],
                dedup_iou: 0.3,
            },
            &goal_venue(),
        );
        assert!(matches!(
            d.intent,
            NextIntent::Stop {
                ref reason
            } if reason == "CONFIRM_CTA_NOT_LOCALIZED"
        ));
    }

    #[test]
    fn slot_time_fallback_picks_nearest_earlier() {
        let mut goal = goal_venue();
        goal.slot_time = Some("19:30".into());
        goal.allow_time_fallback = true;
        goal.time_fallback_order = "up_then_down".into();
        let d = decide_next(
            &snap(Page::SelectSlot),
            &CandidateSet {
                items: vec![
                    available_slot_cell("18:00"),
                    available_slot_cell("12:00"),
                ],
                dedup_iou: 0.3,
            },
            &goal,
        );
        match d.intent {
            NextIntent::Navigate { ref label, .. } => {
                assert_eq!(label, "18:00", "先更早 → 邻近 18:00");
            }
            other => panic!("expected nearby slot, got {other:?}"),
        }
    }

    #[test]
    fn slot_time_mismatch_does_not_pick_another() {
        let mut cell = shell("slot", "18:00").items.remove(0);
        cell.kind = CandidateKind::SlotCell;
        cell.template_id = None;
        cell.slot_meta = Some(crate::candidates::SlotMeta {
            start_time: "18:00".into(),
            court_index: Some(0),
            court_label: Some("01".into()),
            state: crate::slots::SlotState::Available,
        });
        let d = decide_next(
            &snap(Page::SelectSlot),
            &CandidateSet {
                items: vec![cell],
                dedup_iou: 0.3,
            },
            &goal_venue(),
        );
        assert!(matches!(
            d.intent,
            NextIntent::Stop { ref reason } if reason == "SLOT_TIME_MISMATCH"
        ));
    }

    #[test]
    fn slot_court_prefer_falls_back_to_row() {
        let mut cell = shell("slot", "19:30").items.remove(0);
        cell.kind = CandidateKind::SlotCell;
        cell.template_id = None;
        cell.slot_meta = Some(crate::candidates::SlotMeta {
            start_time: "19:30".into(),
            court_index: Some(2),
            court_label: Some("03".into()),
            state: crate::slots::SlotState::Available,
        });
        let mut goal = goal_venue();
        goal.prefer_court = Some("01".into());
        let d = decide_next(
            &snap(Page::SelectSlot),
            &CandidateSet {
                items: vec![cell],
                dedup_iou: 0.3,
            },
            &goal,
        );
        assert!(
            matches!(d.intent, NextIntent::Navigate { ref label, .. } if label == "19:30"),
            "无偏好场地时仍点该时段可约格: {:?}",
            d.intent
        );
    }

    #[test]
    fn slot_court_offset_picks_third_then_clamps() {
        fn cell_at(x: i32, label: &str) -> Candidate {
            let mut c = available_slot_cell("19:30");
            c.id = format!("slot-{label}");
            let box_ = BBox {
                x0: x,
                y0: 200,
                x1: x + 40,
                y1: 240,
            };
            c.bbox = box_.clone();
            c.hit_bbox = Some(box_.clone());
            c.visual_bbox = Some(box_);
            c.slot_meta = Some(crate::candidates::SlotMeta {
                start_time: "19:30".into(),
                court_index: Some(x / 100),
                court_label: Some(label.into()),
                state: crate::slots::SlotState::Available,
            });
            c
        }
        let items = vec![
            cell_at(100, "01"),
            cell_at(200, "02"),
            cell_at(300, "03"),
            cell_at(400, "04"),
            cell_at(500, "05"),
        ];
        let mut goal = goal_venue();
        goal.court_offset = 2;
        let d = decide_next(
            &snap(Page::SelectSlot),
            &CandidateSet {
                items: items.clone(),
                dedup_iou: 0.3,
            },
            &goal,
        );
        match d.intent {
            NextIntent::Navigate { ref anchor, .. } => {
                assert_eq!(anchor.x0, 300, "offset=2 → 第 3 格 x=300");
            }
            other => panic!("expected navigate, got {other:?}"),
        }

        goal.court_offset = 9;
        let d2 = decide_next(
            &snap(Page::SelectSlot),
            &CandidateSet {
                items: vec![cell_at(100, "01"), cell_at(200, "02")],
                dedup_iou: 0.3,
            },
            &goal,
        );
        match d2.intent {
            NextIntent::Navigate { ref anchor, .. } => {
                assert_eq!(anchor.x0, 200, "不足时点最后一个");
            }
            other => panic!("expected navigate, got {other:?}"),
        }
    }

    #[test]
    fn slot_all_grey_stops() {
        let mut cell = shell("slot", "19:30").items.remove(0);
        cell.kind = CandidateKind::SlotCell;
        cell.actionable = false;
        cell.template_id = None;
        cell.slot_meta = Some(crate::candidates::SlotMeta {
            start_time: "19:30".into(),
            court_index: Some(0),
            court_label: Some("01".into()),
            state: crate::slots::SlotState::Locked,
        });
        let d = decide_next(
            &snap(Page::SelectSlot),
            &CandidateSet {
                items: vec![cell],
                dedup_iou: 0.3,
            },
            &goal_venue(),
        );
        assert!(matches!(
            d.intent,
            NextIntent::Stop { ref reason } if reason == "SLOT_ALL_GREY"
        ));
    }

    #[test]
    fn captcha_keyboard_clicks_next_key() {
        let mut frame = snap(Page::ConfirmOrder);
        frame.keyboard_open = true;
        frame.input_chars = String::new();
        let set = cache(&[
            ("验证码", 40, 400),
            ("完成", 300, 620),
            ("取消", 40, 620),
            ("A", 80, 700),
        ]);
        let mut goal = TaskGoal::default();
        goal.recognized_captcha = Some("AB12".into());
        let d = decide_next(&frame, &set, &goal);
        assert!(matches!(
            d.intent,
            NextIntent::Navigate { ref label, .. } if label.eq_ignore_ascii_case("A")
        ));
    }

    #[test]
    fn captcha_digit_key_ignores_date_substring() {
        // Regression: labels_match_exact("2026-09-22 周二", "6") is true via contains.
        // Soft keys must be KeyboardKey (bottom band), never body date OCR.
        let mut frame = snap(Page::ConfirmOrder);
        frame.keyboard_open = true;
        frame.input_chars = String::new();
        let set = cache(&[
            ("验证码", 40, 400),
            ("完成", 300, 620),
            ("取消", 40, 620),
            ("2026-09-22 周二", 120, 80),
            ("6", 80, 700),
            ("E", 120, 700),
            ("A", 160, 700),
            ("X", 200, 700),
        ]);
        let mut goal = TaskGoal::default();
        goal.recognized_captcha = Some("6EAX".into());
        let d = decide_next(&frame, &set, &goal);
        assert!(
            matches!(
                d.intent,
                NextIntent::Navigate { ref label, .. } if label == "6"
            ),
            "expected soft key 6, not date OCR; got {:?}",
            d.intent
        );
        // Preview: remaining keys + 完成 all marked.
        assert!(
            d.evidence_boxes.len() >= 5,
            "expected 4 keys + 完成, got {}",
            d.evidence_boxes.len()
        );
    }

    #[test]
    fn captcha_keyboard_preview_keeps_full_code_when_partially_filled() {
        let mut frame = snap(Page::ConfirmOrder);
        frame.keyboard_open = true;
        frame.input_chars = "PL".into(); // 2/4 filled — still preview all four + 完成
        let set = cache(&[
            ("验证码", 40, 400),
            ("完成", 300, 620),
            ("取消", 40, 620),
            ("P", 80, 700),
            ("L", 120, 700),
            ("R", 160, 700),
            ("A", 200, 700),
        ]);
        let mut goal = TaskGoal::default();
        goal.recognized_captcha = Some("PLRA".into());
        let d = decide_next(&frame, &set, &goal);
        assert_eq!(d.evidence_boxes.len(), 5, "full PLRA + 完成");
        assert!(matches!(
            d.intent,
            NextIntent::Navigate { ref label, .. } if label.eq_ignore_ascii_case("R")
        ));
    }

    #[test]
    fn captcha_keyboard_plan_marks_all_keys_and_done() {
        let mut frame = snap(Page::ConfirmOrder);
        frame.keyboard_open = true;
        frame.input_chars = String::new();
        let set = cache(&[
            ("验证码", 40, 400),
            ("完成", 300, 620),
            ("取消", 40, 620),
            ("X", 80, 700),
            ("Y", 120, 700),
            ("P", 160, 700),
            ("A", 200, 700),
        ]);
        let mut goal = TaskGoal::default();
        goal.recognized_captcha = Some("XYPA".into());
        let d = decide_next(&frame, &set, &goal);
        assert_eq!(d.evidence_boxes.len(), 5);
        assert!(matches!(
            d.intent,
            NextIntent::Navigate { ref label, .. } if label.eq_ignore_ascii_case("X")
        ));
        assert_eq!(d.next_line, "下一步：输入验证码{XYPA}并点击完成");
    }

    #[test]
    fn captcha_partial_does_not_dismiss_keyboard() {
        let mut frame = snap(Page::ConfirmOrder);
        frame.keyboard_open = true;
        frame.input_chars = "X".into(); // 1/4 filled
        let set = cache(&[
            ("验证码", 40, 400),
            ("完成", 300, 520),
            ("取消", 40, 520),
            ("B", 120, 600),
        ]);
        let mut goal = TaskGoal::default();
        goal.recognized_captcha = Some("AB12".into());
        let d = decide_next(&frame, &set, &goal);
        assert!(
            matches!(
                d.intent,
                NextIntent::Navigate { ref label, .. } if label.eq_ignore_ascii_case("B")
            ),
            "expected next key B, got {:?}",
            d.intent
        );
        assert_eq!(d.now_line, "现在：确认预约，键盘已开");
        assert_eq!(d.next_line, "下一步：输入验证码{AB12}并点击完成");
    }

    #[test]
    fn captcha_keyboard_without_ocr_waits() {
        let mut frame = snap(Page::ConfirmOrder);
        frame.keyboard_open = true;
        frame.input_chars = String::new();
        let set = cache(&[
            ("验证码", 40, 400),
            ("完成", 300, 520),
            ("取消", 40, 520),
        ]);
        let d = decide_next(&frame, &set, &TaskGoal::default());
        assert!(matches!(
            d.intent,
            NextIntent::Wait {
                ref reason,
                ..
            } if reason == "CAPTCHA_OCR_PENDING"
        ));
        assert!(d.next_line.contains("识别验证码、输入四位并点击完成"));
    }

    fn captcha_hit(label: &str, kind: CandidateKind, actionable: bool, x0: i32) -> Candidate {
        let mut hit = Candidate {
            id: label.into(),
            bbox: BBox {
                x0,
                y0: 400,
                x1: x0 + 100,
                y1: 440,
            },
            kind,
            label: label.into(),
            score: 0.9,
            sources: vec![CandidateSource::Color],
            actionable,
            template_id: None,
            slot_meta: None,
            evidence: vec![],
            element_id: None,
            element_state: None,
            theme: None,
            visual_bbox: None,
            hit_bbox: None,
        };
        hit.visual_bbox = Some(hit.bbox.clone());
        hit.hit_bbox = Some(hit.bbox.clone());
        hit
    }

    #[test]
    fn captcha_keyboard_closed_without_code_refreshes_tile() {
        // No readable 4-char code → refresh tile before opening the keyboard.
        let frame = snap(Page::ConfirmOrder);
        let set = CandidateSet {
            items: vec![
                captcha_hit("captcha_image", CandidateKind::Other, false, 80),
                captcha_hit("captcha_input", CandidateKind::CaptchaHit, true, 200),
            ],
            dedup_iou: 0.3,
        };
        let d = decide_next(&frame, &set, &TaskGoal::default());
        assert!(
            matches!(
                d.intent,
                NextIntent::Navigate { ref label, .. } if label == "captcha_image"
            ),
            "expected refresh tile, got {:?}",
            d.intent
        );
        assert_eq!(d.next_line, "下一步：刷新验证码图");
    }

    #[test]
    fn captcha_keyboard_closed_with_code_opens_input() {
        let frame = snap(Page::ConfirmOrder);
        let set = CandidateSet {
            items: vec![
                captcha_hit("captcha_image", CandidateKind::Other, false, 80),
                captcha_hit("captcha_input", CandidateKind::CaptchaHit, true, 200),
            ],
            dedup_iou: 0.3,
        };
        let mut goal = TaskGoal::default();
        goal.recognized_captcha = Some("DXS9".into());
        let d = decide_next(&frame, &set, &goal);
        assert!(
            matches!(
                d.intent,
                NextIntent::Navigate { ref label, .. } if label == "captcha_input"
            ),
            "expected open keyboard, got {:?}",
            d.intent
        );
        assert_eq!(d.next_line, "下一步：点击输入区域唤起键盘");
    }

    #[test]
    fn captcha_keyboard_closed_no_tile_falls_back_to_input() {
        // Tile not localized — still open input (keyboard path can refresh later).
        let frame = snap(Page::ConfirmOrder);
        let set = CandidateSet {
            items: vec![captcha_hit(
                "captcha_input",
                CandidateKind::CaptchaHit,
                true,
                200,
            )],
            dedup_iou: 0.3,
        };
        let d = decide_next(&frame, &set, &TaskGoal::default());
        assert!(
            matches!(
                d.intent,
                NextIntent::Navigate { ref label, .. } if label == "captcha_input"
            ),
            "expected open keyboard fallback, got {:?}",
            d.intent
        );
    }

    #[test]
    fn captcha_full_then_done_then_submit() {
        let mut frame = snap(Page::ConfirmOrder);
        frame.keyboard_open = true;
        frame.input_chars = "XXXX".into();
        let set = cache(&[
            ("验证码", 40, 400),
            ("完成", 300, 520),
            ("取消", 40, 520),
        ]);
        let mut goal = TaskGoal::default();
        goal.recognized_captcha = Some("AB12".into());
        let d = decide_next(&frame, &set, &goal);
        assert!(matches!(
            d.intent,
            NextIntent::Navigate { ref label, .. } if label == "完成"
        ));

        frame.keyboard_open = false;
        let set2 = cache(&[("验证码", 40, 400), ("确定预约", 40, 700)]);
        let d2 = decide_next(&frame, &set2, &goal);
        assert!(matches!(
            d2.intent,
            NextIntent::Navigate { ref label, .. } if label == "确定预约"
        ));
    }

    #[test]
    fn each_page_names_the_forward_step() {
        let goal = goal_venue();
        let rows = [
            (
                Page::WechatMain,
                shell("discover_compass_inactive_dark", "发现"),
                "打开发现",
            ),
            (
                Page::WechatMpSearch,
                cache(&[("厦大体育", 80, 160)]),
                "打开厦大体育",
            ),
            (
                Page::XmuLoginGate,
                cache(&[("立即登录", 70, 480)]),
                "打开立即登录",
            ),
            (Page::XmuWxAuthorize, cache(&[]), "协议勾选框"),
            (
                Page::XmuWxAuthorize,
                cache(&[("阅读并同意用户协议", 80, 400)]),
                "勾选协议",
            ),
            (
                Page::XmuWxAuthorize,
                cache(&[("授权登录", 80, 520)]),
                "授权登录",
            ),
            (Page::XmuAuthWaiting, cache(&[]), "等待"),
            (
                Page::Home,
                cache(&[("场馆预约", 80, 280), ("发现", 8, 8)]),
                "打开场馆预约",
            ),
            (
                Page::VenueSportList,
                cache(&[("乒乓球", 80, 200)]),
                "点乒乓球",
            ),
            (
                Page::VenueList,
                cache(&[("翔安校区乒乓球室", 80, 200)]),
                "点翔安校区乒乓球室",
            ),
            (Page::MyReservations, cache(&[]), "已在结果页"),
            (Page::Success, cache(&[]), "已在结果页"),
            (Page::OrderDetail, cache(&[]), "不在预约路径"),
            (Page::CancelModal, cache(&[]), "不在预约路径"),
        ];
        for (page, set, needle) in rows {
            let d = decide_next(&snap(page), &set, &goal);
            let blob = format!("{} {}", d.now_line, d.next_line);
            assert!(blob.contains(needle), "{page:?} => {blob}");
            if !matches!(
                page,
                Page::WechatMain
                    | Page::WechatDiscover
                    | Page::WechatMpPanel
                    | Page::WechatMpSearch
                    | Page::Unknown
            ) {
                assert!(!d.next_line.contains("打开发现"), "{page:?} {blob}");
            }
        }

        let mut cell = shell("slot", "19:30").items.remove(0);
        cell.kind = CandidateKind::SlotCell;
        cell.template_id = None;
        cell.label = "19:30".into();
        cell.slot_meta = Some(crate::candidates::SlotMeta {
            start_time: "19:30".into(),
            court_index: Some(1),
            court_label: None,
            state: crate::slots::SlotState::Available,
        });
        let d = decide_next(
            &snap(Page::SelectSlot),
            &CandidateSet {
                items: vec![cell],
                dedup_iou: 0.3,
            },
            &goal,
        );
        assert!(d.next_line.contains("点可约格"), "{}", d.next_line);
    }
}

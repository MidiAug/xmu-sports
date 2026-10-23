//! Shadow Mode: observe-only recognition. Never produces GuardedAction or calls input.

use serde::{Deserialize, Serialize};
use vision_contracts::{ActionPlan, ActionType, Idempotency};

use crate::candidates::{
    analyze_captcha_input, parse_candidates, CandidateKind, CandidateSet, ColorRules, ParseInput,
    PerceptionFeed,
};
use crate::classify::{apply_shell_page, classify_from_hits, ClassifyInput};
use crate::ocr::OcrRegionCache;
use crate::slots::{parse_time_label, SlotCandidate, SlotState};
use crate::state::{Overlay, Page, SlotCta, UiSnapshot};

/// Default confidence floor — below this, page forced to Unknown / plan refused.
pub const DEFAULT_MIN_CONFIDENCE: f32 = 0.55;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateBox {
    pub label: String,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub score: f32,
    /// Whether this box would be eligible for a future Guarded click (shadow never clicks).
    pub actionable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShadowTiming {
    pub frame_ms: u64,
    /// Total OCR time (`ocr_full_ms + ocr_roi_ms`) for backward compatibility.
    pub ocr_ms: u64,
    /// Full-frame OCR wall time (ms).
    #[serde(default)]
    pub ocr_full_ms: u64,
    /// Region / crop OCR wall time (ms).
    #[serde(default)]
    pub ocr_roi_ms: u64,
    pub classify_ms: u64,
    /// Candidate fusion inside observe (templates / color / OCR attach).
    pub candidates_ms: u64,
    pub total_ms: u64,
    pub rss_bytes: Option<u64>,
    pub cpu_pct: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowObserveResult {
    pub snapshot: UiSnapshot,
    pub slots: Vec<SlotCandidate>,
    pub candidate_set: CandidateSet,
    pub candidate_boxes: Vec<CandidateBox>,
    /// Present when a *proposed* plan exists; Stop plans still set this with refuse semantics.
    pub action_plan: ActionPlan,
    pub refuse_reason: Option<String>,
    pub timing: ShadowTiming,
    /// Hard invariant for Shadow Mode.
    pub input_invoked: bool,
}

pub struct ShadowObserveInput<'a> {
    pub run_id: &'a str,
    pub lease_id: &'a str,
    pub generation: u64,
    pub frame_id: &'a str,
    pub frame_w: i32,
    pub frame_h: i32,
    pub ocr: &'a OcrRegionCache,
    pub title_hint: String,
    pub min_confidence: f32,
    pub target_slot_time: Option<&'a str>,
    pub timing: ShadowTiming,
    pub perception: PerceptionFeed<'a>,
}

/// Time-label OCR hits for diagnostics only.
///
/// **Not** court×time cell localization. Never marks [`SlotState::Available`] from
/// page-wide「可预约」legend alone — that false Available was feeding unsafe Click plans.
pub fn slots_from_ocr(ocr: &OcrRegionCache, frame_h: i32) -> Vec<SlotCandidate> {
    let mut out = Vec::new();
    for hit in ocr.hits() {
        if hit.score < 0.5 {
            continue;
        }
        let Some(start) = parse_time_label(&hit.text) else {
            continue;
        };
        // Time column usually mid-body; skip chrome.
        if hit.y < (frame_h as f32 * 0.12) as i32 {
            continue;
        }
        out.push(SlotCandidate {
            start_time: start,
            end_time: String::new(),
            court_index: None,
            court_label: None,
            // Explicit: unlocalized time text is not a bookable cell.
            state: SlotState::Unavailable,
            click_x: hit.cx(),
            click_y: hit.cy(),
            evidence: vec![
                format!("ocr:{}", hit.text),
                "not_localized_court_x_time_cell".into(),
            ],
        });
    }
    out
}

fn notice_overlay(ocr: &OcrRegionCache) -> bool {
    ocr.find_text("提示", 0.5).is_some()
        && ocr.find_text("确定", 0.5).is_some()
        && ocr.find_text("取消", 0.5).is_some()
}

fn slot_cta_from_ocr(ocr: &OcrRegionCache, has_mint: bool) -> SlotCta {
    // Disabled hint wins — even if OCR also glimpses 确认预约 chrome.
    if ocr.find_text("请先选择场次", 0.5).is_some() {
        return SlotCta::NeedSelect;
    }
    // Bottom CTA text replaced the disabled hint → selection armed (同授权登录灰→蓝).
    if ocr.find_text("确认预约", 0.5).is_some() {
        return SlotCta::Ready;
    }
    if has_mint {
        SlotCta::NeedSelect
    } else {
        SlotCta::Unknown
    }
}

fn stop_plan(
    run_id: &str,
    lease_id: &str,
    generation: u64,
    frame_id: &str,
    source_capture_pixel_size: vision_contracts::SizeI,
    snap: &UiSnapshot,
    reason: &str,
    created_monotonic_ms: u64,
) -> ActionPlan {
    ActionPlan {
        action_plan_id: format!("ap-{frame_id}-stop"),
        run_id: run_id.to_string(),
        lease_id: lease_id.to_string(),
        generation,
        frame_id: frame_id.to_string(),
        page_body_fingerprint: snap.body_fingerprint.clone(),
        source_capture_pixel_size,
        anchor_box: None,
        action_type: ActionType::Stop,
        confidence: snap.confidence,
        expected_transition: reason.to_string(),
        created_monotonic_ms,
        ttl_ms: 0,
        idempotency: Idempotency::Idempotent,
        text: Some(reason.to_string()),
    }
}

/// Observe one frame: classify → diagnostic candidates → Stop plan.
/// Never invokes input. Never emits Click from unlocalized slot heuristics
/// (those must not feed Phase 5).
pub fn observe_shadow(input: ShadowObserveInput<'_>) -> ShadowObserveResult {
    let t0 = std::time::Instant::now();
    let rules = ColorRules::default();
    let t_cand = std::time::Instant::now();
    let candidate_set = parse_candidates(&ParseInput {
        frame_w: input.frame_w,
        frame_h: input.frame_h,
        ocr: input.ocr,
        pixels_bgra: input.perception.pixels_bgra,
        detections: input.perception.detections,
        templates: input.perception.templates,
        rules: &rules,
    });
    let candidates_ms = t_cand.elapsed().as_millis() as u64;
    let localized: Vec<SlotCandidate> = candidate_set
        .items
        .iter()
        .filter(|c| c.kind == CandidateKind::SlotCell)
        .filter_map(|c| {
            let meta = c.slot_meta.as_ref()?;
            Some(SlotCandidate {
                start_time: meta.start_time.clone(),
                end_time: String::new(),
                court_index: meta.court_index,
                court_label: meta.court_label.clone(),
                state: meta.state,
                click_x: c.bbox.cx(),
                click_y: c.bbox.cy(),
                evidence: c.evidence.clone(),
            })
        })
        .collect();
    let has_localized = localized.iter().any(|s| s.state == SlotState::Available);
    let slots = if has_localized {
        localized
    } else {
        slots_from_ocr(input.ocr, input.frame_h)
    };
    let has_mint = has_localized;
    let slot_cta = slot_cta_from_ocr(input.ocr, has_mint);

    let captcha_state = input
        .perception
        .pixels_bgra
        .and_then(|px| analyze_captcha_input(px, input.frame_w, input.frame_h, input.ocr));
    let captcha_input_chars = captcha_state
        .as_ref()
        .map(|s| s.typed_proxy.clone())
        .unwrap_or_default();
    let keyboard_open = candidate_set.has_label("完成")
        && candidate_set.has_label("取消")
        && (candidate_set.has_label("验证码")
            || candidate_set
                .items
                .iter()
                .any(|c| c.kind == CandidateKind::CaptchaHit));

    let mut snap = classify_from_hits(&ClassifyInput {
        frame_w: input.frame_w,
        frame_h: input.frame_h,
        ocr: input.ocr,
        title_hint: input.title_hint.clone(),
        has_mint_slots: has_mint,
        captcha_input_chars,
        keyboard_open,
        toast_kind: None,
        slot_cta,
    });

    // Generic notice popup: force stop path even if page text partially matched.
    if notice_overlay(input.ocr) && snap.page != Page::CancelModal {
        if snap.page == Page::Unknown || snap.confidence < 0.9 {
            snap.overlay = Overlay::Notice;
            snap.evidence.push("overlay:提示+确定+取消".into());
            if snap.confidence < input.min_confidence {
                snap.confidence = 0.6;
            }
        } else {
            snap.overlay = Overlay::Notice;
            snap.evidence.push("overlay:提示+确定+取消".into());
        }
    }

    let classify_ms = t0.elapsed().as_millis() as u64;

    // Low confidence / unmatched → Unknown, never guess buttons.
    if snap.confidence < input.min_confidence {
        snap.page = Page::Unknown;
        snap.confidence = 0.0;
        snap.evidence
            .push(format!("low_confidence:<{}", input.min_confidence));
    }
    if !crate::state::title_is_xmu_miniprogram(&input.title_hint) {
        apply_shell_page(
            &mut snap,
            candidate_set.shell_state("发现"),
            candidate_set.shell_state("小程序"),
        );
    }
    // The mini-program window is not the WeChat shell. A home-grid tile
    // (purple disc + caption) is enough to name the page; do not look for 发现.
    if crate::state::title_is_xmu_miniprogram(&input.title_hint)
        && snap.page == Page::Unknown
        && candidate_set
            .items
            .iter()
            .any(|c| c.element_id.as_deref() == Some(crate::candidates::ELEMENT_VENUE_BOOKING))
    {
        snap.page = Page::Home;
        snap.confidence = snap.confidence.max(0.86);
        snap.evidence.push("tile:venue_booking".into());
        snap.body_fingerprint = snap.evidence.join("|");
    }
    if crate::state::title_is_xmu_miniprogram(&input.title_hint)
        && snap.page == Page::Unknown
        && candidate_set
            .items
            .iter()
            .any(|c| c.element_id.as_deref() == Some(crate::candidates::ELEMENT_SPORT_ICON))
    {
        snap.page = Page::VenueSportList;
        snap.confidence = snap.confidence.max(0.84);
        snap.evidence.push("tile:sport_icon".into());
        snap.body_fingerprint = snap.evidence.join("|");
    }
    if candidate_set
        .items
        .iter()
        .any(|c| c.element_id.as_deref() == Some(crate::candidates::ELEMENT_NOTICE_ACK))
    {
        snap.page = Page::BookingNotice;
        snap.overlay = Overlay::None;
        snap.confidence = snap.confidence.max(0.88);
        snap.evidence.push("tile:notice_ack".into());
        snap.body_fingerprint = snap.evidence.join("|");
    }

    let mut boxes: Vec<CandidateBox> = candidate_set
        .items
        .iter()
        .take(80)
        .map(|c| CandidateBox {
            label: if c.label.is_empty() {
                c.kind.as_str().to_string()
            } else {
                c.label.clone()
            },
            x0: c.bbox.x0,
            y0: c.bbox.y0,
            x1: c.bbox.x1,
            y1: c.bbox.y1,
            score: c.score,
            actionable: c.actionable,
        })
        .collect();

    // Highlight known auth CTAs for human page-map review — still not Phase-5 safe.
    for needle in ["立即登录", "授权登录", "统一身份认证", "取消登录"] {
        if let Some(h) = input.ocr.find_text(needle, 0.5) {
            boxes.push(CandidateBox {
                label: format!("key:{needle}"),
                x0: h.x,
                y0: h.y,
                x1: h.x + h.w,
                y1: h.y + h.h,
                score: h.score,
                actionable: false,
            });
        }
    }

    let mut timing = input.timing;
    timing.candidates_ms = candidates_ms;
    timing.classify_ms = classify_ms;
    timing.total_ms = timing.frame_ms + timing.ocr_ms + classify_ms;

    let empty_slots_hint = input.ocr.find_text("暂无场次", 0.45).is_some()
        || input.ocr.find_text("无场次", 0.45).is_some()
        || input.ocr.find_text("已约满", 0.45).is_some();

    // Always Stop until page-map acceptance + real cell localization unlock Phase 5.
    let refuse = if matches!(snap.overlay, Overlay::Notice | Overlay::Modal) {
        Some("OVERLAY_BLOCKS_ACTION".to_string())
    } else if snap.page == Page::Unknown {
        Some("UNKNOWN_PAGE".to_string())
    } else if snap.page == Page::SelectSlot && empty_slots_hint && !has_localized {
        Some("NO_AVAILABLE_SLOTS".to_string())
    } else if snap.page == Page::SelectSlot && !has_localized {
        Some("SLOT_GRID_NOT_LOCALIZED".to_string())
    } else if snap.page == Page::SelectSlot && has_localized {
        Some("SHADOW_SLOT_READY".to_string())
    } else if snap.page == Page::ConfirmOrder {
        Some("SHADOW_NO_SUBMIT".to_string())
    } else {
        Some("OBSERVE_ONLY_PENDING_PAGE_MAP".to_string())
    };

    let reason = refuse.clone().unwrap_or_else(|| "STOP".into());
    ShadowObserveResult {
        snapshot: snap.clone(),
        slots,
        candidate_set,
        candidate_boxes: boxes,
        action_plan: stop_plan(
            input.run_id,
            input.lease_id,
            input.generation,
            input.frame_id,
            vision_contracts::SizeI {
                width: input.frame_w.max(0) as u32,
                height: input.frame_h.max(0) as u32,
            },
            &snap,
            &reason,
            vision_contracts::monotonic_now_ms(),
        ),
        refuse_reason: refuse,
        timing,
        input_invoked: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::OcrHit;

    fn cache(texts: &[(&str, i32, i32)]) -> OcrRegionCache {
        OcrRegionCache::from_hits(
            texts
                .iter()
                .map(|(t, x, y)| OcrHit {
                    text: (*t).into(),
                    x: *x,
                    y: *y,
                    w: 40,
                    h: 16,
                    score: 0.9,
                })
                .collect(),
        )
    }

    #[test]
    fn unknown_wechat_chat_refuses() {
        let ocr = cache(&[("发送", 400, 600), ("文件传输助手", 40, 80)]);
        let r = observe_shadow(ShadowObserveInput {
            run_id: "r1",
            lease_id: "l1",
            generation: 1,
            frame_id: "f1",
            frame_w: 800,
            frame_h: 600,
            ocr: &ocr,
            title_hint: "微信".into(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            target_slot_time: None,
            timing: ShadowTiming::default(),
            perception: PerceptionFeed::default(),
        });
        assert_eq!(r.snapshot.page, Page::Unknown);
        assert_eq!(r.refuse_reason.as_deref(), Some("UNKNOWN_PAGE"));
        assert!(!r.input_invoked);
        assert_eq!(r.action_plan.action_type, ActionType::Stop);
    }

    #[test]
    fn miniprogram_home_tile_names_home_not_discover() {
        let (fw, fh) = (514i32, 780i32);
        let ocr = cache(&[("场馆预约", 180, 420)]);
        let mut bgra = vec![255u8; (fw * fh * 4) as usize];
        // Caption is 40x16 in this helper. Disc sits just above it.
        for y in 376..416 {
            for x in 180..220 {
                let i = ((y * fw + x) * 4) as usize;
                bgra[i] = 251;
                bgra[i + 1] = 120;
                bgra[i + 2] = 178;
                bgra[i + 3] = 255;
            }
        }
        let r = observe_shadow(ShadowObserveInput {
            run_id: "r-home",
            lease_id: "l1",
            generation: 1,
            frame_id: "f1",
            frame_w: fw,
            frame_h: fh,
            ocr: &ocr,
            title_hint: "厦大体育".into(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            target_slot_time: None,
            timing: ShadowTiming::default(),
            perception: PerceptionFeed {
                pixels_bgra: Some(&bgra),
                detections: &[],
                templates: &[],
            },
        });
        assert_eq!(r.snapshot.page, Page::Home);
        let d = crate::decide_next(&r.snapshot, &r.candidate_set, &crate::TaskGoal::default());
        assert_eq!(d.now_line, "现在：体育首页");
        assert!(d.next_line.contains("场馆预约"));
        assert!(!d.next_line.contains("发现"));
        match d.intent {
            crate::NextIntent::Navigate { ref label, anchor, .. } => {
                assert_eq!(label, "场馆预约");
                assert!(anchor.y0 < 420, "click the icon, not only the caption");
            }
            other => panic!("expected navigate, got {other:?}"),
        }
    }

    #[test]
    fn booking_notice_clicks_ack_not_venue() {
        let (fw, fh) = (514i32, 780i32);
        let ocr = OcrRegionCache::from_hits(vec![
            OcrHit {
                text: "【预约须知】".into(),
                x: 160,
                y: 180,
                w: 120,
                h: 22,
                score: 0.9,
            },
            OcrHit {
                text: "我知道了".into(),
                x: 200,
                y: 520,
                w: 64,
                h: 20,
                score: 0.9,
            },
        ]);
        let mut bgra = vec![255u8; (fw * fh * 4) as usize];
        for y in 500..560 {
            for x in 160..320 {
                let i = ((y * fw + x) * 4) as usize;
                bgra[i] = 136;
                bgra[i + 1] = 60;
                bgra[i + 2] = 0;
                bgra[i + 3] = 255;
            }
        }
        let r = observe_shadow(ShadowObserveInput {
            run_id: "r-notice",
            lease_id: "l1",
            generation: 1,
            frame_id: "f1",
            frame_w: fw,
            frame_h: fh,
            ocr: &ocr,
            title_hint: "厦大体育".into(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            target_slot_time: None,
            timing: ShadowTiming::default(),
            perception: PerceptionFeed {
                pixels_bgra: Some(&bgra),
                detections: &[],
                templates: &[],
            },
        });
        assert_eq!(r.snapshot.page, Page::BookingNotice);
        let d = crate::decide_next(&r.snapshot, &r.candidate_set, &crate::TaskGoal::default());
        assert_eq!(d.now_line, "现在：预约须知");
        assert!(d.next_line.contains("我知道了"));
        assert!(!d.next_line.contains("场馆预约"));
    }

    #[test]
    fn sport_tile_clicks_badminton_not_my_reservations() {
        let (fw, fh) = (524i32, 780i32);
        let ocr = cache(&[("我的预约", 340, 36), ("羽毛球", 160, 280)]);
        let mut bgra = vec![255u8; (fw * fh * 4) as usize];
        for y in 228..276 {
            for x in 160..208 {
                let i = ((y * fw + x) * 4) as usize;
                bgra[i] = 30;
                bgra[i + 1] = 28;
                bgra[i + 2] = 24;
                bgra[i + 3] = 255;
            }
        }
        let r = observe_shadow(ShadowObserveInput {
            run_id: "r-sport",
            lease_id: "l1",
            generation: 1,
            frame_id: "f1",
            frame_w: fw,
            frame_h: fh,
            ocr: &ocr,
            title_hint: "厦大体育".into(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            target_slot_time: None,
            timing: ShadowTiming::default(),
            perception: PerceptionFeed {
                pixels_bgra: Some(&bgra),
                detections: &[],
                templates: &[],
            },
        });
        assert_eq!(r.snapshot.page, Page::VenueSportList);
        let mut goal = crate::TaskGoal::default();
        goal.sport = Some("羽毛球".into());
        let d = crate::decide_next(&r.snapshot, &r.candidate_set, &goal);
        assert_eq!(d.now_line, "现在：运动列表");
        assert!(d.next_line.contains("点羽毛球"));
        assert!(!d.now_line.contains("我的预约"));
        match d.intent {
            crate::NextIntent::Navigate { ref label, anchor, .. } => {
                assert_eq!(label, "羽毛球");
                assert!(anchor.y0 < 280);
            }
            other => panic!("expected sport navigate, got {other:?}"),
        }
    }

    #[test]
    fn select_slot_refuses_unlocalized_grid() {
        let ocr = cache(&[
            ("选择场次", 20, 40),
            ("可预约", 20, 90),
            ("不可选", 80, 90),
            ("19:00", 20, 200),
        ]);
        let r = observe_shadow(ShadowObserveInput {
            run_id: "r1",
            lease_id: "l1",
            generation: 1,
            frame_id: "f1",
            frame_w: 420,
            frame_h: 760,
            ocr: &ocr,
            title_hint: "场馆".into(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            target_slot_time: Some("19:00"),
            timing: ShadowTiming::default(),
            perception: PerceptionFeed::default(),
        });
        assert_eq!(r.snapshot.page, Page::SelectSlot);
        assert_eq!(r.refuse_reason.as_deref(), Some("SLOT_GRID_NOT_LOCALIZED"));
        assert_eq!(r.action_plan.action_type, ActionType::Stop);
        assert!(r.action_plan.anchor_box.is_none());
        assert!(!r.input_invoked);
    }

    #[test]
    fn empty_slots_refuse() {
        let ocr = cache(&[
            ("选择场次", 20, 40),
            ("可预约", 20, 90),
            ("不可选", 80, 90),
            ("暂无场次", 100, 300),
        ]);
        let r = observe_shadow(ShadowObserveInput {
            run_id: "r1",
            lease_id: "l1",
            generation: 1,
            frame_id: "f1",
            frame_w: 420,
            frame_h: 760,
            ocr: &ocr,
            title_hint: "场馆".into(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            target_slot_time: None,
            timing: ShadowTiming::default(),
            perception: PerceptionFeed::default(),
        });
        assert_eq!(r.snapshot.page, Page::SelectSlot);
        assert_eq!(r.refuse_reason.as_deref(), Some("NO_AVAILABLE_SLOTS"));
    }

    #[test]
    fn popup_notice_blocks() {
        let ocr = cache(&[
            ("提示", 180, 280),
            ("确定", 120, 360),
            ("取消", 240, 360),
            ("网络异常", 160, 310),
        ]);
        let r = observe_shadow(ShadowObserveInput {
            run_id: "r1",
            lease_id: "l1",
            generation: 1,
            frame_id: "f1",
            frame_w: 420,
            frame_h: 760,
            ocr: &ocr,
            title_hint: String::new(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            target_slot_time: None,
            timing: ShadowTiming::default(),
            perception: PerceptionFeed::default(),
        });
        assert!(matches!(r.snapshot.overlay, Overlay::Notice) || r.snapshot.page == Page::Unknown);
        assert!(!r.input_invoked);
        assert!(
            r.refuse_reason.as_deref() == Some("OVERLAY_BLOCKS_ACTION")
                || r.refuse_reason.as_deref() == Some("UNKNOWN_PAGE")
                || r.refuse_reason.as_deref() == Some("NO_SAFE_CLICK_TARGET")
        );
    }
}

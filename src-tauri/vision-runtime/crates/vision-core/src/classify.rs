use crate::candidates::ShellVisualState;
use crate::ocr::OcrRegionCache;
use crate::state::{CaptchaPhase, Overlay, Page, SlotCta, UiSnapshot};

pub struct ClassifyInput<'a> {
    pub frame_w: i32,
    pub frame_h: i32,
    pub ocr: &'a OcrRegionCache,
    pub title_hint: String,
    pub has_mint_slots: bool,
    pub captcha_input_chars: String,
    pub keyboard_open: bool,
    pub toast_kind: Option<&'a str>, // "need_input" | "wrong"
    pub slot_cta: SlotCta,
}

fn fingerprint(parts: &[&str]) -> String {
    parts.join("|")
}

/// Second body feature beside「验证码」. CTA may be hidden under the soft keyboard.
fn confirm_order_second_feature(ocr: &OcrRegionCache) -> bool {
    ocr.find_text("确定预约", 0.5).is_some()
        || ocr.find_text("订单信息", 0.5).is_some()
        || ocr.find_text("确认预约", 0.5).is_some()
        || ocr.find_text("场馆信息", 0.5).is_some()
        || ocr.find_text("预约类型", 0.5).is_some()
        || ocr.find_text("订单价格", 0.5).is_some()
        || ocr.find_text("预约项目", 0.5).is_some()
}

fn snap(
    page: Page,
    title: &str,
    evidence: &[&str],
    confidence: f32,
    overlay: Overlay,
    extra: impl FnOnce(&mut UiSnapshot),
) -> UiSnapshot {
    let mut s = UiSnapshot {
        page,
        title_hint: title.to_string(),
        evidence: evidence.iter().map(|e| (*e).to_string()).collect(),
        confidence,
        overlay,
        body_fingerprint: fingerprint(evidence),
        ..UiSnapshot::default()
    };
    extra(&mut s);
    s
}

/// Classify page from OCR hits + light features (no image / platform deps).
/// Mirrors Python body-fingerprint rules at a text-evidence level.
pub fn classify_from_hits(input: &ClassifyInput<'_>) -> UiSnapshot {
    let ocr = input.ocr;
    let title = if input.title_hint.is_empty() {
        "untitled"
    } else {
        input.title_hint.as_str()
    };

    if ocr.find_text("你确定要取消预约吗", 0.5).is_some()
        || ocr.find_text("确定要取消预约吗", 0.5).is_some()
    {
        return snap(
            Page::CancelModal,
            title,
            &["modal:cancel_confirm"],
            0.95,
            Overlay::Modal,
            |_| {},
        );
    }

    if ocr.find_text("预约成功", 0.5).is_some() {
        return snap(
            Page::Success,
            title,
            &["text:预约成功"],
            0.95,
            Overlay::None,
            |_| {},
        );
    }

    // --- XMU mini-program auth gate (before sports home) ---
    if ocr.find_text("立即登录", 0.5).is_some()
        && (ocr.find_text("统一身份认证", 0.45).is_some()
            || ocr.find_text("厦门大学", 0.45).is_some())
    {
        return snap(
            Page::XmuLoginGate,
            title,
            &["auth:立即登录", "auth:统一身份|厦大"],
            0.9,
            Overlay::None,
            |_| {},
        );
    }
    // 「请求中」盖在授权页上时，下一步是等，不是再点授权登录。
    if ocr.find_text("请求中", 0.5).is_some()
        || (ocr.find_text("等待中", 0.55).is_some()
            && ocr.find_text("立即登录", 0.5).is_none()
            && ocr.find_text("授权登录", 0.5).is_none())
    {
        return snap(
            Page::XmuAuthWaiting,
            title,
            &["auth:请求中|等待中"],
            0.8,
            Overlay::None,
            |_| {},
        );
    }
    if ocr.find_text("授权登录", 0.5).is_some()
        && (ocr.find_text("微信授权", 0.45).is_some()
            || ocr.find_text("用户协议", 0.45).is_some()
            || ocr.find_text("隐私政策", 0.45).is_some())
    {
        return snap(
            Page::XmuWxAuthorize,
            title,
            &["auth:授权登录", "auth:协议|微信授权"],
            0.9,
            Overlay::None,
            |_| {},
        );
    }

    // 预约须知盖在运动列表上。白字「我知道了」在小窗上经常读不到，标题先定页。
    if ocr.find_text("预约须知", 0.45).is_some() || ocr.find_text("须知", 0.45).is_some() {
        return snap(
            Page::BookingNotice,
            title,
            &["notice:预约须知"],
            0.9,
            Overlay::None,
            |_| {},
        );
    }

    let header_y = (input.frame_h as f32 * 0.16) as i32;
    let has_status = ocr.hits().iter().any(|h| {
        h.score >= 0.5
            && h.y >= header_y
            && !h.text.contains("我的预约")
            && (crate::ocr::labels_match_exact(&h.text, "已预约")
                || crate::ocr::labels_match_exact(&h.text, "已取消"))
    });
    // Body「已预约/已取消」is enough — footer (“已经到底”) OCR is flaky after submit,
    // and venue names with「校区」must not fall through to venue_list.
    if has_status {
        return snap(
            Page::MyReservations,
            title,
            &["list:已预约|已取消"],
            0.9,
            Overlay::None,
            |_| {},
        );
    }

    if ocr.find_text("预约详情", 0.5).is_some()
        || ocr.find_text("订单详情", 0.5).is_some()
        || (ocr.find_text("取消预约", 0.5).is_some()
            && (ocr.find_text("进场凭证", 0.5).is_some()
                || ocr.find_text("预约编号", 0.5).is_some()))
    {
        return snap(
            Page::OrderDetail,
            title,
            &["order:详情|凭证|编号"],
            0.88,
            Overlay::None,
            |_| {},
        );
    }

    // Soft keyboard covers bottom CTA「确定预约」; form labels / title still show.
    // Never fall through to venue_list just because the venue name contains「校区」.
    if ocr.find_text("验证码", 0.5).is_some() && confirm_order_second_feature(ocr) {
        let phase = match input.toast_kind {
            Some("need_input") => CaptchaPhase::ToastNeedInput,
            Some("wrong") => CaptchaPhase::ToastError,
            _ if input.keyboard_open => {
                let n = input.captcha_input_chars.len();
                if n >= 4 {
                    CaptchaPhase::KbOpenFull
                } else if n > 0 {
                    CaptchaPhase::KbOpenPartial
                } else {
                    CaptchaPhase::KbOpenEmpty
                }
            }
            _ if input.captcha_input_chars.len() >= 4 => CaptchaPhase::IdleFull,
            _ => CaptchaPhase::IdleEmpty,
        };
        let overlay = if input.keyboard_open {
            Overlay::Keyboard
        } else if matches!(
            phase,
            CaptchaPhase::ToastError | CaptchaPhase::ToastNeedInput
        ) {
            Overlay::Toast
        } else {
            Overlay::None
        };
        return snap(
            Page::ConfirmOrder,
            title,
            &["anchor:验证码", "cta:确定预约|订单字段|确认预约"],
            0.92,
            overlay,
            |s| {
                s.captcha = phase;
                s.keyboard_open = input.keyboard_open;
                s.input_chars = input.captcha_input_chars.clone();
                s.toast_error = phase == CaptchaPhase::ToastError;
                s.toast_need_input = phase == CaptchaPhase::ToastNeedInput;
            },
        );
    }

    if ocr.find_text("选择场次", 0.5).is_some()
        || (ocr.find_text("可预约", 0.5).is_some() && ocr.find_text("不可选", 0.5).is_some())
    {
        return snap(
            Page::SelectSlot,
            title,
            &["legend:可预约/不可选|选择场次"],
            0.9,
            Overlay::None,
            |s| {
                s.slot_cta = input.slot_cta;
                s.has_mint_slots = input.has_mint_slots;
            },
        );
    }

    if ocr.find_text("场馆详情", 0.5).is_some()
        || ((ocr.find_text("预定", 0.5).is_some() || ocr.find_text("暂无", 0.5).is_some())
            && (ocr.find_text("场馆简介", 0.5).is_some() || ocr.find_text("今天", 0.5).is_some()))
    {
        return snap(
            Page::VenueDetail,
            title,
            &["venue_detail"],
            0.85,
            Overlay::None,
            |_| {},
        );
    }

    // Nav 「场馆列表」 is the venue-list page. The sport grid's nav is 「场馆预约」;
    // a header 「场馆列表」 plus two sport captions is still the sport grid.
    let sports = [
        "乒乓球", "羽毛球", "篮球", "足球", "排球", "网球", "健身房", "瑜伽", "游泳", "匹克球",
    ];
    let sport_hits: Vec<_> = sports
        .iter()
        .filter(|s| ocr.body_has(s, 0.08, 0.75, input.frame_h))
        .collect();
    let sport_grid = sport_hits.len() >= 2 || ocr.find_text("所有场馆", 0.5).is_some();
    if sport_grid
        && ocr.find_text("选择场次", 0.5).is_none()
        && ocr.find_text("场馆详情", 0.5).is_none()
    {
        return snap(
            Page::VenueSportList,
            title,
            &["grid:sports"],
            0.85,
            Overlay::None,
            |_| {},
        );
    }

    let venue_named = ocr.hits().iter().any(|h| {
        let n = crate::ocr::normalize_ocr_text(&h.text);
        n.contains('校') && n.chars().count() >= 6
    });
    // 「翔安校区乒乓球室」also appears as 场馆信息 on confirm_order (esp. with keyboard up).
    // Never treat reservation-list rows (status badge) as venue_list.
    if (ocr.find_text("场馆列表", 0.5).is_some() || venue_named)
        && ocr.find_text("选择场次", 0.5).is_none()
        && ocr.find_text("场馆详情", 0.5).is_none()
        && ocr.find_text("所有场馆", 0.5).is_none()
        && ocr.find_text("验证码", 0.5).is_none()
        && ocr.find_text("场馆信息", 0.5).is_none()
        && !has_status
    {
        return snap(
            Page::VenueList,
            title,
            &["title:场馆列表"],
            0.86,
            Overlay::None,
            |_| {},
        );
    }

    // images/9：运动图标 +「所有场馆」。不是首页。
    if ocr.find_text("场馆预约", 0.5).is_some() && ocr.find_text("所有场馆", 0.5).is_some()
    {
        return snap(
            Page::VenueSportList,
            title,
            &["sport:场馆预约+所有场馆"],
            0.88,
            Overlay::None,
            |_| {},
        );
    }

    // images/8：首页图标「场馆预约」+ 底栏「首页」。没有「所有场馆」。
    if ocr.find_text("场馆预约", 0.5).is_some()
        && ocr.find_text("首页", 0.5).is_some()
        && ocr.find_text("所有场馆", 0.5).is_none()
    {
        return snap(
            Page::Home,
            title,
            &["home:场馆预约+首页"],
            0.88,
            Overlay::None,
            |_| {},
        );
    }

    let has_xmu_app =
        ocr.find_text("厦大体育", 0.5).is_some() || ocr.find_text("步道乐跑", 0.45).is_some();
    let has_mp_word = ocr.find_text("小程序", 0.5).is_some();
    let has_discover_list = ocr.find_text("朋友圈", 0.5).is_some()
        || ocr.find_text("视频号", 0.5).is_some()
        || ocr.find_text("搜一搜", 0.5).is_some();
    let has_mp_panel_chrome =
        ocr.find_text("最近使用", 0.45).is_some() || ocr.find_text("我的常用", 0.45).is_some();

    // Discover list first. 「搜一搜」lives here — never treat it as the MP search UI.
    if has_mp_word && has_discover_list && !has_mp_panel_chrome {
        return snap(
            Page::WechatDiscover,
            title,
            &["wechat:discover+小程序"],
            0.82,
            Overlay::None,
            |_| {},
        );
    }
    if has_mp_word && has_xmu_app {
        return snap(
            Page::WechatMpPanel,
            title,
            &["wechat:mp_panel+厦大体育"],
            0.84,
            Overlay::None,
            |_| {},
        );
    }
    if has_mp_word && has_mp_panel_chrome {
        return snap(
            Page::WechatMpPanel,
            title,
            &["wechat:mp_panel"],
            0.8,
            Overlay::None,
            |_| {},
        );
    }
    // Only the in-panel search surface — needs panel chrome, not bare 「搜索」/「搜一搜」.
    if has_mp_word
        && has_mp_panel_chrome
        && ocr.find_text("搜索", 0.5).is_some()
        && ocr.find_text("搜一搜", 0.5).is_none()
        && !has_xmu_app
    {
        return snap(
            Page::WechatMpSearch,
            title,
            &["wechat:mp_search"],
            0.72,
            Overlay::None,
            |_| {},
        );
    }
    if has_mp_word && (ocr.find_text("发现", 0.5).is_some() || has_discover_list) {
        return snap(
            Page::WechatDiscover,
            title,
            &["wechat:discover+小程序"],
            0.78,
            Overlay::None,
            |_| {},
        );
    }

    snap(
        Page::Unknown,
        title,
        &["unmatched"],
        0.0,
        Overlay::None,
        |_| {},
    )
}

/// Upgrade an unmatched frame using shell element state. Booking pages are left alone.
pub fn apply_shell_page(
    snap: &mut UiSnapshot,
    discover: Option<ShellVisualState>,
    miniprogram: Option<ShellVisualState>,
) {
    if snap.page != Page::Unknown {
        return;
    }
    // Mini-program Active only means the Discover list row is selected.
    // Without an Active compass, ignore a stray glyph hit on the chat list.
    if miniprogram == Some(ShellVisualState::Active) && discover == Some(ShellVisualState::Active) {
        snap.page = Page::WechatMpPanel;
        snap.confidence = snap.confidence.max(0.74);
        snap.evidence.push("shell:miniprogram_active".into());
    } else if discover == Some(ShellVisualState::Active) {
        snap.page = Page::WechatDiscover;
        snap.confidence = snap.confidence.max(0.72);
        snap.evidence.push("shell:discover_active".into());
    } else if discover == Some(ShellVisualState::Inactive) {
        snap.page = Page::WechatMain;
        snap.confidence = snap.confidence.max(0.7);
        snap.evidence.push("shell:discover_inactive".into());
    } else {
        return;
    }
    snap.body_fingerprint = snap.evidence.join("|");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::OcrHit;

    fn hits(texts: &[(&str, i32, i32)]) -> OcrRegionCache {
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
    fn classify_select_slot() {
        let ocr = hits(&[("选择场次", 20, 40), ("可预约", 20, 80), ("不可选", 80, 80)]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 400,
            frame_h: 700,
            ocr: &ocr,
            title_hint: "场馆".into(),
            has_mint_slots: true,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::NeedSelect,
        });
        assert_eq!(snap.page, Page::SelectSlot);
        assert!(snap.has_mint_slots);
        assert!(!snap.body_fingerprint.is_empty());
    }

    #[test]
    fn classify_confirm_requires_two_body_features() {
        let only_title = hits(&[("验证码", 10, 100)]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 400,
            frame_h: 700,
            ocr: &only_title,
            title_hint: "确认".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::Unknown);

        let both = hits(&[("验证码", 10, 100), ("确定预约", 200, 650)]);
        let snap2 = classify_from_hits(&ClassifyInput {
            frame_w: 400,
            frame_h: 700,
            ocr: &both,
            title_hint: "确认".into(),
            has_mint_slots: false,
            captcha_input_chars: "ab12".into(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap2.page, Page::ConfirmOrder);
        assert_eq!(snap2.captcha, CaptchaPhase::IdleFull);
    }

    /// Keyboard covers「确定预约」; venue name still OCR'd → must stay confirm_order.
    #[test]
    fn captcha_keyboard_is_not_venue_list() {
        let ocr = hits(&[
            ("确认预约", 180, 40),
            ("场馆信息", 40, 180),
            ("翔安校区乒乓球室", 200, 180),
            ("预约项目", 40, 220),
            ("乒乓球", 200, 220),
            ("订单价格", 40, 260),
            ("预约类型", 40, 300),
            ("验证码", 40, 380),
            // Soft keyboard glyphs — no「确定预约」
            ("完成", 360, 520),
            ("q", 40, 580),
            ("w", 80, 580),
        ]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 520,
            frame_h: 719,
            ocr: &ocr,
            title_hint: "厦大体育".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: true,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::ConfirmOrder);
        assert!(snap.keyboard_open);
        assert_eq!(snap.captcha, CaptchaPhase::KbOpenEmpty);
    }

    #[test]
    fn classify_xmu_login_gate() {
        let ocr = hits(&[
            ("厦门大学", 150, 48),
            ("立即登录", 70, 520),
            ("统一身份认证", 70, 580),
        ]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 390,
            frame_h: 780,
            ocr: &ocr,
            title_hint: String::new(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::XmuLoginGate);
    }

    #[test]
    fn classify_xmu_wx_authorize() {
        let ocr = hits(&[
            ("微信授权", 80, 160),
            ("用户协议", 200, 420),
            ("授权登录", 70, 480),
        ]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 390,
            frame_h: 780,
            ocr: &ocr,
            title_hint: "登录".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::XmuWxAuthorize);
    }

    #[test]
    fn requesting_overlay_is_waiting_not_authorize() {
        let ocr = hits(&[
            ("微信授权", 80, 160),
            ("授权登录", 70, 480),
            ("请求中", 180, 400),
        ]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 390,
            frame_h: 780,
            ocr: &ocr,
            title_hint: "登录".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::XmuAuthWaiting);
    }

    #[test]
    fn sport_page_is_not_home() {
        let ocr = hits(&[
            ("场馆预约", 160, 80),
            ("我的预约", 300, 80),
            ("乒乓球", 80, 180),
            ("羽毛球", 160, 180),
            ("所有场馆", 40, 360),
        ]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 390,
            frame_h: 780,
            ocr: &ocr,
            title_hint: String::new(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::VenueSportList);
    }

    #[test]
    fn header_my_reservations_is_not_the_result_list() {
        let ocr = hits(&[
            ("场馆列表", 180, 16),
            ("我的预约", 300, 36),
            ("已预约", 300, 20),
            ("羽毛球", 80, 220),
            ("乒乓球", 160, 220),
            ("暂无预约", 40, 700),
        ]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 524,
            frame_h: 780,
            ocr: &ocr,
            title_hint: "厦大体育".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::VenueSportList);
    }

    #[test]
    fn reservation_rows_with_campus_name_are_my_reservations() {
        // After submit: list shows「已预约」+ venue「…校区…」; footer OCR often missing.
        let ocr = hits(&[
            ("我的预约", 200, 36),
            ("已预约", 80, 220),
            ("翔安校区乒乓球室", 80, 250),
            ("09-24 19:30-21:00", 80, 280),
        ]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 517,
            frame_h: 719,
            ocr: &ocr,
            title_hint: "厦大体育".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::MyReservations);
    }

    #[test]
    fn campus_gym_rows_are_the_venue_list() {
        let ocr = hits(&[
            ("场馆列表", 180, 40),
            ("翔安校区健身房", 80, 220),
            ("翔安校区", 80, 250),
            ("思明校区健身房", 80, 340),
            ("思明校区", 80, 370),
        ]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 1054,
            frame_h: 1563,
            ocr: &ocr,
            title_hint: "厦大体育".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::VenueList);
    }

    #[test]
    fn notice_title_beats_sport_names_behind_the_card() {
        let ocr = hits(&[
            ("【预约须知】", 140, 180),
            ("羽毛球", 80, 220),
            ("乒乓球", 160, 220),
            ("健身房", 240, 220),
        ]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 524,
            frame_h: 780,
            ocr: &ocr,
            title_hint: "厦大体育".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::BookingNotice);
    }

    #[test]
    fn sousousou_on_discover_is_not_mp_search() {
        let ocr = hits(&[
            ("发现", 20, 40),
            ("朋友圈", 80, 80),
            ("视频号", 80, 140),
            ("搜一搜", 80, 180),
            ("小程序", 80, 220),
            ("搜索", 360, 40),
        ]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 400,
            frame_h: 700,
            ocr: &ocr,
            title_hint: "微信".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::WechatDiscover);
    }

    #[test]
    fn home_needs_bottom_tab() {
        let ocr = hits(&[("场馆预约", 80, 280), ("首页", 40, 720)]);
        let snap = classify_from_hits(&ClassifyInput {
            frame_w: 390,
            frame_h: 780,
            ocr: &ocr,
            title_hint: "步道乐跑".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::Home);
    }

    #[test]
    fn shell_state_names_discover_without_ocr() {
        let ocr = hits(&[]);
        let mut snap = classify_from_hits(&ClassifyInput {
            frame_w: 400,
            frame_h: 700,
            ocr: &ocr,
            title_hint: "微信".into(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        assert_eq!(snap.page, Page::Unknown);
        apply_shell_page(&mut snap, Some(ShellVisualState::Active), None);
        assert_eq!(snap.page, Page::WechatDiscover);

        let mut home = classify_from_hits(&ClassifyInput {
            frame_w: 390,
            frame_h: 780,
            ocr: &hits(&[("场馆预约", 80, 280), ("首页", 40, 720)]),
            title_hint: String::new(),
            has_mint_slots: false,
            captcha_input_chars: String::new(),
            keyboard_open: false,
            toast_kind: None,
            slot_cta: SlotCta::Unknown,
        });
        apply_shell_page(
            &mut home,
            Some(ShellVisualState::Inactive),
            Some(ShellVisualState::Inactive),
        );
        assert_eq!(home.page, Page::Home);
    }

    #[test]
    fn active_miniprogram_names_panel_without_ocr() {
        let mut snap = UiSnapshot {
            page: Page::Unknown,
            confidence: 0.0,
            ..UiSnapshot::default()
        };
        apply_shell_page(
            &mut snap,
            Some(ShellVisualState::Active),
            Some(ShellVisualState::Active),
        );
        assert_eq!(snap.page, Page::WechatMpPanel);
    }

    #[test]
    fn miniprogram_without_discover_does_not_name_panel() {
        let mut snap = UiSnapshot {
            page: Page::Unknown,
            confidence: 0.0,
            ..UiSnapshot::default()
        };
        apply_shell_page(
            &mut snap,
            Some(ShellVisualState::Inactive),
            Some(ShellVisualState::Active),
        );
        assert_eq!(snap.page, Page::WechatMain);
    }
}

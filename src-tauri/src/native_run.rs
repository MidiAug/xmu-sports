//! Host-side atomic run loop decisions (trial / submit).
//! Actual glance/click live in `vision-platform-windows`; this module only decides
//! whether to click, wait, poll, succeed, or fail after each report.

use serde::{Deserialize, Serialize};

/// Re-observe interval while splash / anchors are not ready yet.
pub const SOFT_POLL_MS: u64 = 500;
/// Cap for soft polls (splash, late home). Policy Wait (auth/captcha) is separate.
pub const SOFT_POLL_BUDGET_MS: u64 = 45_000;

/// Monotonic elapsed time includes OCR/capture work, rather than only sleeps.
pub fn readiness_expired(started_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(started_ms) >= SOFT_POLL_BUDGET_MS
}

/// Tighten only near the deadline; keep long overnight waits cheap.
pub fn fire_wait_ms(remaining_ms: i64) -> u64 {
    if remaining_ms > 1_000 {
        500
    } else if remaining_ms > 100 {
        50
    } else {
        10
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunUntil {
    /// Open/authenticate the miniprogram and reach H5; HTTP owns all slot/submit actions.
    HttpEntry,
    /// Stop after slot-grid action (or all-grey); never confirm / captcha / submit.
    TrialSlot,
    /// Continue through confirm + captcha to a result page.
    Submit,
}

impl RunUntil {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("http_entry") => Self::HttpEntry,
            None | Some("") | Some("fill_captcha") | Some("submit") | Some("done") => Self::Submit,
            Some("trial_slot") | Some("select_date") | Some("select_slot") | Some("trial") => {
                Self::TrialSlot
            }
            _ => Self::TrialSlot,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GlanceSnap {
    pub page: String,
    pub can_click: bool,
    pub intent_kind: String,
    pub refuse_reason: Option<String>,
    pub wait_ms: Option<u64>,
    /// True when the click we just finished was on `select_slot`.
    pub clicked_from_select_slot: bool,
    /// Navigate label about to be clicked (e.g. 「确认预约」).
    pub click_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepAction {
    Click,
    /// Policy wait (auth settle, captcha error) — use reported ms.
    Wait(u64),
    /// Splash / anchors not ready — short re-observe; host enforces budget.
    Poll(u64),
    Succeed(&'static str),
    Fail(String),
}

fn reason(snap: &GlanceSnap) -> Option<&str> {
    snap.refuse_reason.as_deref()
}

fn is_result_page(page: &str) -> bool {
    matches!(page, "success" | "my_reservations")
}

/// Loading splash / shell before booking anchors exist — keep glancing.
pub fn is_soft_not_ready(snap: &GlanceSnap) -> bool {
    if snap.can_click {
        return false;
    }
    if snap.intent_kind == "wait" {
        return false;
    }
    match reason(snap) {
        Some("ANCHOR_NOT_FOUND") => matches!(
            snap.page.as_str(),
            "unknown"
                | "home"
                | "wechat_main"
                | "wechat_discover"
                | "wechat_mp_panel"
                | "wechat_mp_search"
                | ""
        ),
        // List pages often paint before OCR catches the goal row — re-glance.
        Some("VENUE_MISMATCH") => matches!(snap.page.as_str(), "venue_list" | "venue_detail"),
        Some("SPORT_NOT_ON_SCREEN") => snap.page == "venue_sport_list",
        Some("SLOT_GRID_NOT_LOCALIZED") => snap.page == "select_slot",
        _ => false,
    }
}

fn trial_success_after_glance(snap: &GlanceSnap) -> Option<&'static str> {
    if snap.page == "select_slot" && reason(snap) == Some("SLOT_ALL_GREY") {
        return Some("slot_all_grey");
    }
    // CTA already armed — trial must not click 「确认预约」.
    if snap.page == "select_slot" && snap.click_label == "确认预约" {
        return Some("trial_slot_ready");
    }
    if snap.page == "confirm_order" {
        return Some("trial_reached_confirm");
    }
    if is_result_page(&snap.page) || reason(snap) == Some("ALREADY_DONE") {
        return Some("already_done");
    }
    None
}

fn submit_success(snap: &GlanceSnap) -> Option<&'static str> {
    if is_result_page(&snap.page) || reason(snap) == Some("ALREADY_DONE") {
        return Some("submitted");
    }
    None
}

/// Decide what to do after a glance (or post-click re-observe) report.
pub fn decide_step(until: RunUntil, snap: &GlanceSnap) -> StepAction {
    if snap.clicked_from_select_slot && until == RunUntil::TrialSlot {
        return StepAction::Succeed("trial_slot_clicked");
    }

    match until {
        RunUntil::HttpEntry => {
            if matches!(snap.page.as_str(), "venue_sport_list" | "venue_list" | "venue_detail" | "select_slot" | "confirm_order") {
                return StepAction::Succeed("http_entry_ready");
            }
        }
        RunUntil::TrialSlot => {
            if let Some(code) = trial_success_after_glance(snap) {
                return StepAction::Succeed(code);
            }
        }
        RunUntil::Submit => {
            if let Some(code) = submit_success(snap) {
                return StepAction::Succeed(code);
            }
        }
    }

    if snap.intent_kind == "wait" {
        return StepAction::Wait(snap.wait_ms.unwrap_or(1_200).max(200));
    }

    if is_soft_not_ready(snap) {
        return StepAction::Poll(SOFT_POLL_MS);
    }

    if matches!(snap.intent_kind.as_str(), "stop" | "need_human" | "rebind") {
        let code = reason(snap)
            .unwrap_or(snap.intent_kind.as_str())
            .to_string();
        return StepAction::Fail(code);
    }

    if until == RunUntil::TrialSlot && snap.page == "confirm_order" {
        return StepAction::Succeed("trial_reached_confirm");
    }

    if snap.can_click && matches!(snap.intent_kind.as_str(), "navigate" | "atomic_composite") {
        return StepAction::Click;
    }

    StepAction::Fail(reason(snap).unwrap_or("NO_CLICKABLE_STEP").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_wait_tightens_near_seven_without_an_early_fire() {
        assert_eq!(fire_wait_ms(60_000), 500);
        assert_eq!(fire_wait_ms(900), 50);
        assert_eq!(fire_wait_ms(90), 10);
    }

    #[test]
    fn slow_blank_slot_page_remains_retryable_then_expires_by_elapsed_time() {
        let s = snap(
            "select_slot",
            "stop",
            false,
            Some("SLOT_GRID_NOT_LOCALIZED"),
            None,
        );
        assert_eq!(
            decide_step(RunUntil::Submit, &s),
            StepAction::Poll(SOFT_POLL_MS)
        );
        assert!(!readiness_expired(1_000, 16_000));
        assert!(readiness_expired(1_000, 46_000));
        let detail = snap("venue_detail", "stop", false, Some("VENUE_MISMATCH"), None);
        assert_eq!(
            decide_step(RunUntil::Submit, &detail),
            StepAction::Poll(SOFT_POLL_MS)
        );
        let unavailable = snap("select_slot", "stop", false, Some("SLOT_ALL_GREY"), None);
        assert_eq!(
            decide_step(RunUntil::Submit, &unavailable),
            StepAction::Fail("SLOT_ALL_GREY".into())
        );
    }

    fn snap(
        page: &str,
        kind: &str,
        can_click: bool,
        refuse: Option<&str>,
        wait_ms: Option<u64>,
    ) -> GlanceSnap {
        GlanceSnap {
            page: page.into(),
            can_click,
            intent_kind: kind.into(),
            refuse_reason: refuse.map(str::to_string),
            wait_ms,
            clicked_from_select_slot: false,
            click_label: String::new(),
        }
    }

    #[test]
    fn parse_until_aliases() {
        assert_eq!(RunUntil::parse(Some("http_entry")), RunUntil::HttpEntry);
        assert_eq!(RunUntil::parse(None), RunUntil::Submit);
        assert_eq!(RunUntil::parse(Some("trial_slot")), RunUntil::TrialSlot);
        assert_eq!(RunUntil::parse(Some("select_date")), RunUntil::TrialSlot);
        assert_eq!(RunUntil::parse(Some("fill_captcha")), RunUntil::Submit);
    }

    #[test]
    fn http_handoff_never_clicks_slots_or_confirmation() {
        for page in ["venue_sport_list", "venue_list", "venue_detail", "select_slot", "confirm_order"] {
            let current = snap(page, "navigate", true, None, None);
            assert_eq!(decide_step(RunUntil::HttpEntry, &current), StepAction::Succeed("http_entry_ready"));
        }
        let login = snap("xmu_wx_authorize", "navigate", true, None, None);
        assert_eq!(decide_step(RunUntil::HttpEntry, &login), StepAction::Click);
    }

    #[test]
    fn trial_clicks_available_slot() {
        let mut s = snap("select_slot", "navigate", true, None, None);
        s.click_label = "16:30".into();
        assert_eq!(decide_step(RunUntil::TrialSlot, &s), StepAction::Click);
    }

    #[test]
    fn trial_stops_when_confirm_cta_armed() {
        let mut s = snap("select_slot", "navigate", true, None, None);
        s.click_label = "确认预约".into();
        assert_eq!(
            decide_step(RunUntil::TrialSlot, &s),
            StepAction::Succeed("trial_slot_ready")
        );
    }

    #[test]
    fn trial_stops_after_slot_click() {
        let mut s = snap("confirm_order", "navigate", true, None, None);
        s.clicked_from_select_slot = true;
        assert_eq!(
            decide_step(RunUntil::TrialSlot, &s),
            StepAction::Succeed("trial_slot_clicked")
        );
    }

    #[test]
    fn trial_stops_on_all_grey() {
        let s = snap("select_slot", "stop", false, Some("SLOT_ALL_GREY"), None);
        assert_eq!(
            decide_step(RunUntil::TrialSlot, &s),
            StepAction::Succeed("slot_all_grey")
        );
    }

    #[test]
    fn trial_does_not_click_confirm() {
        let s = snap("confirm_order", "navigate", true, None, None);
        assert_eq!(
            decide_step(RunUntil::TrialSlot, &s),
            StepAction::Succeed("trial_reached_confirm")
        );
    }

    #[test]
    fn submit_continues_on_confirm() {
        let s = snap("confirm_order", "navigate", true, None, None);
        assert_eq!(decide_step(RunUntil::Submit, &s), StepAction::Click);
    }

    #[test]
    fn submit_succeeds_on_already_done_even_if_page_misfires() {
        let s = snap("venue_list", "stop", false, Some("ALREADY_DONE"), None);
        assert_eq!(
            decide_step(RunUntil::Submit, &s),
            StepAction::Succeed("submitted")
        );
    }

    #[test]
    fn submit_succeeds_on_result_page() {
        let s = snap("success", "stop", false, Some("ALREADY_DONE"), None);
        assert_eq!(
            decide_step(RunUntil::Submit, &s),
            StepAction::Succeed("submitted")
        );
    }

    #[test]
    fn wait_uses_report_ms() {
        let s = snap(
            "xmu_auth_waiting",
            "wait",
            false,
            Some("AUTH_WAITING"),
            Some(5_000),
        );
        assert_eq!(decide_step(RunUntil::Submit, &s), StepAction::Wait(5_000));
    }

    #[test]
    fn splash_anchor_missing_polls() {
        let s = snap("unknown", "stop", false, Some("ANCHOR_NOT_FOUND"), None);
        assert_eq!(
            decide_step(RunUntil::TrialSlot, &s),
            StepAction::Poll(SOFT_POLL_MS)
        );
    }

    #[test]
    fn hard_stop_fails() {
        let s = snap(
            "order_detail",
            "stop",
            false,
            Some("NOT_ON_BOOKING_PATH"),
            None,
        );
        assert_eq!(
            decide_step(RunUntil::TrialSlot, &s),
            StepAction::Fail("NOT_ON_BOOKING_PATH".into())
        );
    }

    #[test]
    fn slot_grid_not_ready_polls() {
        let s = snap(
            "select_slot",
            "stop",
            false,
            Some("SLOT_GRID_NOT_LOCALIZED"),
            None,
        );
        assert_eq!(
            decide_step(RunUntil::TrialSlot, &s),
            StepAction::Poll(SOFT_POLL_MS)
        );
    }

    #[test]
    fn venue_list_mismatch_polls_while_ocr_catches_up() {
        let s = snap("venue_list", "stop", false, Some("VENUE_MISMATCH"), None);
        assert_eq!(
            decide_step(RunUntil::TrialSlot, &s),
            StepAction::Poll(SOFT_POLL_MS)
        );
    }

    #[test]
    fn venue_mismatch_still_hard() {
        // Wrong page for this refuse — not soft.
        let s = snap("home", "stop", false, Some("VENUE_MISMATCH"), None);
        assert_eq!(
            decide_step(RunUntil::TrialSlot, &s),
            StepAction::Fail("VENUE_MISMATCH".into())
        );
    }
}

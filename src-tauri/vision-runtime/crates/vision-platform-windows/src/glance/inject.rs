//! Guarded click / atomic search / captcha keyboard / fragile-nav inject path.

#![cfg(windows)]

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use vision_contracts::{ActionPlan, ActionType, ReasonCode, ValidateOutcome};
use vision_core::{
    bottom_band_fingerprint, build_click_plan, click_safety_ok, click_safety_score,
    fragile_nav_target_changed, guard_action, is_fragile_nav_label, keyboard_done_cy,
    relocate_anchor_patch, AtomicStep, BandSettleTracker, GuardInput, NavLabelStreak, NextIntent,
    Page, SubmitPre, TaskGoal, ANCHOR_PATCH_SEARCH_RADIUS, CLICK_SAFETY_MIN, DEFAULT_PLAN_TTL_MS,
    FRAGILE_NAV_STREAK_NEED, KBD_BAND_STREAK_NEED, KBD_BAND_Y0_FRAC,
};
use vision_platform_api::PlatformPort;

use crate::glance::bind::{
    bind_wechat, ensure_winrt_thread, grab_bound_timeout, BoundWechat, Observed,
};
use crate::glance::observe::{
    captcha_filled_count, captcha_tile_fingerprint, observe_bound, observe_captured,
    poll_after_click, read_captcha_tile, PollReady,
};
use crate::glance::report::{
    build_report, load_min_confidence, log_timing, GlanceMode, GlanceReport,
};
use crate::ocr::ocr_frame;

pub(crate) const SEARCH_FOCUS_MS: u64 = 350;
/// Post-click: re-observe on a short interval until the page changes (no fixed settle).
pub(crate) const CAPTCHA_KEY_GAP_MS: u64 = 180;
/// Soft keyboard overshoots then drops; poll band until pixels stop moving.
pub(crate) const KBD_SETTLE_POLL_MS: u64 = 70;
/// Ignore a brief freeze at the overshoot peak.
pub(crate) const KBD_SETTLE_MIN_MS: u64 = 350;
/// Cap waiting for keyboard sheet bounce to finish.
pub(crate) const KBD_SETTLE_BUDGET_MS: u64 = 2_500;
/// Short WGC wait while sampling settle frames.
pub(crate) const KBD_SETTLE_GRAB_MS: u64 = 400;
/// Leave a margin under the same-frame Guard TTL when WGC is static. A 400ms
/// wait consumed the entire 400ms allowance and caused deterministic STALE_FRAME.
pub(crate) const VERIFY_GRAB_MS: u64 = 100;
/// When WGC has no new frame, allow same-frame inject only within this age.
pub(crate) const SAME_FRAME_VERIFY_MAX_AGE_MS: u64 = 400;
/// Re-observe gap while confirming 立即登录 / 场馆预约 (login flash guard).
pub(crate) const FRAGILE_NAV_POLL_MS: u64 = 250;
/// Cap for fragile-nav multi-frame confirm.
pub(crate) const FRAGILE_NAV_BUDGET_MS: u64 = 2_500;
pub(crate) fn glance_wechat_win(
    data_root: &Path,
    goal: TaskGoal,
    input_invoked: bool,
    mode: GlanceMode,
) -> Result<GlanceReport, String> {
    ensure_winrt_thread();
    let t_all = std::time::Instant::now();
    let run_id = format!("ui-{}", vision_contracts::monotonic_now_ms());
    let mut session = crate::session::RunSession::open(data_root, &run_id)?;
    let obs = session.observe(&goal, mode)?;
    let mut report = obs.report;
    if input_invoked {
        report.input_invoked = true;
    }
    log_timing(
        t_all,
        &session.bound_title(),
        session.last_observed().expect("observe just succeeded"),
        &report,
    );
    Ok(report)
}

pub(crate) fn click_wechat_step_win(
    data_root: &Path,
    goal: TaskGoal,
    mode: GlanceMode,
) -> Result<GlanceReport, String> {
    ensure_winrt_thread();
    let t_all = std::time::Instant::now();
    let mut bound = bind_wechat(data_root)?;
    bound.run_id = format!("ui-{}", vision_contracts::monotonic_now_ms());
    let observed = observe_bound(data_root, &mut bound, &goal, None)?;
    let (report, after) =
        click_from_observed(data_root, &mut bound, observed, &goal, mode, None)?;
    log_timing(t_all, &bound.native.title, &after, &report);
    Ok(report)
}

/// Execute a click from an already-observed frame on an existing bind (no rebind).
/// Returns `(report, post_click_observation)`.
///
/// When `pending_plan` is `Some`, that ActionPlan (source frame) is used for the
/// primary click — never rebuilt. Verification still grabs a fresh WGC frame.
pub(crate) fn click_from_observed(
    data_root: &Path,
    bound: &mut BoundWechat,
    observed: Observed,
    goal: &TaskGoal,
    mode: GlanceMode,
    pending_plan: Option<ActionPlan>,
) -> Result<(GlanceReport, Observed), String> {
    // Prefer host-persisted captcha; fall back to what observe just OCR'd.
    let mut effective_goal = observed.decision.goal.clone();
    if effective_goal.recognized_captcha.is_none() {
        effective_goal.recognized_captcha = goal.recognized_captcha.clone();
    }
    let min_conf = load_min_confidence(data_root);
    if observed.decision.confidence < min_conf {
        return Err("这一步把握不够，先别点".into());
    }

    // Confirm-order soft keyboard open: wait bounce settle → OCR + burst +「完成」;
    // if OCR cannot read 4 chars, click the captcha tile to refresh (keyboard collapses).
    if observed.result.snapshot.page == Page::ConfirmOrder
        && observed.result.snapshot.keyboard_open
    {
        let settled =
            wait_keyboard_settled(data_root, bound, &effective_goal, observed)?;
        let (after, refreshed) =
            type_captcha_or_refresh(data_root, bound, settled, effective_goal)?;
        let mut report = build_report(&bound.native.title, &after, true, mode)?;
        if refreshed {
            mark_captcha_refreshed(&mut report);
        }
        return Ok((report, after));
    }

    let intent = observed.decision.intent.clone();
    match intent {
        NextIntent::Navigate {
            label,
            anchor,
            expected_page,
        } => {
            let mut current = observed;
            // After fragile confirm, never reuse the observe-time pending plan:
            // confirm can take > DEFAULT_PLAN_TTL_MS (esp. static_no_frame streak),
            // and same-frame verify age is measured from plan.created → PLAN_EXPIRED /
            // STALE_FRAME. Rebuild at inject with a fresh created_monotonic_ms.
            let (label, anchor, expected_page, from_page, fragile_confirmed) =
                if is_fragile_nav_label(&label) {
                    match confirm_fragile_navigate(
                        data_root,
                        bound,
                        &effective_goal,
                        current,
                        &label,
                    )? {
                        FragileNav::Ready(obs, lab, anc, exp) => {
                            let page = obs.result.snapshot.page;
                            current = obs;
                            (lab, anc, exp, page, true)
                        }
                        FragileNav::Redirected(obs) => {
                            let report = build_report(&bound.native.title, &obs, false, mode)?;
                            return Ok((report, obs));
                        }
                    }
                } else {
                    (
                        label,
                        anchor,
                        expected_page,
                        current.result.snapshot.page,
                        false,
                    )
                };
            let opened_captcha = from_page == Page::ConfirmOrder && label == "captcha_input";
            let refresh_tile = from_page == Page::ConfirmOrder && label == "captcha_image";
            let submit_pre = if from_page == Page::ConfirmOrder && label == "确定预约" {
                let filled = captcha_filled_count(&current.result.snapshot.input_chars);
                let tile_fp = captcha_tile_fingerprint(
                    current.frame.pixel_width,
                    current.frame.pixel_height,
                    &current.pixels,
                    &current.result.candidate_set,
                );
                log::info!(
                    target: "glance",
                    "[captcha_submit] pre filled={filled} tile_fp={tile_fp:#x} code_len={}",
                    effective_goal
                        .recognized_captcha
                        .as_deref()
                        .map(|c| c.chars().filter(|ch| ch.is_ascii_alphanumeric()).count())
                        .unwrap_or(0)
                );
                Some(SubmitPre { filled, tile_fp })
            } else {
                None
            };
            let ready = if expected_page == from_page {
                PollReady::SamePageNextControl
            } else {
                PollReady::PageChanged
            };
            let baseline_fp = crate::rapid_ocr::frame_fingerprint(&current.pixels);
            // Pending plan is only valid for the same source frame. Fragile confirm
            // either re-observes (new frame) or keeps the static frame for >TTL —
            // both require a freshly dated plan at inject.
            let plan_for_click = pending_plan
                .as_ref()
                .filter(|p| p.source_frame_id() == current.frame.frame_id)
                .filter(|_| !fragile_confirmed);
            inject_one_click(
                data_root,
                bound,
                &current,
                &label,
                anchor,
                &current.decision.expected_transition,
                current.decision.confidence,
                plan_for_click,
            )?;
            let mut after = poll_after_click(
                data_root,
                bound,
                &effective_goal,
                from_page,
                &label,
                expected_page,
                ready,
                baseline_fp,
                submit_pre,
            )?;
            let mut refreshed = false;
            if opened_captcha && after.result.snapshot.keyboard_open {
                let mut goal = effective_goal.clone();
                if let Some(code) = after.decision.goal.recognized_captcha.clone() {
                    goal.recognized_captcha = Some(code);
                }
                let settled = wait_keyboard_settled(data_root, bound, &goal, after)?;
                let (next, did_refresh) =
                    type_captcha_or_refresh(data_root, bound, settled, goal)?;
                after = next;
                refreshed = did_refresh;
            } else if refresh_tile {
                // Pre-keyboard refresh: clear only when the new tile still has no
                // readable 4-char code (otherwise keep it and open keyboard next).
                let code_ok = after
                    .decision
                    .goal
                    .recognized_captcha
                    .as_deref()
                    .map(|c| {
                        c.chars()
                            .filter(|ch| ch.is_ascii_alphanumeric())
                            .take(4)
                            .count()
                            >= 4
                    })
                    .unwrap_or(false);
                refreshed = !code_ok;
            }
            let mut report = build_report(&bound.native.title, &after, true, mode)?;
            if refreshed {
                mark_captcha_refreshed(&mut report);
            }
            Ok((report, after))
        }
        NextIntent::AtomicComposite {
            label,
            steps,
            expected_page: _,
        } => {
            let baseline_fp = crate::rapid_ocr::frame_fingerprint(&observed.pixels);
            let plan_for_click = pending_plan
                .as_ref()
                .filter(|p| p.source_frame_id() == observed.frame.frame_id);
            run_atomic_search(
                data_root,
                bound,
                &observed,
                &effective_goal,
                &label,
                &steps,
                observed.decision.confidence,
                baseline_fp,
                plan_for_click,
            )?;
            let after = observe_bound(data_root, bound, &effective_goal, None)?;
            let report = build_report(&bound.native.title, &after, true, mode)?;
            Ok((report, after))
        }
        _ => Err("这一步没有明确可点的位置，先「看这一页」确认绿框".into()),
    }
}

pub(crate) fn inject_one_click(
    data_root: &Path,
    bound: &mut BoundWechat,
    observed: &Observed,
    label: &str,
    anchor: vision_contracts::AnchorBox,
    expected_transition: &str,
    confidence: f32,
    source_plan: Option<&ActionPlan>,
) -> Result<(), String> {
    validate_click_surface(bound, label, &observed.frame.frame_id, "before_activate")?;
    bound
        .plat
        .activate_surface(&bound.lease, true)
        .map_err(|e| e.to_string())?;

    let created = vision_contracts::monotonic_now_ms();
    let run_id = if bound.run_id.is_empty() {
        format!("ui-{}", created)
    } else {
        bound.run_id.clone()
    };
    let built_plan;
    let plan: &ActionPlan = if let Some(p) = source_plan {
        p
    } else {
        built_plan = build_click_plan(
            &run_id,
            &bound.lease.lease_id,
            bound.lease.generation,
            &observed.frame.frame_id,
            vision_contracts::SizeI {
                width: observed.frame.pixel_width,
                height: observed.frame.pixel_height,
            },
            &observed.result.snapshot,
            anchor.clone(),
            ActionType::Click,
            expected_transition,
            confidence,
            created,
            DEFAULT_PLAN_TTL_MS,
        );
        &built_plan
    };
    let plan_anchor = plan
        .anchor_box
        .clone()
        .unwrap_or_else(|| anchor.clone());

    // Lightweight verification frame — never re-decide; only drift / size / lease.
    let (verify_frame, verify_pixels, is_new_capture) =
        match grab_bound_timeout(bound, Duration::from_millis(VERIFY_GRAB_MS)) {
            Ok((frame, pixels)) => {
                let is_new = frame.frame_id != observed.frame.frame_id;
                (frame, pixels, is_new)
            }
            Err(e) if e.contains("还没有新的一帧") => {
                let capture_age = vision_contracts::monotonic_now_ms()
                    .saturating_sub(observed.frame.monotonic_capture_time_ms);
                if capture_age <= SAME_FRAME_VERIFY_MAX_AGE_MS {
                    (observed.frame.clone(), observed.pixels.clone(), false)
                } else {
                    // The plan timestamp cannot make old pixels fresh. Static UI
                    // gets one real verification capture, without replaying input.
                    bound.restart_capture()?;
                    let (frame, pixels) = grab_bound_timeout(bound, Duration::from_millis(1_000))?;
                    log::info!(target: "glance", "[verify_capture] label={} source_age_ms={} outcome=refreshed wgc_open={}",
                        label, capture_age, bound.capture_open_count);
                    (frame, pixels, true)
                }
            }
            Err(e) => return Err(e),
        };

    let verification_anchor = if is_new_capture {
        let src_fp = crate::rapid_ocr::frame_fingerprint(&observed.pixels);
        let ver_fp = crate::rapid_ocr::frame_fingerprint(&verify_pixels);
        if src_fp == ver_fp {
            plan_anchor.clone()
        } else {
            relocate_anchor_patch(
                &observed.pixels,
                observed.frame.pixel_width,
                observed.frame.pixel_height,
                &verify_pixels,
                verify_frame.pixel_width,
                verify_frame.pixel_height,
                &plan_anchor,
                ANCHOR_PATCH_SEARCH_RADIUS,
            )
            .ok_or_else(|| format!("{}ANCHOR_MOVED:verification_patch_lost", crate::PRE_INPUT_REOBSERVE_PREFIX))?
        }
    } else {
        plan_anchor.clone()
    };

    let safety = click_safety_score(
        plan.confidence.max(confidence),
        Some(&plan_anchor),
        Some(&verification_anchor),
        ANCHOR_PATCH_SEARCH_RADIUS,
    );
    if !click_safety_ok(safety) {
        log::warn!(target: "glance",
            "[click_safety] label={} plan_confidence={} source_anchor={:?} verification_anchor={:?} drift_px={} safety={} verify_new={}",
            label, plan.confidence.max(confidence), plan_anchor, verification_anchor,
            plan_anchor.center_drift_px(&verification_anchor), safety, is_new_capture);
        return Err(format!(
            "{}CLICK_SAFETY_LOW:score={safety:.3}:min={CLICK_SAFETY_MIN}",
            crate::PRE_INPUT_REOBSERVE_PREFIX
        ));
    }
    let validate = validate_click_surface(bound, label, &verify_frame.frame_id, "before_input")?;
    let guard_now = vision_contracts::monotonic_now_ms();
    let plan_age_ms = guard_now.saturating_sub(plan.created_monotonic_ms);
    let guarded = guard_action(GuardInput {
        plan,
        lease: &bound.lease,
        validate: &validate,
        session_run_id: &plan.run_id,
        fresh_snap: &observed.result.snapshot,
        verification_frame_id: &verify_frame.frame_id,
        verification_generation: bound.lease.generation,
        verification_capture_size: vision_contracts::SizeI {
            width: verify_frame.pixel_width,
            height: verify_frame.pixel_height,
        },
        verification_anchor: Some(&verification_anchor),
        now_monotonic_ms: guard_now,
        verification_is_new_capture: is_new_capture,
        same_frame_max_age_ms: SAME_FRAME_VERIFY_MAX_AGE_MS,
        input_enabled: true,
        min_confidence: load_min_confidence(data_root),
        max_anchor_drift_px: ANCHOR_PATCH_SEARCH_RADIUS,
        already_consumed: false,
    })
    .map_err(|e| {
        log::warn!(
            target: "glance",
            "[click_guard] outcome=rejected run_id={} plan_id={} source_frame={} verification_frame={} lease_id={} generation={} label={} verify_new={} verify_wait_budget_ms={} plan_age_ms={} same_frame_max_age_ms={} reason={}",
            plan.run_id, plan.action_plan_id, plan.frame_id, verify_frame.frame_id,
            bound.lease.lease_id, bound.lease.generation, label, is_new_capture,
            VERIFY_GRAB_MS, plan_age_ms, SAME_FRAME_VERIFY_MAX_AGE_MS, e,
        );
        let reason = e.to_string();
        if reason == ReasonCode::STALE_FRAME || reason == ReasonCode::PLAN_EXPIRED {
            format!("{}{reason}", crate::PRE_INPUT_REOBSERVE_PREFIX)
        } else {
            reason
        }
    })?;

    // Screen transform from the verification frame (source anchor semantics).
    bound.plat.input_enabled = true;
    let injected = bound
        .plat
        .inject_input(&guarded, &bound.lease, &verify_frame);
    bound.plat.input_enabled = false;
    let receipt = injected.map_err(|e| {
        log::error!(
            target: "glance",
            "[click_input] outcome=failed run_id={} plan_id={} source_frame={} verification_frame={} lease_id={} generation={} label={} reason={}",
            plan.run_id, plan.action_plan_id, plan.frame_id, verify_frame.frame_id,
            bound.lease.lease_id, bound.lease.generation, label, e,
        );
        e.to_string()
    })?;
    log::info!(
        target: "glance",
        "[click_input] outcome=accepted run_id={} plan_id={} source_frame={} verification_frame={} lease_id={} generation={} label={} verify_new={} detail={}",
        plan.run_id,
        plan.action_plan_id,
        plan.frame_id,
        verify_frame.frame_id,
        bound.lease.lease_id,
        bound.lease.generation,
        label,
        is_new_capture,
        receipt.detail
    );
    Ok(())
}

fn validate_click_surface(
    bound: &mut BoundWechat,
    label: &str,
    frame_id: &str,
    phase: &str,
) -> Result<ValidateOutcome, String> {
    let validate = bound
        .plat
        .validate_surface(&bound.lease)
        .map_err(|e| e.to_string())?;
    match &validate {
        ValidateOutcome::Valid => Ok(validate),
        ValidateOutcome::RebindRequired { reason } | ValidateOutcome::Invalid { reason } => {
            log::warn!(
                target: "glance",
                "[click_guard] outcome=pre_input_rebind phase={} run_id={} label={} frame={} lease_id={} generation={} reason={}",
                phase, bound.run_id, label, frame_id, bound.lease.lease_id,
                bound.lease.generation, reason,
            );
            Err(format!("{}{reason}", crate::PRE_INPUT_REBIND_PREFIX))
        }
    }
}

#[cfg(test)]
mod verification_budget_tests {
    use super::{SAME_FRAME_VERIFY_MAX_AGE_MS, VERIFY_GRAB_MS};

    #[test]
    fn static_frame_wait_leaves_guard_time_margin() {
        assert!(VERIFY_GRAB_MS * 2 < SAME_FRAME_VERIFY_MAX_AGE_MS);
    }
}

/// Magnifier → type → click result, without returning focus to this app.
pub(crate) fn run_atomic_search(
    data_root: &Path,
    bound: &mut BoundWechat,
    first: &Observed,
    goal: &TaskGoal,
    label: &str,
    steps: &[AtomicStep],
    confidence: f32,
    baseline_fp: u64,
    pending_plan: Option<&ActionPlan>,
) -> Result<(), String> {
    let (search_label, search_anchor) = match steps.first() {
        Some(AtomicStep::Click { label, anchor }) => (label.as_str(), anchor.clone()),
        _ => return Err("搜索步骤缺少输入内容".into()),
    };
    let type_text = match steps.get(1) {
        Some(AtomicStep::TypeText { text }) => text.as_str(),
        _ => return Err("搜索步骤缺少输入内容".into()),
    };

    inject_one_click(
        data_root,
        bound,
        first,
        search_label,
        search_anchor,
        "wechat_mp_search",
        confidence,
        pending_plan,
    )?;
    // Keep WeChat foreground. Do not observe/report yet — that would steal focus.
    thread::sleep(Duration::from_millis(SEARCH_FOCUS_MS));
    bound.plat.input_enabled = true;
    let type_created = vision_contracts::monotonic_now_ms();
    let type_run_id = format!("ui-type-{}", type_created);
    let type_plan = {
        let mut plan = build_click_plan(
            &type_run_id,
            &bound.lease.lease_id,
            bound.lease.generation,
            &first.frame.frame_id,
            vision_contracts::SizeI {
                width: first.frame.pixel_width,
                height: first.frame.pixel_height,
            },
            &first.result.snapshot,
            vision_contracts::AnchorBox {
                x0: 0,
                y0: 0,
                x1: 1,
                y1: 1,
            },
            ActionType::TypeText,
            "wechat_mp_search",
            confidence,
            type_created,
            vision_core::DEFAULT_PLAN_TTL_MS,
        );
        plan.anchor_box = None;
        plan.text = Some(type_text.to_string());
        plan
    };
    let validate = bound
        .plat
        .validate_surface(&bound.lease)
        .map_err(|e| e.to_string())?;
    let guarded = guard_action(GuardInput {
        plan: &type_plan,
        lease: &bound.lease,
        validate: &validate,
        session_run_id: &type_run_id,
        fresh_snap: &first.result.snapshot,
        verification_frame_id: &first.frame.frame_id,
        verification_generation: bound.lease.generation,
        verification_capture_size: vision_contracts::SizeI {
            width: first.frame.pixel_width,
            height: first.frame.pixel_height,
        },
        verification_anchor: None,
        now_monotonic_ms: vision_contracts::monotonic_now_ms(),
        verification_is_new_capture: false,
        same_frame_max_age_ms: vision_core::DEFAULT_PLAN_TTL_MS,
        input_enabled: true,
        min_confidence: load_min_confidence(data_root),
        max_anchor_drift_px: 24,
        already_consumed: false,
    })
    .map_err(|e| e.to_string())?;
    let receipt = bound
        .plat
        .inject_input(&guarded, &bound.lease, &first.frame)
        .map_err(|e| e.to_string())?;
    log::info!(target: "glance", "[type] label={label} detail={}", receipt.detail);
    bound.plat.input_enabled = false;

    // Poll until search results yield a Navigate — no fixed SEARCH_RESULTS sleep.
    let search_from = first.result.snapshot.page;
    let mid = poll_after_click(
        data_root,
        bound,
        goal,
        search_from,
        "搜索结果",
        search_from,
        PollReady::PageChangedOrNavigate,
        baseline_fp,
        None,
    )?;
    let result_hit = match &mid.decision.intent {
        NextIntent::Navigate { label, anchor, .. } if label == "厦大体育" => anchor.clone(),
        NextIntent::AtomicComposite { .. } => {
            return Err("搜索后仍停在搜索入口，没有匹配结果".into());
        }
        other => {
            return Err(format!(
                "搜索后没有可点的厦大体育结果：{}",
                match other {
                    NextIntent::Stop { reason }
                    | NextIntent::NeedHuman { reason }
                    | NextIntent::Rebind { reason } => reason.clone(),
                    NextIntent::Wait { reason, .. } => reason.clone(),
                    NextIntent::Navigate { label, .. } => label.clone(),
                    NextIntent::AtomicComposite { label, .. } => label.clone(),
                }
            ));
        }
    };
    let from_open = mid.result.snapshot.page;
    let open_baseline = crate::rapid_ocr::frame_fingerprint(&mid.pixels);
    inject_one_click(
        data_root,
        bound,
        &mid,
        "厦大体育",
        result_hit,
        "home",
        mid.decision.confidence.max(confidence),
        None,
    )?;
    let _ = poll_after_click(
        data_root,
        bound,
        goal,
        from_open,
        "厦大体育",
        from_open,
        PollReady::PageChanged,
        open_baseline,
        None,
    )?;
    Ok(())
}

/// Soft keyboard already open: burst-click code keys, then「完成」.
///
/// Do not OCR/ink-check between keys — thin glyphs (`l`) look like a blinking
/// cursor and falsely stall progress. Research path: 连敲 4 键 → 完成.
///
/// Wait until the soft-keyboard sheet stops bouncing (overshoot → rest).
///
/// Samples the lower-band fingerprint without OCR; when stable for
/// [`KBD_BAND_STREAK_NEED`] frames and past [`KBD_SETTLE_MIN_MS`], runs one
/// full observe so key mapping uses the settled frame.
pub(crate) fn wait_keyboard_settled(
    data_root: &Path,
    bound: &mut BoundWechat,
    goal: &TaskGoal,
    seed: Observed,
) -> Result<Observed, String> {
    let t0 = Instant::now();
    let mut band = BandSettleTracker::new(KBD_BAND_STREAK_NEED);
    let mut last = seed;
    let mut polls = 0u32;
    let mut last_logged_streak = 0u32;

    let seed_fp = bottom_band_fingerprint(
        &last.pixels,
        last.frame.pixel_width,
        last.frame.pixel_height,
        KBD_BAND_Y0_FRAC,
    );
    let seed_cy = keyboard_done_cy(&last.result.candidate_set);
    band.push(seed_fp);
    log::info!(
        target: "glance",
        "[kbd_settle] start fp={seed_fp:#x} done_cy={seed_cy:?} streak={} y0_frac={KBD_BAND_Y0_FRAC} keyboard_open={}",
        band.streak(),
        last.result.snapshot.keyboard_open
    );

    while t0.elapsed() < Duration::from_millis(KBD_SETTLE_BUDGET_MS) {
        polls += 1;
        thread::sleep(Duration::from_millis(KBD_SETTLE_POLL_MS));
        let (frame, pixels) =
            match grab_bound_timeout(bound, Duration::from_millis(KBD_SETTLE_GRAB_MS)) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!(target: "glance", "[kbd_settle] grab failed: {e}");
                    continue;
                }
            };
        let fp = bottom_band_fingerprint(
            &pixels,
            frame.pixel_width,
            frame.pixel_height,
            KBD_BAND_Y0_FRAC,
        );
        let band_ok = band.push(fp);
        let held = t0.elapsed() >= Duration::from_millis(KBD_SETTLE_MIN_MS);
        // Quiet: only log when streak advances or on settle/timeout (caret noise
        // used to spam every 70ms when the band included the input blink).
        if band.streak() != last_logged_streak && band.streak() >= 2 {
            last_logged_streak = band.streak();
            log::info!(
                target: "glance",
                "[kbd_settle] streak={} fp={fp:#x} held={held} elapsed_ms={}",
                band.streak(),
                t0.elapsed().as_millis()
            );
        }
        if !(band_ok && held) {
            last.frame = frame;
            last.pixels = pixels;
            continue;
        }
        let obs = observe_captured(data_root, bound, goal, frame, pixels, None)?;
        let cy = keyboard_done_cy(&obs.result.candidate_set);
        log::info!(
            target: "glance",
            "[kbd_settle] settled polls={polls} done_cy={cy:?} keyboard_open={} elapsed_ms={}",
            obs.result.snapshot.keyboard_open,
            t0.elapsed().as_millis()
        );
        if obs.result.snapshot.keyboard_open {
            return Ok(obs);
        }
        last = obs;
        band = BandSettleTracker::new(KBD_BAND_STREAK_NEED);
        last_logged_streak = 0;
    }

    log::warn!(
        target: "glance",
        "[kbd_settle] timeout polls={polls} elapsed_ms={} → observe last",
        t0.elapsed().as_millis()
    );
    if last.frame.pixel_width > 0 && !last.pixels.is_empty() {
        let frame = last.frame.clone();
        let pixels = last.pixels.clone();
        return observe_captured(data_root, bound, goal, frame, pixels, None);
    }
    observe_bound(data_root, bound, goal, None)
}

pub(crate) enum FragileNav {
    /// Same fragile label confirmed across frames — safe to click.
    Ready(
        Observed,
        String,
        vision_contracts::AnchorBox,
        Page,
    ),
    /// Flash resolved to another page/intent — report without clicking.
    Redirected(Observed),
}

/// Re-observe until「立即登录」/「场馆预约」stick for [`FRAGILE_NAV_STREAK_NEED`]
/// frames, or the page flips away from the flash.
///
/// Static home (WGC `还没有新的一帧`) counts as another confirm of the *same*
/// observation — settled UI is the opposite of a login flash.
pub(crate) fn confirm_fragile_navigate(
    data_root: &Path,
    bound: &mut BoundWechat,
    goal: &TaskGoal,
    seed: Observed,
    wanted_label: &str,
) -> Result<FragileNav, String> {
    let t0 = Instant::now();
    let mut streak = NavLabelStreak::new(FRAGILE_NAV_STREAK_NEED);
    let mut current = seed;
    let mut polls = 0u32;

    let push_obs = |streak: &mut NavLabelStreak,
                    obs: &Observed|
     -> Option<(String, vision_contracts::AnchorBox, Page)> {
        match &obs.decision.intent {
            NextIntent::Navigate {
                label,
                anchor,
                expected_page,
            } if is_fragile_nav_label(label) => {
                let page = obs.result.snapshot.page;
                if streak.push(page, label) {
                    Some((label.clone(), anchor.clone(), *expected_page))
                } else {
                    None
                }
            }
            _ => None,
        }
    };

    // Seed counts as sample 1.
    let _ = push_obs(&mut streak, &current);
    log::info!(
        target: "glance",
        "[nav_confirm] start want={wanted_label} page={} streak={} need={FRAGILE_NAV_STREAK_NEED}",
        current.result.snapshot.page.as_str(),
        streak.streak()
    );

    while t0.elapsed() < Duration::from_millis(FRAGILE_NAV_BUDGET_MS) {
        if streak.streak() >= FRAGILE_NAV_STREAK_NEED {
            if let NextIntent::Navigate {
                label,
                anchor,
                expected_page,
            } = &current.decision.intent
            {
                if is_fragile_nav_label(label) {
                    let lab = label.clone();
                    let anc = anchor.clone();
                    let exp = *expected_page;
                    log::info!(
                        target: "glance",
                        "[nav_confirm] confirmed label={lab} page={} polls={polls} streak={} elapsed_ms={}",
                        current.result.snapshot.page.as_str(),
                        streak.streak(),
                        t0.elapsed().as_millis()
                    );
                    return Ok(FragileNav::Ready(current, lab, anc, exp));
                }
            }
        }

        polls += 1;
        thread::sleep(Duration::from_millis(FRAGILE_NAV_POLL_MS));

        // Short wait: static home stops delivering frames — that is settle, not failure.
        let grabbed = match grab_bound_timeout(bound, Duration::from_millis(400)) {
            Ok(pair) => Some(pair),
            Err(e) if e.contains("还没有新的一帧") => {
                log::info!(
                    target: "glance",
                    "[nav_confirm] poll={polls} gate=static_no_frame page={} streak={} elapsed_ms={}",
                    current.result.snapshot.page.as_str(),
                    streak.streak(),
                    t0.elapsed().as_millis()
                );
                // Same pixels still on screen → confirm the last classification again.
                if let Some(ready) = push_obs(&mut streak, &current) {
                    log::info!(
                        target: "glance",
                        "[nav_confirm] confirmed label={} page={} polls={polls} streak={} (static) elapsed_ms={}",
                        ready.0,
                        current.result.snapshot.page.as_str(),
                        streak.streak(),
                        t0.elapsed().as_millis()
                    );
                    return Ok(FragileNav::Ready(current, ready.0, ready.1, ready.2));
                }
                None
            }
            Err(e) => {
                log::warn!(target: "glance", "[nav_confirm] grab failed: {e}");
                None
            }
        };

        let Some((frame, pixels)) = grabbed else {
            continue;
        };
        current = observe_captured(data_root, bound, goal, frame, pixels, None)?;
        let page = current.result.snapshot.page;
        let next_lab = match &current.decision.intent {
            NextIntent::Navigate { label, .. } => Some(label.as_str()),
            _ => None,
        };

        // Flash cleared: non-fragile action, or a *different* fragile label
        // (login gate → home「场馆预约」). Don't burn budget waiting for wanted.
        let left_fragile = !matches!(
            &current.decision.intent,
            NextIntent::Navigate { label, .. } if is_fragile_nav_label(label)
        );
        let other_fragile = matches!(
            &current.decision.intent,
            NextIntent::Navigate { label, .. }
                if fragile_nav_target_changed(wanted_label, label)
        );
        if left_fragile || other_fragile {
            log::info!(
                target: "glance",
                "[nav_confirm] redirected away from fragile nav page={} next={next_lab:?} want={wanted_label} polls={polls}",
                page.as_str()
            );
            return Ok(FragileNav::Redirected(current));
        }

        if let Some(ready) = push_obs(&mut streak, &current) {
            log::info!(
                target: "glance",
                "[nav_confirm] confirmed label={} page={} polls={polls} streak={} elapsed_ms={}",
                ready.0,
                page.as_str(),
                streak.streak(),
                t0.elapsed().as_millis()
            );
            return Ok(FragileNav::Ready(current, ready.0, ready.1, ready.2));
        }
        log::info!(
            target: "glance",
            "[nav_confirm] poll={polls} page={} next={next_lab:?} streak={} elapsed_ms={}",
            page.as_str(),
            streak.streak(),
            t0.elapsed().as_millis()
        );
    }

    // Budget exhausted but still on the same fragile label with partial streak:
    // prefer click if we saw it at least twice (flash would have flipped by now).
    if streak.streak() >= 2 {
        if let NextIntent::Navigate {
            label,
            anchor,
            expected_page,
        } = &current.decision.intent
        {
            if is_fragile_nav_label(label) {
                let lab = label.clone();
                let anc = anchor.clone();
                let exp = *expected_page;
                log::warn!(
                    target: "glance",
                    "[nav_confirm] timeout but streak={} ≥2 → click label={lab}",
                    streak.streak()
                );
                return Ok(FragileNav::Ready(current, lab, anc, exp));
            }
        }
    }

    log::warn!(
        target: "glance",
        "[nav_confirm] timeout want={wanted_label} streak={} polls={polls} → skip click",
        streak.streak()
    );
    Ok(FragileNav::Redirected(current))
}

/// When the input band already shows 4 chars, skip retyping and only click
/// 「完成」(keyboard still open). After dismiss, policy next-steps「确定预约」.
pub(crate) fn finish_captcha_keyboard(
    data_root: &Path,
    bound: &mut BoundWechat,
    mut current: Observed,
    goal: &TaskGoal,
) -> Result<Observed, String> {
    let code = goal
        .recognized_captcha
        .as_deref()
        .filter(|c| c.chars().filter(|ch| ch.is_ascii_alphanumeric()).count() >= 4)
        .ok_or_else(|| "没有找到微信窗口。先打开微信，窗口不要缩到托盘".to_string())?;

    if current.result.snapshot.page != Page::ConfirmOrder
        || !current.result.snapshot.keyboard_open
    {
        return Ok(current);
    }

    let filled = captcha_filled_count(&current.result.snapshot.input_chars);
    // Mid-burst: never trust partial filled (thin `l` under-counts). Once the
    // band clearly shows 4 glyphs, only dismiss the keyboard.
    let plan_filled = if filled >= 4 { 4 } else { 0 };
    let plan = vision_core::plan_captcha_keyboard(
        &current.result.candidate_set,
        code,
        plan_filled,
    )
    .ok_or_else(|| "软键盘上找不到验证码按键".to_string())?;
    let done = plan
        .done
        .ok_or_else(|| "软键盘上找不到「完成」".to_string())?;

    if filled >= 4 {
        log::info!(
            target: "glance",
            "[captcha_keys] input already full filled={filled} — only 完成 @({},{})-({},{})",
            done.anchor.x0,
            done.anchor.y0,
            done.anchor.x1,
            done.anchor.y1
        );
    } else {
        if plan.remaining_keys.len() < 4 {
            return Err(format!(
                "软键盘只定位到 {}/4 个验证码按键",
                plan.remaining_keys.len()
            ));
        }
        let labels: Vec<String> = plan
            .remaining_keys
            .iter()
            .map(|k| k.label.clone())
            .collect();
        log::info!(
            target: "glance",
            "[captcha_keys] burst keys={:?} then 完成",
            labels
        );

        for (i, stroke) in plan.remaining_keys.iter().enumerate() {
            log::info!(
                target: "glance",
                "[captcha_keys] burst={}/4 next={}",
                i + 1,
                stroke.label
            );
            inject_one_click(
                data_root,
                bound,
                &current,
                &stroke.label,
                stroke.anchor.clone(),
                "confirm_order",
                current.decision.confidence,
                None,
            )?;
            thread::sleep(Duration::from_millis(CAPTCHA_KEY_GAP_MS));
        }
    }

    let baseline_fp = crate::rapid_ocr::frame_fingerprint(&current.pixels);
    inject_one_click(
        data_root,
        bound,
        &current,
        &done.label,
        done.anchor,
        "confirm_order",
        current.decision.confidence,
        None,
    )?;
    current = poll_after_click(
        data_root,
        bound,
        goal,
        vision_core::Page::ConfirmOrder,
        &done.label,
        vision_core::Page::ConfirmOrder,
        PollReady::SamePageNextControl,
        baseline_fp,
        None,
    )?;
    Ok(current)
}

pub(crate) fn ensure_captcha_code(goal: &mut TaskGoal, observed: &Observed) -> Result<(), String> {
    if goal
        .recognized_captcha
        .as_deref()
        .map(|c| c.chars().filter(|ch| ch.is_ascii_alphanumeric()).count() >= 4)
        .unwrap_or(false)
    {
        return Ok(());
    }
    let (ocr, _) = ocr_frame(
        observed.frame.pixel_width,
        observed.frame.pixel_height,
        &observed.pixels,
    )?;
    match read_captcha_tile(
        &ocr,
        observed.frame.pixel_width,
        observed.frame.pixel_height,
        &observed.pixels,
    ) {
        Some(code) => {
            goal.recognized_captcha = Some(code);
            Ok(())
        }
        None => Err("验证码未读出四位".into()),
    }
}

/// OCR then burst-type, or click the captcha tile to refresh when OCR fails.
/// Returns `(observation, refreshed)`.
pub(crate) fn type_captcha_or_refresh(
    data_root: &Path,
    bound: &mut BoundWechat,
    observed: Observed,
    mut goal: TaskGoal,
) -> Result<(Observed, bool), String> {
    match ensure_captcha_code(&mut goal, &observed) {
        Ok(()) => {
            let after = finish_captcha_keyboard(data_root, bound, observed, &goal)?;
            Ok((after, false))
        }
        Err(err) => {
            log::warn!(
                target: "glance",
                "[captcha] {err} → click image to refresh (keyboard will close)"
            );
            let after = refresh_captcha_image(data_root, bound, observed, &goal)?;
            Ok((after, true))
        }
    }
}

/// Click the noisy captcha tile (not the input band). Collapses the soft keyboard
/// and asks the page for a new image — same as Python `refresh_captcha`.
pub(crate) fn refresh_captcha_image(
    data_root: &Path,
    bound: &mut BoundWechat,
    current: Observed,
    goal: &TaskGoal,
) -> Result<Observed, String> {
    let anchor = captcha_image_anchor(&current)
        .ok_or_else(|| "找不到验证码图片，无法刷新".to_string())?;
    let baseline_fp = crate::rapid_ocr::frame_fingerprint(&current.pixels);
    inject_one_click(
        data_root,
        bound,
        &current,
        "captcha_image",
        anchor,
        "confirm_order",
        current.decision.confidence,
        None,
    )?;
    let mut cleared = goal.clone();
    cleared.recognized_captcha = None;
    let after = poll_after_click(
        data_root,
        bound,
        &cleared,
        vision_core::Page::ConfirmOrder,
        "captcha_image",
        vision_core::Page::ConfirmOrder,
        PollReady::SamePageNextControl,
        baseline_fp,
        None,
    )?;
    // Tile click animates; give the new image a moment if the keyboard is still up.
    if after.result.snapshot.keyboard_open {
        thread::sleep(Duration::from_millis(900));
        return observe_bound(data_root, bound, &cleared, None);
    }
    Ok(after)
}

pub(crate) fn captcha_image_anchor(observed: &Observed) -> Option<vision_contracts::AnchorBox> {
    let hit = observed
        .result
        .candidate_set
        .items
        .iter()
        .find(|c| c.label == "captcha_image")?;
    let b = hit.visual_bbox.as_ref().unwrap_or(&hit.bbox);
    Some(vision_contracts::AnchorBox {
        x0: b.x0,
        y0: b.y0,
        x1: b.x1,
        y1: b.y1,
    })
}

pub(crate) fn mark_captcha_refreshed(report: &mut GlanceReport) {
    report.recognized_captcha = None;
    report.refuse_reason = Some("CAPTCHA_REFRESHED".into());
    // After refresh the keyboard is down; next glance/click should open input again.
    if !report.next.contains("唤起键盘") {
        report.next = "下一步：点击输入区域唤起键盘".into();
    }
    log::info!(
        target: "glance",
        "[captcha] refreshed tile; keyboard should be closed, code cleared"
    );
}

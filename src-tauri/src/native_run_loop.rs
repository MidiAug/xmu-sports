//! Native vision run loop: RunSession observe → decide_step → execute_pending.
//!
//! Extracted from `lib.rs` (Plan P5) so the host shell stays free of the hot path.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use tauri::Manager;

use crate::{enqueue_pending_evidence, trace_event, AppState};
use crate::{native_run, run_trace};

pub(crate) fn run_still_active(app: &tauri::AppHandle, run_gen: u64) -> bool {
    app.try_state::<AppState>()
        .map(|s| s.run_gen.load(Ordering::SeqCst) == run_gen)
        .unwrap_or(false)
}

pub(crate) fn sleep_cancellable(app: &tauri::AppHandle, run_gen: u64, total_ms: u64) -> bool {
    let mut left = total_ms;
    while left > 0 {
        if !run_still_active(app, run_gen) {
            return false;
        }
        let slice = left.min(200);
        thread::sleep(Duration::from_millis(slice));
        left = left.saturating_sub(slice);
    }
    run_still_active(app, run_gen)
}

fn report_to_snap(
    report: &vision_platform_windows::GlanceReport,
    clicked_from_select_slot: bool,
) -> native_run::GlanceSnap {
    native_run::GlanceSnap {
        page: report.page.clone(),
        can_click: report.can_click,
        intent_kind: report.intent_kind.clone(),
        refuse_reason: report.refuse_reason.clone(),
        wait_ms: report.wait_ms,
        clicked_from_select_slot,
        click_label: report.click_label.clone(),
    }
}

const MAX_CONSECUTIVE_REBINDS: u32 = 2;
const MAX_CONSECUTIVE_REOBSERVES: u32 = 2;

fn pre_input_rebind_reason(error: &str) -> Option<&str> {
    error.strip_prefix(vision_platform_windows::PRE_INPUT_REBIND_PREFIX)
}

fn pre_input_reobserve_reason(error: &str) -> Option<&str> {
    error.strip_prefix(vision_platform_windows::PRE_INPUT_REOBSERVE_PREFIX)
}

fn next_rebind_attempt(consecutive: &mut u32) -> Option<u32> {
    if *consecutive >= MAX_CONSECUTIVE_REBINDS {
        None
    } else {
        *consecutive += 1;
        Some(*consecutive)
    }
}

fn next_reobserve_attempt(consecutive: &mut u32) -> Option<u32> {
    if *consecutive >= MAX_CONSECUTIVE_REOBSERVES {
        None
    } else {
        *consecutive += 1;
        Some(*consecutive)
    }
}

pub(crate) fn run_native_loop(
    app: &tauri::AppHandle,
    run_gen: u64,
    data_root: &Path,
    goal: &mut vision_core::TaskGoal,
    until: native_run::RunUntil,
    trace: &mut run_trace::RunTraceHandle,
) -> Result<String, String> {
    let task_id = trace.task_id().to_string();
    let run_id = trace.run_id().to_string();
    ensure_miniprogram_open(app, run_gen, trace)?;

    const MAX_STEPS: u32 = 120;
    let mut steps = 0u32;
    let mut consecutive_rebinds = 0u32;
    let mut consecutive_reobserves = 0u32;
    let mut readiness_started_at: Option<std::time::Instant> = None;
    let run_t0 = std::time::Instant::now();
    let mode = vision_platform_windows::GlanceMode::run();

    let mut session = vision_platform_windows::RunSession::open(data_root, &run_id).map_err(|e| {
        trace_event(trace, "session_open_failed", None, "failed", None, None);
        e
    })?;

    log::info!(
        target: "vision",
        "[native_loop] start until={until:?} task_id={task_id} run_id={run_id} bind={} wgc={}",
        session.bind_count(),
        session.wgc_open_count()
    );

    loop {
        if until == native_run::RunUntil::HttpEntry && run_t0.elapsed() > Duration::from_secs(60) {
            trace_event(trace, "login_recovery_failed", Some("xmu_login_gate"), "LOGIN_RECOVERY_TIMEOUT", None, None);
            return Err("LOGIN_RECOVERY_TIMEOUT".into());
        }
        if !run_still_active(app, run_gen) {
            trace_event(trace, "cancelled", None, "cancelled", None, None);
            return Err("已取消".into());
        }
        if steps >= MAX_STEPS {
            trace_event(trace, "fail", None, "MAX_STEPS", None, None);
            return Err("步数超限".into());
        }
        steps += 1;
        let step_t0 = std::time::Instant::now();
        let step_s = steps.to_string();

        let mut obs = session.observe(goal, mode).map_err(|e| {
            let code = e.split(':').next().unwrap_or("GLANCE_FAILED");
            trace_event(trace, "glance_failed", None, code, Some(&step_s), None);
            e
        })?;
        if obs.report.intent_kind == "rebind" {
            trace_event(trace, "rebind_requested", Some(obs.report.page.as_str()),
                obs.report.refuse_reason.as_deref().unwrap_or("scene_rebind"), Some(&step_s),
                Some(obs.source_frame_id.as_str()));
            if next_rebind_attempt(&mut consecutive_rebinds).is_none() {
                trace_event(trace, "rebind_exhausted", None, "REBIND_RETRY_EXHAUSTED", Some(&step_s), None);
                return Err("REBIND_RETRY_EXHAUSTED".into());
            }
            log::warn!(target: "vision", "[native_loop] rebind requested attempt={consecutive_rebinds}/{MAX_CONSECUTIVE_REBINDS}");
            session.rebind().map_err(|e| {
                trace_event(trace, "rebind_failed", None, &e, Some(&step_s), None);
                e
            })?;
            trace_event(trace, "rebind_succeeded", None, "OK", Some(&step_s), None);
            continue;
        }
        if let Some(code) = obs.report.recognized_captcha.as_ref() {
            if goal.recognized_captcha.as_deref() != Some(code.as_str()) {
                log::info!(target: "vision", "[captcha] persist code len={}", code.len());
                goal.recognized_captcha = Some(code.clone());
            }
        }

        let glance_reason = obs
            .report
            .refuse_reason
            .clone()
            .unwrap_or_else(|| "ok".into());
        trace_event(
            trace,
            "glance",
            Some(obs.report.page.as_str()),
            &glance_reason,
            Some(&step_s),
            Some(obs.source_frame_id.as_str()),
        );
        let _ = trace.try_observation(steps, "glance", goal, &obs.report);
        log::info!(
            target: "vision",
            "[native_step] n={steps} glance page={} can_click={} refuse={:?} next={} frame={} bind={} wgc={} glance_ms={} total_ms={}",
            obs.report.page,
            obs.report.can_click,
            obs.report.refuse_reason,
            obs.report.next,
            obs.source_frame_id,
            obs.bind_count,
            obs.wgc_open_count,
            step_t0.elapsed().as_millis(),
            run_t0.elapsed().as_millis()
        );

        let snap = report_to_snap(&obs.report, false);
        match native_run::decide_step(until, &snap) {
            native_run::StepAction::Succeed(code) => {
                trace_event(
                    trace,
                    if until == native_run::RunUntil::HttpEntry { "auth_ready" } else { "done" },
                    Some(obs.report.page.as_str()),
                    code,
                    Some(&step_s),
                    Some(obs.source_frame_id.as_str()),
                );
                log::info!(
                    target: "vision",
                    "[native_loop] succeed code={code} page={} bind={} wgc={} total_ms={}",
                    obs.report.page,
                    session.bind_count(),
                    session.wgc_open_count(),
                    run_t0.elapsed().as_millis()
                );
                return Ok(code.to_string());
            }
            native_run::StepAction::Fail(code) => {
                trace_event(
                    trace,
                    "fail",
                    Some(obs.report.page.as_str()),
                    &code,
                    Some(&step_s),
                    Some(obs.source_frame_id.as_str()),
                );
                enqueue_pending_evidence(trace, steps, &code, &mut obs);
                log::warn!(
                    target: "vision",
                    "[native_loop] fail code={code} page={} total_ms={}",
                    obs.report.page,
                    run_t0.elapsed().as_millis()
                );
                return Err(code);
            }
            native_run::StepAction::Wait(ms) => {
                let reason = format!("{ms}ms");
                trace_event(
                    trace,
                    "wait",
                    Some(obs.report.page.as_str()),
                    &reason,
                    Some(&step_s),
                    None,
                );
                if !sleep_cancellable(app, run_gen, ms) {
                    trace_event(trace, "cancelled", None, "cancelled", None, None);
                    return Err("已取消".into());
                }
            }
            native_run::StepAction::Poll(ms) => {
                let soft_polled_ms = readiness_started_at
                    .get_or_insert(step_t0).elapsed().as_millis() as u64;
                if native_run::readiness_expired(0, soft_polled_ms) {
                    trace_event(
                        trace,
                        "fail",
                        Some(obs.report.page.as_str()),
                        "READY_TIMEOUT",
                        Some(&step_s),
                        Some(obs.source_frame_id.as_str()),
                    );
                    enqueue_pending_evidence(trace, steps, "READY_TIMEOUT", &mut obs);
                    return Err("等页面就绪超时（仍像加载中）".into());
                }
                let reason = format!("{ms}ms soft={soft_polled_ms}");
                trace_event(
                    trace,
                    "poll",
                    Some(obs.report.page.as_str()),
                    &reason,
                    Some(&step_s),
                    None,
                );
                if !sleep_cancellable(app, run_gen, ms) {
                    trace_event(trace, "cancelled", None, "cancelled", None, None);
                    return Err("已取消".into());
                }
            }
            native_run::StepAction::Click => {
                readiness_started_at = None;
                let mut from_page = obs.report.page.clone();
                let mut pending_next = obs.report.next.clone();
                let mut token = obs
                    .execution_token
                    .clone()
                    .ok_or_else(|| "NO_EXECUTION_TOKEN".to_string())?;
                loop {
                    if !run_still_active(app, run_gen) {
                        trace_event(trace, "cancelled", None, "cancelled", None, None);
                        return Err("已取消".into());
                    }
                    if steps >= MAX_STEPS {
                        trace_event(trace, "fail", None, "MAX_STEPS", None, None);
                        return Err("步数超限".into());
                    }
                    steps += 1;
                    let step_s = steps.to_string();
                    let click_t0 = std::time::Instant::now();
                    let _ = trace.try_event(
                        "click", Some(from_page.as_str()), pending_next.as_str(),
                        Some(&step_s), Some(obs.source_frame_id.as_str()), None,
                        serde_json::json!({
                            "label": obs.report.click_label,
                            "next": pending_next,
                            "phase": "requested",
                            "input_invoked": false,
                            "bind_count": session.bind_count(),
                        }),
                    );
                    log::info!(
                        target: "vision",
                        "[native_step] n={steps} click phase=requested from={from_page} next={pending_next} token={token} bind={}",
                        session.bind_count()
                    );
                    obs = match session.execute_pending(&token, goal, mode) {
                        Ok(after) => {
                            if after.report.input_invoked {
                                consecutive_rebinds = 0;
                                consecutive_reobserves = 0;
                            }
                            after
                        }
                        Err(e) => {
                            let pre_input = pre_input_rebind_reason(&e).is_some();
                            let reobserve = pre_input_reobserve_reason(&e).is_some();
                            let code = e.split(':').next().unwrap_or("UNKNOWN_CLICK_ERROR");
                            let _ = trace.try_event(
                                "click_failed", Some(from_page.as_str()), code,
                                Some(&step_s), None, None,
                                serde_json::json!({
                                    "detail": e,
                                    "input_status": if pre_input || reobserve { "not_invoked" } else { "unknown" },
                                    "recovery": if pre_input { "rebind_and_reobserve" } else if reobserve { "reobserve" } else { "terminal" },
                                    "bind_count": session.bind_count(),
                                }),
                            );
                            if pre_input {
                                if next_rebind_attempt(&mut consecutive_rebinds).is_none() {
                                    trace_event(trace, "rebind_exhausted", Some(from_page.as_str()),
                                        "REBIND_RETRY_EXHAUSTED", Some(&step_s), None);
                                    return Err(format!("REBIND_RETRY_EXHAUSTED:{e}"));
                                }
                                trace_event(trace, "rebind_requested", Some(from_page.as_str()),
                                    &e, Some(&step_s), None);
                                log::warn!(target: "vision",
                                    "[native_step] pre_input_rebind step={steps} attempt={consecutive_rebinds}/{MAX_CONSECUTIVE_REBINDS} reason={e}");
                                session.rebind().map_err(|rebind_error| {
                                    trace_event(trace, "rebind_failed", Some(from_page.as_str()),
                                        &rebind_error, Some(&step_s), None);
                                    rebind_error
                                })?;
                                trace_event(trace, "rebind_succeeded", Some(from_page.as_str()),
                                    "OK", Some(&step_s), None);
                                break;
                            }
                            if reobserve {
                                if next_reobserve_attempt(&mut consecutive_reobserves).is_none() {
                                    trace_event(trace, "reobserve_exhausted", Some(from_page.as_str()),
                                        "REOBSERVE_RETRY_EXHAUSTED", Some(&step_s), None);
                                    return Err(format!("REOBSERVE_RETRY_EXHAUSTED:{e}"));
                                }
                                trace_event(trace, "reobserve_requested", Some(from_page.as_str()),
                                    &e, Some(&step_s), None);
                                log::warn!(target: "vision",
                                    "[native_step] pre_input_reobserve step={steps} attempt={consecutive_reobserves}/{MAX_CONSECUTIVE_REOBSERVES} reason={e}");
                                break;
                            }
                            return Err(e);
                        }
                    };
                    let after = &obs.report;
                    if after.refuse_reason.as_deref() == Some("CAPTCHA_ERROR_SETTLE")
                        || after.refuse_reason.as_deref() == Some("CAPTCHA_WRONG")
                        || after.refuse_reason.as_deref() == Some("CAPTCHA_REFRESHED")
                    {
                        goal.recognized_captcha = None;
                        log::info!(
                            target: "vision",
                            "[captcha] invalidate reason=host_refuse refuse={:?}",
                            after.refuse_reason
                        );
                    } else if let Some(code) = after.recognized_captcha.as_ref() {
                        goal.recognized_captcha = Some(code.clone());
                    }
                    let after_reason = after
                        .refuse_reason
                        .clone()
                        .unwrap_or_else(|| "ok".into());
                    let _ = trace.try_event(
                        "after_click", Some(after.page.as_str()), &after_reason,
                        Some(&step_s), Some(obs.source_frame_id.as_str()), None,
                        serde_json::json!({
                            "input_invoked": after.input_invoked,
                            "bind_count": obs.bind_count,
                            "wgc_open_count": obs.wgc_open_count,
                        }),
                    );
                    let _ = trace.try_observation(steps, "after_click", goal, after);
                    log::info!(
                        target: "vision",
                        "[native_step] n={steps} after_click page={} can_click={} refuse={:?} frame={} bind={} wgc={} click_ms={} total_ms={}",
                        after.page,
                        after.can_click,
                        after.refuse_reason,
                        obs.source_frame_id,
                        obs.bind_count,
                        obs.wgc_open_count,
                        click_t0.elapsed().as_millis(),
                        run_t0.elapsed().as_millis()
                    );

                    let slot_click = from_page == "select_slot";
                    let after_snap = report_to_snap(after, slot_click);
                    match native_run::decide_step(until, &after_snap) {
                        native_run::StepAction::Succeed(code) => {
                            trace_event(
                                trace,
                                if until == native_run::RunUntil::HttpEntry { "auth_ready" } else { "done" },
                                Some(after.page.as_str()),
                                code,
                                Some(&step_s),
                                Some(obs.source_frame_id.as_str()),
                            );
                            return Ok(code.to_string());
                        }
                        native_run::StepAction::Fail(code) => {
                            trace_event(
                                trace,
                                "fail",
                                Some(after.page.as_str()),
                                &code,
                                Some(&step_s),
                                Some(obs.source_frame_id.as_str()),
                            );
                            enqueue_pending_evidence(trace, steps, &code, &mut obs);
                            return Err(code);
                        }
                        native_run::StepAction::Wait(ms) => {
                            if !sleep_cancellable(app, run_gen, ms) {
                                trace_event(trace, "cancelled", None, "cancelled", None, None);
                                return Err("已取消".into());
                            }
                            break;
                        }
                        native_run::StepAction::Poll(ms) => {
                            let soft_polled_ms = readiness_started_at
                                .get_or_insert(click_t0).elapsed().as_millis() as u64;
                            if native_run::readiness_expired(0, soft_polled_ms) {
                                trace_event(
                                    trace,
                                    "fail",
                                    Some(after.page.as_str()),
                                    "READY_TIMEOUT",
                                    Some(&step_s),
                                    Some(obs.source_frame_id.as_str()),
                                );
                                enqueue_pending_evidence(trace, steps, "READY_TIMEOUT", &mut obs);
                                return Err("等页面就绪超时（仍像加载中）".into());
                            }
                            if !sleep_cancellable(app, run_gen, ms) {
                                trace_event(trace, "cancelled", None, "cancelled", None, None);
                                return Err("已取消".into());
                            }
                            break;
                        }
                        native_run::StepAction::Click => {
                            readiness_started_at = None;
                            from_page = after.page.clone();
                            pending_next = after.next.clone();
                            token = obs
                                .execution_token
                                .clone()
                                .ok_or_else(|| "NO_EXECUTION_TOKEN".to_string())?;
                        }
                    }
                }
            }
        }
    }
}

/// Prefer protocol launch (核对页同款)，不再走发现→小程序→搜索壳导航。
/// 窗体出现后必须置顶：协议常只拉起微信主窗，小程序 HWND 会被挡住。
/// 加载动画由主循环 Poll 重看，不在这里固定长 sleep。
fn ensure_miniprogram_open(
    app: &tauri::AppHandle,
    run_gen: u64,
    trace: &run_trace::RunTraceHandle,
) -> Result<(), String> {
    if !run_still_active(app, run_gen) {
        return Err("已取消".into());
    }
    trace_event(trace, "launch_mp", None, "checking", None, None);

    if miniprogram_visible() {
        match vision_platform_windows::foreground_miniprogram_window() {
            Ok(title) => {
                trace_event(trace, "launch_mp", None, &format!("already_open:{title}"), None, None);
            }
            Err(e) => {
                log::warn!(target: "native_run", "[launch_mp] already_open foreground: {e}");
                trace_event(trace, "launch_mp", None, "already_open", None, None);
            }
        }
        return Ok(());
    }

    let attempts = vision_platform_windows::DEFAULT_LAUNCH_ATTEMPTS;
    let wait_ms = vision_platform_windows::DEFAULT_LAUNCH_WAIT_MS;
    let reason = format!("protocol:attempts={attempts}");
    trace_event(trace, "launch_mp", None, &reason, None, None);
    let launched =
        vision_platform_windows::launch_and_wait_n(None, wait_ms, attempts).map_err(|e| {
            let code = vision_platform_windows::launch_fail_code(&e);
            trace_event(trace, "launch_mp_failed", None, code, None, None);
            e.strip_prefix(&format!("{code}: "))
                .unwrap_or(e.as_str())
                .to_string()
        })?;
    // launch_and_wait already foregrounds; one more pass if WeChat shell reclaimed focus.
    let _ = vision_platform_windows::foreground_miniprogram_window();
    let opened = format!("opened:{}", launched.title);
    trace_event(trace, "launch_mp", None, &opened, None, None);
    Ok(())
}

fn miniprogram_visible() -> bool {
    match vision_platform_windows::discover_wechat_candidates() {
        Ok(found) => found
            .iter()
            .any(|n| n.visible && vision_platform_windows::is_miniprogram_title(&n.title)),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        next_rebind_attempt, next_reobserve_attempt, pre_input_rebind_reason,
        pre_input_reobserve_reason,
    };

    #[test]
    fn only_explicit_pre_input_rejections_are_safe_to_rebind() {
        assert_eq!(
            pre_input_rebind_reason(
                "PRE_INPUT_REBIND_REQUIRED:geometry_or_fingerprint_changed"
            ),
            Some("geometry_or_fingerprint_changed")
        );
        assert_eq!(
            pre_input_rebind_reason("LEASE_REBIND_REQUIRED:geometry_or_fingerprint_changed"),
            None
        );
        assert_eq!(pre_input_rebind_reason("PLAN_EXPIRED"), None);
    }

    #[test]
    fn rebind_retry_budget_is_bounded_until_real_input_succeeds() {
        let mut consecutive = 0;
        assert_eq!(next_rebind_attempt(&mut consecutive), Some(1));
        assert_eq!(next_rebind_attempt(&mut consecutive), Some(2));
        assert_eq!(next_rebind_attempt(&mut consecutive), None);
        assert_eq!(consecutive, 2);
        consecutive = 0; // successful input starts a new recovery window
        assert_eq!(next_rebind_attempt(&mut consecutive), Some(1));
    }

    #[test]
    fn only_explicit_pre_input_staleness_is_safe_to_reobserve() {
        assert_eq!(
            pre_input_reobserve_reason("PRE_INPUT_REOBSERVE_REQUIRED:STALE_FRAME"),
            Some("STALE_FRAME")
        );
        assert_eq!(
            pre_input_reobserve_reason("PRE_INPUT_REOBSERVE_REQUIRED:PLAN_EXPIRED"),
            Some("PLAN_EXPIRED")
        );
        assert_eq!(pre_input_reobserve_reason("STALE_FRAME"), None);
        let mut consecutive = 0;
        assert_eq!(next_reobserve_attempt(&mut consecutive), Some(1));
        assert_eq!(next_reobserve_attempt(&mut consecutive), Some(2));
        assert_eq!(next_reobserve_attempt(&mut consecutive), None);
    }
}

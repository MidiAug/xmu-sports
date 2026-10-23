//! Capture → OCR → shadow observe → decide_next.

#![cfg(windows)]

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use vision_contracts::Frame;
use vision_core::{
    analyze_captcha_input, bundled_shell_templates, classify_outcome, count_input_glyphs,
    decide_next_with_scene, latch_as_str, latch_from_texts, load_shell_templates, merge_latch,
    merge_ocr_hits, merge_ocr_hits_replacing_bands, merge_typed_proxy, need_full_ocr,
    observe_shadow, outcome_as_str, page_roi_bands, refuse_for_outcome, roi_quality_ok,
    should_clear_code, structural_wrong_latch, wrong_wait_ms, NextIntent, ObserveFrameMeta, Page,
    OcrHit, PageRoiBand, PerceptionFeed, SceneState, ShadowObserveInput, ShadowObserveResult,
    ShadowTiming, SubmitLatch, SubmitPre, TaskGoal,
};
use vision_pack::active_pack_dir;

use crate::captcha_dddd::recognize_captcha_bgra;
use crate::glance::bind::{grab_bound_timeout, BoundWechat, Observed};
use crate::glance::report::load_min_confidence;
use crate::ocr::{ocr_frame, ocr_roi_bgra};

pub(crate) const POST_CLICK_POLL_MS: u64 = 250;
/// Cap for post-click polling before accepting the last frame.
pub(crate) const POST_CLICK_POLL_BUDGET_MS: u64 = 6_000;
/// Once submission is sent, only observe its result; never replay a slow request.
pub(crate) const SUBMIT_RESULT_BUDGET_MS: u64 = 30_000;
/// While the screen is still animating, force a full OCR at most this often.
/// Spec §就绪策略: ~0.65s (was 1.2s) so transition frames are not held too long.
pub(crate) const FORCE_OCR_WHILE_MOVING_MS: u64 = 650;
/// After an `unknown` transition frame, retry force-OCR sooner.
pub(crate) const FORCE_OCR_AFTER_UNKNOWN_MS: u64 = 300;
pub(crate) fn elapsed_ms(t: std::time::Instant) -> u128 {
    t.elapsed().as_millis()
}

pub(crate) fn captcha_tile_fingerprint(
    width: u32,
    height: u32,
    pixels: &[u8],
    candidates: &vision_core::CandidateSet,
) -> u64 {
    let bbox = candidates
        .items
        .iter()
        .find(|c| c.label == "captcha_image")
        .map(|c| c.visual_bbox.as_ref().unwrap_or(&c.bbox).clone())
        .or_else(|| None);
    let Some(b) = bbox else {
        return 0;
    };
    region_fingerprint(pixels, width, height, b.x0, b.y0, b.x1, b.y1)
}

pub(crate) fn region_fingerprint(
    pixels: &[u8],
    width: u32,
    height: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
) -> u64 {
    let x0 = x0.clamp(0, width as i32) as u32;
    let y0 = y0.clamp(0, height as i32) as u32;
    let x1 = x1.clamp(0, width as i32) as u32;
    let y1 = y1.clamp(0, height as i32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return 0;
    }
    let mut buf = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for y in y0..y1 {
        let row = (y * width * 4) as usize;
        for x in x0..x1 {
            let i = row + (x * 4) as usize;
            if i + 4 <= pixels.len() {
                buf.extend_from_slice(&pixels[i..i + 4]);
            }
        }
    }
    crate::rapid_ocr::frame_fingerprint(&buf)
}

#[derive(Clone, Copy)]
pub(crate) enum PollReady {
    /// Stop when `snapshot.page` differs from the pre-click page.
    PageChanged,
    /// Also stop when a Navigate intent appears (search results on same page).
    PageChangedOrNavigate,
    /// Same-page UI state change (协议勾选 → 授权登录变蓝). Stop when the next
    /// Navigate/Atomic label differs from what we just clicked, or the page flips.
    SamePageNextControl,
}

pub(crate) fn content_still_settling(obs: &Observed) -> bool {
    if matches!(
        obs.decision.intent,
        NextIntent::Navigate { .. } | NextIntent::AtomicComposite { .. }
    ) {
        return false;
    }
    if matches!(obs.decision.intent, NextIntent::Wait { .. }) {
        return false;
    }
    // New page classified but labels/cells not OCR'd yet (thin frame after navigation).
    matches!(
        obs.decision.refuse_reason.as_deref(),
        Some("VENUE_MISMATCH")
            | Some("SPORT_NOT_ON_SCREEN")
            | Some("ANCHOR_NOT_FOUND")
            | Some("SLOT_GRID_NOT_LOCALIZED")
    )
}

pub(crate) fn poll_after_click(
    data_root: &Path,
    bound: &mut BoundWechat,
    goal: &TaskGoal,
    from_page: vision_core::Page,
    label: &str,
    expected_page: vision_core::Page,
    ready: PollReady,
    baseline_fp: u64,
    submit_pre: Option<SubmitPre>,
) -> Result<Observed, String> {
    let t0 = std::time::Instant::now();
    let budget_ms = if submit_pre.is_some() { SUBMIT_RESULT_BUDGET_MS } else { POST_CLICK_POLL_BUDGET_MS };
    let deadline = t0 + Duration::from_millis(budget_ms);
    let mut polls = 0u32;
    let mut last: Option<Observed> = None;
    let mut prev_fp: Option<u64> = None;
    let mut pending: Option<(Frame, Vec<u8>)> = None;
    let mut ocr_skips = 0u32;
    let mut ocr_runs = 0u32;
    let mut last_ocr_at = t0;
    let mut force_gap_ms = if submit_pre.is_some() {
        TOAST_FORCE_OCR_MS
    } else {
        FORCE_OCR_WHILE_MOVING_MS
    };
    let mut submit_latch = SubmitLatch::None;
    let until = crate::locator_gate::until_after_click(label, expected_page, goal);
    log::info!(
        target: "glance",
        "[poll_after_click] start label={label} from={} expected={} until={:?} budget_ms={} submit_observe={}",
        from_page.as_str(),
        expected_page.as_str(),
        until.labels_for_log(),
        budget_ms,
        submit_pre.is_some()
    );

    while std::time::Instant::now() < deadline {
        let drained = bound.session.drain_ready();
        if drained == 0 {
            thread::sleep(Duration::from_millis(POST_CLICK_POLL_MS));
        }
        polls += 1;

        // Once we already hold a post-click frame, use a short WGC wait: a settled
        // (often near-white) list stops producing frames — that IS the settle signal.
        // SubmitObserve keeps the short wait for the whole budget (toast + structural).
        let wait = if pending.is_some() || last.is_some() || submit_pre.is_some() {
            Duration::from_millis(400)
        } else {
            Duration::from_millis(2_500)
        };

        let grabbed = match grab_bound_timeout(bound, wait) {
            Ok(pair) => Some(pair),
            Err(e) if e.contains("还没有新的一帧") => {
                if let Some((frame, pixels)) = pending.take() {
                    log::info!(
                        target: "glance",
                        "[poll_after_click] label={label} poll={polls} gate=static_no_frame force_ocr drained={drained} elapsed_ms={}",
                        t0.elapsed().as_millis()
                    );
                    Some((frame, pixels))
                } else if let Some(obs) = last.as_ref() {
                    // Retain the last post-click frame after OCR consumed pending.
                    // This is read-only; keep its original id/capture timestamp.
                    Some((obs.frame.clone(), obs.pixels.clone()))
                } else {
                    log::info!(
                        target: "glance",
                        "[poll_after_click] label={label} poll={polls} no_frame drained={drained} elapsed_ms={}",
                        t0.elapsed().as_millis()
                    );
                    continue;
                }
            }
            Err(e) => return Err(e),
        };

        let Some((frame, pixels)) = grabbed else {
            continue;
        };
        let fp = crate::rapid_ocr::frame_fingerprint(&pixels);

        // Cheap gate: identical to pre-click → no OCR.
        // SubmitObserve still force-OCR in the toast window (toast may appear without fp change).
        let in_toast_window =
            submit_pre.is_some() && t0.elapsed() < Duration::from_millis(TOAST_WINDOW_MS);
        if fp == baseline_fp && !in_toast_window {
            ocr_skips += 1;
            prev_fp = Some(fp);
            log::info!(
                target: "glance",
                "[poll_after_click] label={label} poll={polls} gate=unchanged skip_ocr drained={drained} elapsed_ms={}",
                t0.elapsed().as_millis()
            );
            continue;
        }

        let force_ocr = last_ocr_at.elapsed() >= Duration::from_millis(force_gap_ms);
        let stable = prev_fp == Some(fp);
        prev_fp = Some(fp);

        // L2: templates + ROI only on stable/force (budget bench — never every animating frame).
        let locator_hit = if until.is_empty() || !(stable || force_ocr) {
            None
        } else {
            crate::locator_gate::any_locator_hit(
                frame.pixel_width,
                frame.pixel_height,
                &pixels,
                &until,
            )
        };

        // Still moving — keep latest unless force-OCR or an until-anchor is visible.
        // Toast window: never fully skip OCR while force due.
        if !stable && !force_ocr && locator_hit.is_none() {
            pending = Some((frame, pixels));
            ocr_skips += 1;
            log::info!(
                target: "glance",
                "[poll_after_click] label={label} poll={polls} gate=animating skip_ocr drained={drained} elapsed_ms={}",
                t0.elapsed().as_millis()
            );
            continue;
        }
        if let Some(ref hit) = locator_hit {
            log::info!(
                target: "glance",
                "[poll_after_click] label={label} poll={polls} gate=locator_hit hit={hit} until={:?} force_observe elapsed_ms={}",
                until.labels_for_log(),
                t0.elapsed().as_millis()
            );
        }

        pending = None;
        let mut obs = match observe_captured_with_locator_hint(
            data_root,
            bound,
            goal,
            frame,
            pixels,
            locator_hit.as_deref(),
        ) {
            Ok(o) => o,
            Err(e) => return Err(e),
        };
        ocr_runs += 1;
        last_ocr_at = std::time::Instant::now();

        if submit_pre.is_some() {
            let from_refuse = match obs.decision.refuse_reason.as_deref() {
                Some("CAPTCHA_ERROR_SETTLE") => Some(SubmitLatch::Wrong),
                Some("CAPTCHA_NEED_INPUT") => Some(SubmitLatch::Empty),
                _ => None,
            };
            let from_texts = latch_from_texts(
                obs.result
                    .candidate_set
                    .items
                    .iter()
                    .map(|c| c.label.as_str())
                    .chain(toast_needles_from_obs(&obs)),
                obs.result.snapshot.page,
            );
            let pre = submit_pre.expect("submit_pre checked");
            let now_filled = captcha_filled_count(&obs.result.snapshot.input_chars);
            let now_tile = captcha_tile_fingerprint(
                obs.frame.pixel_width,
                obs.frame.pixel_height,
                &obs.pixels,
                &obs.result.candidate_set,
            );
            let tile_changed = now_tile != 0 && pre.tile_fp != 0 && now_tile != pre.tile_fp;
            let from_struct =
                structural_wrong_latch(obs.result.snapshot.page, pre, now_filled, tile_changed);
            let prev = submit_latch;
            submit_latch = merge_latch(
                submit_latch,
                from_refuse.or(from_texts).or(from_struct),
            );
            if submit_latch != prev && submit_latch != SubmitLatch::None {
                log::info!(
                    target: "glance",
                    "[captcha_submit] latch kind={} poll={polls} elapsed_ms={}",
                    latch_as_str(submit_latch),
                    t0.elapsed().as_millis()
                );
            }
        }

        let now_page = obs.result.snapshot.page;
        let changed = now_page != from_page;
        let next_label = match &obs.decision.intent {
            NextIntent::Navigate { label: l, .. } | NextIntent::AtomicComposite { label: l, .. } => {
                Some(l.as_str())
            }
            _ => None,
        };
        let actionable = next_label.is_some();
        let settling = content_still_settling(&obs);
        // Unknown transition frames are not a completed page change (Spec §就绪策略).
        // SubmitObserve: keep toast-rate force OCR for the whole poll (not only first 2.5s).
        if now_page == Page::Unknown {
            force_gap_ms = FORCE_OCR_AFTER_UNKNOWN_MS;
        } else if submit_pre.is_some() {
            force_gap_ms = TOAST_FORCE_OCR_MS;
        } else {
            force_gap_ms = FORCE_OCR_WHILE_MOVING_MS;
        }
        let done = match ready {
            PollReady::PageChanged => {
                if !changed || now_page == Page::Unknown {
                    false
                } else if actionable {
                    true
                } else if settling {
                    false
                } else {
                    true
                }
            }
            PollReady::PageChangedOrNavigate => {
                actionable || (changed && now_page != Page::Unknown && !settling)
            }
            PollReady::SamePageNextControl => {
                if changed && now_page != Page::Unknown && !settling {
                    true
                } else if let Some(next) = next_label {
                    next != label
                } else {
                    matches!(obs.decision.intent, NextIntent::Wait { .. })
                }
            }
        };
        // SubmitObserve: WRONG/EMPTY Wait counts as done so we don't timeout past toast.
        let submit_done = submit_pre.is_some()
            && matches!(
                submit_latch,
                SubmitLatch::Wrong | SubmitLatch::Empty | SubmitLatch::Success
            )
            && (matches!(obs.decision.intent, NextIntent::Wait { .. })
                || matches!(submit_latch, SubmitLatch::Success)
                || !settling);
        log::info!(
            target: "glance",
            "[poll_after_click] label={label} poll={polls} from={} now={} changed={changed} actionable={actionable} next={:?} settling={settling} refuse={:?} input_chars={:?} filled={} drained={drained} ocr_runs={ocr_runs} ocr_skips={ocr_skips} latch={} elapsed_ms={}",
            from_page.as_str(),
            now_page.as_str(),
            next_label,
            obs.decision.refuse_reason,
            obs.result.snapshot.input_chars,
            captcha_filled_count(&obs.result.snapshot.input_chars),
            latch_as_str(submit_latch),
            t0.elapsed().as_millis()
        );
        if (submit_pre.is_none() && done) || submit_done {
            if let Some(pre) = submit_pre {
                apply_submit_outcome(&mut obs, pre, submit_latch);
            }
            return Ok(obs);
        }
        last = Some(obs);
    }
    log::warn!(
        target: "glance",
        "[poll_after_click] timeout label={label} from={} polls={} ocr_runs={ocr_runs} ocr_skips={ocr_skips} latch={} elapsed_ms={}",
        from_page.as_str(),
        polls,
        latch_as_str(submit_latch),
        t0.elapsed().as_millis()
    );
    if let Some(mut obs) = last {
        if let Some(pre) = submit_pre {
            apply_submit_outcome(&mut obs, pre, submit_latch);
        }
        return Ok(obs);
    }
    // Prefer the last post-click pixels over another WGC grab (often fails on static UI).
    if let Some((frame, pixels)) = pending {
        let mut obs = observe_captured(data_root, bound, goal, frame, pixels, None)?;
        if let Some(pre) = submit_pre {
            apply_submit_outcome(&mut obs, pre, submit_latch);
        }
        return Ok(obs);
    }
    let mut obs = observe_bound(data_root, bound, goal, None)?;
    if let Some(pre) = submit_pre {
        apply_submit_outcome(&mut obs, pre, submit_latch);
    }
    Ok(obs)
}

pub(crate) const TOAST_WINDOW_MS: u64 = 2_500;
pub(crate) const TOAST_FORCE_OCR_MS: u64 = 280;

pub(crate) fn toast_needles_from_obs(obs: &Observed) -> impl Iterator<Item = &str> {
    // refuse_reason / now_line may already carry toast text from decide_next.
    let mut out: Vec<&str> = Vec::new();
    if let Some(r) = obs.decision.refuse_reason.as_deref() {
        out.push(r);
    }
    out.push(obs.decision.now_line.as_str());
    out.push(obs.decision.next_line.as_str());
    // Snapshot flags if classify ever wires toast_kind.
    if obs.result.snapshot.toast_error {
        out.push("验证码错误");
    }
    if obs.result.snapshot.toast_need_input {
        out.push("请输入验证码");
    }
    out.into_iter()
}

pub(crate) fn apply_submit_outcome(obs: &mut Observed, pre: SubmitPre, latch: SubmitLatch) {
    let now_filled = captcha_filled_count(&obs.result.snapshot.input_chars);
    let now_tile = captcha_tile_fingerprint(
        obs.frame.pixel_width,
        obs.frame.pixel_height,
        &obs.pixels,
        &obs.result.candidate_set,
    );
    let tile_changed = now_tile != 0 && pre.tile_fp != 0 && now_tile != pre.tile_fp;
    let input_empty = now_filled == 0;
    let toast_visible = matches!(
        latch_from_texts(
            obs.result
                .candidate_set
                .items
                .iter()
                .map(|c| c.label.as_str())
                .chain(toast_needles_from_obs(obs)),
            obs.result.snapshot.page,
        ),
        Some(SubmitLatch::Wrong | SubmitLatch::Empty)
    );
    let outcome = classify_outcome(latch, obs.result.snapshot.page, pre, now_filled, tile_changed);
    log::info!(
        target: "glance",
        "[captcha_submit] effect input_empty={} tile_changed={} toast_visible={} page={} latch={}",
        input_empty as u8,
        tile_changed as u8,
        toast_visible as u8,
        obs.result.snapshot.page.as_str(),
        latch_as_str(latch)
    );
    let refuse = refuse_for_outcome(outcome);
    let clear = should_clear_code(outcome);
    let wait_ms = match outcome {
        vision_core::SubmitOutcome::Wrong => wrong_wait_ms(tile_changed, input_empty),
        vision_core::SubmitOutcome::Empty => 1_500,
        _ => 0,
    };
    log::info!(
        target: "glance",
        "[captcha_submit] outcome={} clear_code={} wait_desired={} refuse={:?} wait_ms={}",
        outcome_as_str(outcome),
        clear as u8,
        matches!(outcome, vision_core::SubmitOutcome::Wrong) as u8,
        refuse,
        wait_ms
    );
    if clear {
        log::info!(
            target: "glance",
            "[captcha] invalidate reason={}",
            if matches!(latch, SubmitLatch::Wrong) {
                "toast_wrong"
            } else {
                "struct_tile_change"
            }
        );
        obs.decision.goal.recognized_captcha = None;
    }
    match outcome {
        vision_core::SubmitOutcome::Wrong => {
            obs.decision.refuse_reason = Some("CAPTCHA_ERROR_SETTLE".into());
            obs.decision.intent = NextIntent::Wait {
                reason: "CAPTCHA_ERROR_SETTLE".into(),
                approx_ms: wait_ms,
            };
            obs.decision.next_line = "下一步：点击输入区域唤起键盘".into();
            obs.decision.now_line = "现在：确认预约".into();
        }
        vision_core::SubmitOutcome::Empty => {
            obs.decision.refuse_reason = Some("CAPTCHA_NEED_INPUT".into());
            obs.decision.intent = NextIntent::Wait {
                reason: "CAPTCHA_NEED_INPUT".into(),
                approx_ms: wait_ms,
            };
            obs.decision.next_line = "下一步：点击输入区域唤起键盘".into();
        }
        vision_core::SubmitOutcome::Unknown => {
            obs.decision.refuse_reason = Some("CAPTCHA_SUBMIT_UNKNOWN".into());
            obs.decision.intent = NextIntent::Stop {
                reason: "CAPTCHA_SUBMIT_UNKNOWN".into(),
            };
            obs.decision.next_line = "下一步：提交结果不明，禁止盲点确定预约".into();
        }
        vision_core::SubmitOutcome::Success => {
            // Stop even if page taxonomy misfired (venue_list vs my_reservations).
            obs.decision.refuse_reason = Some("ALREADY_DONE".into());
            obs.decision.intent = NextIntent::Stop {
                reason: "ALREADY_DONE".into(),
            };
            obs.decision.now_line = "现在：已提交".into();
            obs.decision.next_line = "下一步：停止，预约已提交".into();
        }
    }
}

pub(crate) fn read_captcha_tile(
    ocr: &vision_core::OcrRegionCache,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Option<String> {
    let fw = width as i32;
    let fh = height as i32;
    let (x0, y0, cw, ch) = if let Some(state) =
        vision_core::analyze_captcha_input(pixels, fw, fh, ocr)
    {
        let b = &state.image;
        (
            b.x0.max(0) as u32,
            b.y0.max(0) as u32,
            b.w().max(1) as u32,
            b.h().max(1) as u32,
        )
    } else {
        let label = ocr
            .hits()
            .iter()
            .find(|h| h.text.contains("验证码") && h.score >= 0.45)?;
        let x0 = (label.x + label.w + 2).max(0) as u32;
        let y0 = (label.y - 2).max(0) as u32;
        let cw = (label.h as u32).saturating_mul(4).clamp(48, 180);
        let ch = (label.h as u32).saturating_mul(2).clamp(20, 72);
        (x0, y0, cw, ch)
    };
    if x0.saturating_add(cw) > width || y0.saturating_add(ch) > height {
        log::warn!(
            target: "glance",
            "[captcha] crop out of bounds x0={x0} y0={y0} cw={cw} ch={ch} frame={width}x{height}"
        );
        return None;
    }
    log::info!(
        target: "glance",
        "[captcha] crop x0={x0} y0={y0} {}x{}",
        cw,
        ch
    );
    let mut crop = Vec::with_capacity((cw * ch * 4) as usize);
    for y in y0..y0 + ch {
        for x in x0..x0 + cw {
            let i = ((y * width + x) * 4) as usize;
            if i + 4 > pixels.len() {
                return None;
            }
            crop.extend_from_slice(&pixels[i..i + 4]);
        }
    }
    match recognize_captcha_bgra(cw, ch, &crop) {
        Ok(code) if code.chars().count() >= 4 => {
            log::info!(target: "glance", "[captcha] recognized len={}", code.len());
            Some(code.chars().take(4).collect())
        }
        Ok(code) => {
            log::warn!(target: "glance", "[captcha] short ocr len={}", code.len());
            None
        }
        Err(e) => {
            log::warn!(target: "glance", "[captcha] ocr failed: {e}");
            None
        }
    }
}

/// Fill `goal.recognized_captcha` from the current frame when still missing.
pub(crate) fn observe_bound(
    data_root: &Path,
    bound: &mut BoundWechat,
    goal: &TaskGoal,
    scene: Option<&mut SceneState>,
) -> Result<Observed, String> {
    let (frame, pixels) = match grab_bound_timeout(bound, Duration::from_millis(2_500)) {
        Ok(pair) => pair,
        Err(e) if e == "还没有新的一帧" => {
            bound.restart_capture()?;
            grab_bound_timeout(bound, Duration::from_millis(1_000))?
        }
        Err(e) => return Err(e),
    };
    observe_captured(data_root, bound, goal, frame, pixels, scene)
}

pub(crate) fn observe_bound_timeout(
    data_root: &Path,
    bound: &mut BoundWechat,
    goal: &TaskGoal,
    scene: Option<&mut SceneState>,
    wait: Duration,
) -> Result<Observed, String> {
    let (frame, pixels) = grab_bound_timeout(bound, wait)?;
    observe_captured(data_root, bound, goal, frame, pixels, scene)
}

pub(crate) fn observe_captured(
    data_root: &Path,
    bound: &BoundWechat,
    goal: &TaskGoal,
    frame: Frame,
    pixels: Vec<u8>,
    scene: Option<&mut SceneState>,
) -> Result<Observed, String> {
    observe_captured_with_prefilled_ocr(data_root, bound, goal, frame, pixels, scene, None)
}

/// A locator only gates readiness; the cropped OCR must still produce the
/// expected page and action. Otherwise redo the same frame with full OCR.
fn observe_captured_with_locator_hint(
    data_root: &Path,
    bound: &BoundWechat,
    goal: &TaskGoal,
    frame: Frame,
    pixels: Vec<u8>,
    locator_hit: Option<&str>,
) -> Result<Observed, String> {
    let Some(label) = locator_hit else {
        return observe_captured(data_root, bound, goal, frame, pixels, None);
    };
    let Some(band) = locator_ocr_band(label, goal) else {
        return observe_captured(data_root, bound, goal, frame, pixels, None);
    };
    let quick = match ocr_roi_bgra(frame.pixel_width, frame.pixel_height, &pixels, band) {
        Ok((hits, ms)) => {
            let region_hits = hits.len();
            log::info!(target: "glance",
                "[locator_ocr] label={label} mode=roi hits={region_hits} ocr_ms={ms} band=({:.2},{:.2},{:.2},{:.2})",
                band.x0, band.y0, band.x1, band.y1);
            observe_captured_with_prefilled_ocr(
                data_root, bound, goal, frame, pixels, None, Some((hits, ms)),
            )?
        }
        Err(e) => {
            log::warn!(target: "glance", "[locator_ocr] label={label} mode=full reason=roi_error detail={e}");
            return observe_captured(data_root, bound, goal, frame, pixels, None);
        }
    };
    let expected_action = if label == "预约须知" { "我知道了" } else { label };
    let matched = matches!(
        &quick.decision.intent,
        NextIntent::Navigate { label: next, .. } if next == expected_action
    ) && quick.decision.confidence >= load_min_confidence(data_root)
        && match expected_action {
            "我知道了" => quick.result.snapshot.page == Page::BookingNotice,
            _ => quick.result.snapshot.page == Page::VenueSportList,
        };
    if matched {
        log::info!(target: "glance", "[locator_ocr] label={label} outcome=accepted page={} next={expected_action}", quick.result.snapshot.page.as_str());
        Ok(quick)
    } else {
        log::info!(target: "glance", "[locator_ocr] label={label} outcome=full_fallback page={} confidence={:.3}", quick.result.snapshot.page.as_str(), quick.decision.confidence);
        observe_captured(data_root, bound, goal, quick.frame, quick.pixels, None)
    }
}

fn locator_ocr_band(label: &str, goal: &TaskGoal) -> Option<PageRoiBand> {
    match label {
        "我知道了" | "预约须知" => Some(PageRoiBand {
            // Notice title proves page identity; the blue ack button is grounded
            // by the existing color candidate even if its white text is missed.
            x0: 0.08, y0: 0.12, x1: 0.92, y1: 0.40, scale: 1.0,
        }),
        sport if goal.sport.as_deref() == Some(sport) => Some(PageRoiBand {
            // Sport captions and icons occupy the upper grid, not venue rows.
            x0: 0.02, y0: 0.10, x1: 0.98, y1: 0.61, scale: 1.0,
        }),
        _ => None,
    }
}

fn observe_captured_with_prefilled_ocr(
    data_root: &Path,
    bound: &BoundWechat,
    goal: &TaskGoal,
    frame: Frame,
    pixels: Vec<u8>,
    mut scene: Option<&mut SceneState>,
    prefilled_ocr: Option<(Vec<OcrHit>, u64)>,
) -> Result<Observed, String> {
    let min_conf = load_min_confidence(data_root);
    let w = frame.pixel_width;
    let h = frame.pixel_height;
    let now_ms = frame.monotonic_capture_time_ms;

    let force_zoom = scene
        .as_ref()
        .map(|s| s.pending_date_roi_zoom)
        .unwrap_or(false);
    let (cached_page, cache_age, cache_q, has_hits) = match scene.as_ref().and_then(|s| s.ocr_cache.as_ref()) {
        Some(c) => (
            Some(c.page),
            Some(now_ms.saturating_sub(c.full_monotonic_ms)),
            Some(c.quality),
            !c.hits.is_empty(),
        ),
        None => (None, None, None, false),
    };
    let has_prefilled_ocr = prefilled_ocr.is_some();
    let mut do_full = !has_prefilled_ocr
        && need_full_ocr(cached_page, cache_age, cache_q, force_zoom, has_hits);

    let mut ocr_full_ms = 0u64;
    let (mut hits, mut ocr_roi_ms) = prefilled_ocr.unwrap_or_default();

    if has_prefilled_ocr {
        // The caller validates page + next action and falls back to full OCR if weak.
    } else if do_full {
        let t_full = Instant::now();
        let (full, _) = ocr_frame(w, h, &pixels)?;
        ocr_full_ms = t_full.elapsed().as_millis() as u64;
        hits = full.hits().to_vec();
    } else if let Some(page) = cached_page {
        let base = scene
            .as_ref()
            .and_then(|s| s.ocr_cache.as_ref())
            .map(|c| c.hits.clone())
            .unwrap_or_default();
        let mut roi_hits = Vec::new();
        for band in page_roi_bands(page) {
            match ocr_roi_bgra(w, h, &pixels, band) {
                Ok((mapped, ms)) => {
                    ocr_roi_ms = ocr_roi_ms.saturating_add(ms);
                    roi_hits.extend(mapped);
                }
                Err(e) => {
                    log::warn!(target: "glance", "[ocr_roi] failed: {e}");
                    do_full = true;
                    break;
                }
            }
        }
        if force_zoom {
            // Date-strip recovery: force VenueDetail date band at higher scale.
            let zoom = vision_core::PageRoiBand {
                x0: 0.02,
                y0: 0.08,
                x1: 0.98,
                y1: 0.45,
                scale: 2.0,
            };
            if let Ok((mapped, ms)) = ocr_roi_bgra(w, h, &pixels, zoom) {
                ocr_roi_ms = ocr_roi_ms.saturating_add(ms);
                roi_hits.extend(mapped);
            }
        }
        if !do_full && roi_quality_ok(&roi_hits, 3, 0.45) {
            let bands = page_roi_bands(page);
            hits = merge_ocr_hits_replacing_bands(&base, &roi_hits, &bands, w, h, 0.3);
        } else {
            do_full = true;
        }
    } else {
        do_full = true;
    }

    if do_full && ocr_full_ms == 0 {
        let t_full = Instant::now();
        let (full, _) = ocr_frame(w, h, &pixels)?;
        ocr_full_ms = t_full.elapsed().as_millis() as u64;
        hits = full.hits().to_vec();
    } else if do_full && force_zoom {
        // Full already ran; still merge an upscaled date strip for recovery.
        let zoom = vision_core::PageRoiBand {
            x0: 0.02,
            y0: 0.08,
            x1: 0.98,
            y1: 0.45,
            scale: 2.0,
        };
        if let Ok((mapped, ms)) = ocr_roi_bgra(w, h, &pixels, zoom) {
            ocr_roi_ms = ocr_roi_ms.saturating_add(ms);
            hits = merge_ocr_hits(&hits, &mapped, 0.3);
        }
    }

    if let Some(s) = scene.as_mut() {
        s.pending_date_roi_zoom = false;
    }

    let ocr = vision_core::OcrRegionCache::from_hits(hits);
    let ocr_ms = ocr_full_ms.saturating_add(ocr_roi_ms);
    let timing_seed = ShadowTiming {
        ocr_ms,
        ocr_full_ms,
        ocr_roi_ms,
        ..ShadowTiming::default()
    };

    let mut goal = goal.clone();
    if goal.recognized_captcha.is_none() {
        if let Some(code) = read_captcha_tile(&ocr, w, h, &pixels) {
            goal.recognized_captcha = Some(code);
        }
    }
    let mut templates = bundled_shell_templates();
    templates.extend(
        active_pack_dir(data_root)
            .map(|p| load_shell_templates(&p.join("templates").join("wechat_shell")))
            .unwrap_or_default(),
    );
    let template_count = templates.len();
    let run_id = if bound.run_id.is_empty() {
        "ui-glance"
    } else {
        bound.run_id.as_str()
    };
    let mut result = observe_shadow(ShadowObserveInput {
        run_id,
        lease_id: &bound.lease.lease_id,
        generation: bound.lease.generation,
        frame_id: &frame.frame_id,
        frame_w: w as i32,
        frame_h: h as i32,
        ocr: &ocr,
        title_hint: bound.lease.title_hint.clone(),
        min_confidence: min_conf,
        target_slot_time: goal.slot_time.as_deref(),
        timing: timing_seed,
        perception: PerceptionFeed {
            pixels_bgra: Some(pixels.as_slice()),
            detections: &[],
            templates: &templates,
        },
    });
    if result.input_invoked {
        return Err("input invoked during observe".into());
    }
    // Full-page OCR misses tiny slot glyphs; crop+upscale the orange input band.
    refine_captcha_input_chars(&mut result, &ocr, w, h, &pixels);

    // Refresh OCR cache after classify when we ran (or upgraded to) full page.
    if ocr_full_ms > 0 {
        if let Some(s) = scene.as_mut() {
            s.remember_full_ocr(
                result.snapshot.page,
                ocr.hits().to_vec(),
                now_ms,
                result.snapshot.confidence.max(0.0),
            );
        }
    } else if let Some(s) = scene.as_mut() {
        if let Some(cache) = s.ocr_cache.as_mut() {
            cache.page = result.snapshot.page;
            cache.quality = result.snapshot.confidence.max(0.0);
        }
    }

    let meta = ObserveFrameMeta {
        frame_id: frame.frame_id.clone(),
        capture_monotonic_ms: now_ms,
        frame_w: w as i32,
        frame_h: h as i32,
        pixels_bgra: Some(pixels.as_slice()),
    };
    let decision = decide_next_with_scene(
        &result.snapshot,
        &result.candidate_set,
        &goal,
        scene,
        Some(&meta),
    );
    Ok(Observed {
        frame,
        pixels,
        ocr,
        result,
        decision,
        template_count,
    })
}

pub(crate) fn captcha_filled_count(input_chars: &str) -> usize {
    input_chars
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == 'X')
        .count()
        .min(4)
}

/// Crop the captcha input band (right of the tile), upscale, OCR — same as Python
/// `read_captcha_input_text`. Overrides whole-frame guesses that miss typed glyphs.
pub(crate) fn refine_captcha_input_chars(
    result: &mut ShadowObserveResult,
    ocr: &vision_core::OcrRegionCache,
    width: u32,
    height: u32,
    pixels: &[u8],
) {
    let fw = width as i32;
    let fh = height as i32;
    let Some(state) = analyze_captcha_input(pixels, fw, fh, ocr) else {
        return;
    };
    let ink = count_input_glyphs(pixels, fw, fh, &state.input_region);
    let crop_text = ocr_input_band_crop(width, height, pixels, &state.input_region).unwrap_or_default();
    let before = result.snapshot.input_chars.clone();
    // Do not carry forward whole-frame leaks via `before` — crop+ink are the source of truth.
    let merged = merge_typed_proxy(&crop_text, ink);
    let final_text = {
        let crop_n = captcha_filled_count(&crop_text);
        let merged_n = captcha_filled_count(&merged);
        if crop_n >= merged_n && crop_n > 0 {
            crop_text
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_uppercase())
                .take(4)
                .collect()
        } else {
            merged
        }
    };
    if final_text != before {
        log::info!(
            target: "glance",
            "[captcha_input] before={before:?} crop_ocr={crop_text:?} ink={ink} final={final_text:?}"
        );
        result.snapshot.input_chars = final_text;
    } else {
        log::info!(
            target: "glance",
            "[captcha_input] chars={before:?} crop_ocr={crop_text:?} ink={ink}"
        );
    }
}

pub(crate) fn ocr_input_band_crop(
    width: u32,
    height: u32,
    pixels: &[u8],
    region: &vision_core::BBox,
) -> Option<String> {
    let x0 = region.x0.max(0) as u32;
    let y0 = region.y0.max(0) as u32;
    let x1 = region.x1.clamp(region.x0 + 1, width as i32) as u32;
    let y1 = region.y1.clamp(region.y0 + 1, height as i32) as u32;
    if x1 <= x0 + 4 || y1 <= y0 + 4 {
        return None;
    }
    let cw = x1 - x0;
    let ch = y1 - y0;
    let mut crop = Vec::with_capacity((cw * ch * 4) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * width + x) * 4) as usize;
            if i + 4 > pixels.len() {
                return None;
            }
            crop.extend_from_slice(&pixels[i..i + 4]);
        }
    }
    // Upscale small slot strip so RapidOCR can read typed glyphs (Python fx=2.5).
    let scale = if cw.max(ch) < 160 { 2.5f32 } else { 1.5f32 };
    let rgb = upscale_bgra_crop(cw, ch, &crop, scale)?;
    match crate::rapid_ocr::recognize_rgb_text(&rgb) {
        Ok(raw) => {
            let text: String = raw
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_uppercase())
                .take(4)
                .collect();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        Err(e) => {
            log::warn!(target: "glance", "[captcha_input] crop ocr failed: {e}");
            None
        }
    }
}

pub(crate) fn upscale_bgra_crop(width: u32, height: u32, bgra: &[u8], scale: f32) -> Option<image::RgbImage> {
    use image::{imageops, RgbImage};
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for px in bgra.chunks_exact(4) {
        rgb.push(px[2]);
        rgb.push(px[1]);
        rgb.push(px[0]);
    }
    let src = RgbImage::from_raw(width, height, rgb)?;
    let nw = ((width as f32) * scale).round().max(1.0) as u32;
    let nh = ((height as f32) * scale).round().max(1.0) as u32;
    Some(imageops::resize(
        &src,
        nw,
        nh,
        imageops::FilterType::CatmullRom,
    ))
}

#[cfg(test)]
mod locator_roi_tests {
    use super::locator_ocr_band;
    use vision_core::{NextIntent, Page, TaskGoal};

    #[test]
    fn locator_roi_only_crops_known_next_controls() {
        let mut goal = TaskGoal::default();
        goal.sport = Some("健身房".into());
        let notice = locator_ocr_band("我知道了", &goal).unwrap();
        assert!(notice.y1 <= 0.40);
        let sport = locator_ocr_band("健身房", &goal).unwrap();
        assert!(sport.y1 <= 0.61);
        assert!(locator_ocr_band("羽毛球", &goal).is_none());
    }

    #[test]
    fn sport_roi_smoke_when_real_image_is_set() {
        let Ok(path) = std::env::var("XMU_OCR_SMOKE") else {
            return;
        };
        let rgb = image::open(&path).expect("open smoke image").to_rgb8();
        let (w, h) = rgb.dimensions();
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for px in rgb.pixels() {
            bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
        let mut goal = TaskGoal::default();
        goal.sport = Some("健身房".into());
        let band = locator_ocr_band("健身房", &goal).unwrap();
        let (hits, ms) = super::ocr_roi_bgra(w, h, &bgra, band).expect("ROI OCR");
        eprintln!("sport ROI {ms}ms: {:?}", hits.iter().map(|h| &h.text).collect::<Vec<_>>());
        let mut changed = bgra.clone();
        let inside = (((h as f32 * 0.20) as u32 * w + (w / 2)) * 4) as usize;
        changed[inside] ^= 1;
        let (_, warm_ms) = super::ocr_roi_bgra(w, h, &changed, band).expect("warm ROI OCR");
        let full_t0 = std::time::Instant::now();
        let _ = super::ocr_frame(w, h, &bgra).expect("full OCR");
        eprintln!("warm sport ROI {warm_ms}ms, full OCR {}ms", full_t0.elapsed().as_millis());
        let ocr = vision_core::OcrRegionCache::from_hits(hits);
        let result = vision_core::observe_shadow(vision_core::ShadowObserveInput {
            run_id: "smoke", lease_id: "lease", generation: 1, frame_id: "frame",
            frame_w: w as i32, frame_h: h as i32, ocr: &ocr,
            title_hint: "厦大体育".into(),
            min_confidence: vision_core::DEFAULT_MIN_CONFIDENCE,
            target_slot_time: None,
            timing: vision_core::ShadowTiming::default(),
            perception: vision_core::PerceptionFeed {
                pixels_bgra: Some(&bgra), detections: &[], templates: &[],
            },
        });
        assert_eq!(result.snapshot.page, Page::VenueSportList);
        let decision = vision_core::decide_next(&result.snapshot, &result.candidate_set, &goal);
        assert!(matches!(decision.intent, vision_core::NextIntent::Navigate { ref label, .. } if label == "健身房"));
    }

    #[test]
    fn date_strip_smoke_when_real_image_is_set() {
        let Ok(path) = std::env::var("XMU_DATE_SMOKE") else {
            return;
        };
        let rgb = image::open(&path).expect("open date screenshot").to_rgb8();
        let (w, h) = rgb.dimensions();
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for px in rgb.pixels() {
            bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
        let (ocr, _) = super::ocr_frame(w, h, &bgra).expect("date OCR");
        eprintln!("date OCR: {:?}", ocr.hits().iter().map(|h| &h.text).collect::<Vec<_>>());
        let result = vision_core::observe_shadow(vision_core::ShadowObserveInput {
            run_id: "date-smoke", lease_id: "lease", generation: 1, frame_id: "frame",
            frame_w: w as i32, frame_h: h as i32, ocr: &ocr,
            title_hint: "厦大体育".into(),
            min_confidence: vision_core::DEFAULT_MIN_CONFIDENCE,
            target_slot_time: None,
            timing: vision_core::ShadowTiming::default(),
            perception: vision_core::PerceptionFeed {
                pixels_bgra: Some(&bgra), detections: &[], templates: &[],
            },
        });
        let mut cards = vision_core::locate_date_cards_ex(&result.candidate_set, w as i32);
        vision_core::enrich_date_cards_color(&mut cards, Some(&bgra), w, h);
        let cov = vision_core::build_date_strip_coverage(&cards, "2026-10-01", w as i32, h as i32);
        eprintln!("date cards: {cards:?} coverage: {cov:?}");
        assert_eq!(result.snapshot.page, Page::VenueDetail);
        assert!(cov.is_sufficient());
        assert!(matches!(
            vision_core::assess_frame_date(&cards, "2026-10-01", &cov),
            vision_core::FrameDateEvidence::UnavailableCandidate(_)
        ));
        let mut goal = TaskGoal::default();
        goal.venue = Some("翔安校区健身房".into());
        goal.date = Some("2026-10-01".into());
        let mut scene = vision_core::SceneState::new();
        let mut last = None;
        for n in 0..3 {
            let meta = vision_core::ObserveFrameMeta {
                frame_id: format!("capture-{n}"), capture_monotonic_ms: 1_000 + n * 500,
                frame_w: w as i32, frame_h: h as i32, pixels_bgra: Some(&bgra),
            };
            last = Some(vision_core::decide_next_with_scene(
                &result.snapshot, &result.candidate_set, &goal, Some(&mut scene), Some(&meta),
            ));
        }
        assert!(matches!(
            last.unwrap().intent,
            NextIntent::Stop { ref reason } if reason == "DATE_UNAVAILABLE"
        ));
    }
}

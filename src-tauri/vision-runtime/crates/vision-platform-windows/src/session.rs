//! Persistent WeChat vision session: one bind + WGC across observe/execute steps,
//! with a bounded fresh-capture attempt when change-driven WGC becomes static.
//!
//! Thread constraint: use only on the native-run / WinRT worker thread that
//! created the session. Do not move WGC / WinRT objects across threads.

use std::path::{Path, PathBuf};
use std::time::Duration;

use vision_contracts::{monotonic_now_ms, ActionType, SizeI};
use vision_core::{
    build_click_plan, is_fragile_nav_label, pending_kind_from_click, AtomicStep, NextIntent,
    Page, PendingError, PendingKind, PendingStore, SceneState, TaskGoal, DEFAULT_PLAN_TTL_MS,
    FRAGILE_NAV_STREAK_NEED,
};

#[cfg(windows)]
use crate::glance::{
    bind_wechat, bind_wechat_with_generation, build_report, click_from_observed, ensure_winrt_thread,
    observe_bound_timeout, terminal_evidence_payload, BoundWechat, GlanceMode, GlanceReport, Observed,
};

const DATE_STATIC_GRAB_MS: u64 = 400;
const DATE_RESTART_FIRST_FRAME_MS: u64 = 1_000;

fn needs_static_date_refresh(page: Page, reason: Option<&str>) -> bool {
    page == Page::VenueDetail && reason == Some("DATE_EVIDENCE_WEAK")
}

fn navigation_still_pending(from: Page, expected: Page, label: &str, page: Page, next: Option<&str>) -> bool {
    expected != from && page == from && next == Some(label)
}

#[cfg(windows)]
struct NavigationWait {
    from: Page,
    expected: Page,
    label: String,
    deadline_ms: u64,
}

/// Raw BGRA + overlay boxes for async terminal evidence encoding (run mode).
#[derive(Debug, Clone)]
pub struct PendingEvidenceFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub boxes: Vec<vision_core::CandidateBox>,
}

/// Host-facing observation result. Opaque `execution_token` must be passed back
/// to [`RunSession::execute_pending`] — never rebuild actions from the report.
#[derive(Debug)]
pub struct SessionObservation {
    pub report: GlanceReport,
    pub execution_token: Option<String>,
    pub bind_count: u32,
    pub wgc_open_count: u32,
    pub source_frame_id: String,
    /// Present on terminal decisions in run mode (`include_image == false`).
    pub pending_evidence: Option<PendingEvidenceFrame>,
}

/// Cross-step WeChat vision session (process-local; not vision-worker).
#[cfg(windows)]
    pub struct RunSession {
    data_root: PathBuf,
    run_id: String,
    bound: BoundWechat,
    pending: PendingStore,
    scene: SceneState,
    last_observed: Option<Observed>,
    bind_count: u32,
    date_static_refreshes: u32,
    navigation_wait: Option<NavigationWait>,
}

#[cfg(windows)]
impl RunSession {
    pub fn open(data_root: &Path, run_id: &str) -> Result<Self, String> {
        ensure_winrt_thread();
        if run_id.trim().is_empty() {
            return Err("run_id must not be empty".into());
        }
        let mut bound = bind_wechat(data_root)?;
        bound.run_id = run_id.to_string();
        Ok(Self {
            data_root: data_root.to_path_buf(),
            run_id: run_id.to_string(),
            bound,
            pending: PendingStore::new(),
            scene: SceneState::new(),
            last_observed: None,
            bind_count: 1,
            date_static_refreshes: 0,
            navigation_wait: None,
        })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn bind_count(&self) -> u32 {
        self.bind_count
    }

    pub fn wgc_open_count(&self) -> u32 {
        self.bound.capture_open_count
    }

    pub fn bound_title(&self) -> String {
        self.bound.native.title.clone()
    }

    pub(crate) fn last_observed(&self) -> Option<&Observed> {
        self.last_observed.as_ref()
    }

    /// Explicit rebuild of window lease + WGC (identity change / Guard failure).
    pub fn rebind(&mut self) -> Result<(), String> {
        ensure_winrt_thread();
        let mut bound = bind_wechat_with_generation(
            &self.data_root,
            self.bound.lease.generation.saturating_add(1),
        )?;
        bound.run_id = self.run_id.clone();
        bound.capture_open_count = self.bound.capture_open_count.saturating_add(1);
        log::info!(
            target: "glance",
            "[session_rebind] run_id={} old_lease={} new_lease={} old_hwnd={} new_hwnd={} old_bounds={:?} new_bounds={:?} old_generation={} new_generation={}",
            self.run_id,
            self.bound.lease.lease_id,
            bound.lease.lease_id,
            self.bound.native.hwnd_u64,
            bound.native.hwnd_u64,
            self.bound.lease.bounds_in_screen_points,
            bound.lease.bounds_in_screen_points,
            self.bound.lease.generation,
            bound.lease.generation,
        );
        self.bound = bound;
        self.pending.clear();
        self.scene.clear_date();
        self.last_observed = None;
        self.navigation_wait = None;
        self.bind_count = self.bind_count.saturating_add(1);
        self.date_static_refreshes = 0;
        Ok(())
    }

    pub fn observe(
        &mut self,
        goal: &TaskGoal,
        mode: GlanceMode,
    ) -> Result<SessionObservation, String> {
        ensure_winrt_thread();
        let date_weak = self.last_observed.as_ref().is_some_and(|last| {
            needs_static_date_refresh(
                last.result.snapshot.page,
                last.decision.refuse_reason.as_deref(),
            )
        });
        // No FrameArrived is normal for a static page, including a blank page
        // waiting on HTTP. Acquire a real independent frame, never relabel cached
        // pixels as fresh. The host owns the content readiness deadline.
        let first = observe_bound_timeout(
            &self.data_root, &mut self.bound, goal, Some(&mut self.scene),
            Duration::from_millis(DATE_STATIC_GRAB_MS),
        );
        let mut observed = match first {
            Ok(observed) => observed,
            Err(e) if e == "还没有新的一帧" => {
                self.pending.clear();
                self.bound.restart_capture().map_err(|restart_error| {
                    format!("CAPTURE_REFRESH_FAILED:{restart_error}")
                })?;
                self.date_static_refreshes = self.date_static_refreshes.saturating_add(1);
                log::info!(target: "glance",
                    "[static_capture] outcome=restarted run_id={} refresh={} wgc_open={} date_weak={} reason=static_no_frame",
                    self.run_id, self.date_static_refreshes, self.wgc_open_count(), date_weak);
                observe_bound_timeout(
                    &self.data_root, &mut self.bound, goal, Some(&mut self.scene),
                    Duration::from_millis(DATE_RESTART_FIRST_FRAME_MS),
                ).map_err(|capture_error| format!("CAPTURE_REFRESH_FAILED:{capture_error}"))?
            }
            Err(e) => return Err(e),
        };
        self.apply_navigation_wait(&mut observed);
        if !needs_static_date_refresh(
            observed.result.snapshot.page,
            observed.decision.refuse_reason.as_deref(),
        ) {
            self.date_static_refreshes = 0;
        }
        let token = self.install_pending_from_observed(&observed)?;
        let report = build_report(&self.bound.native.title, &observed, false, mode)?;
        let source_frame_id = observed.frame.frame_id.clone();
        let pending_evidence = if !mode.include_image {
            terminal_evidence_payload(&observed).map(|(w, h, bgra, boxes)| {
                PendingEvidenceFrame {
                    width: w,
                    height: h,
                    bgra,
                    boxes,
                }
            })
        } else {
            None
        };
        self.last_observed = Some(observed);
        Ok(SessionObservation {
            report,
            execution_token: token,
            bind_count: self.bind_count,
            wgc_open_count: self.wgc_open_count(),
            source_frame_id,
            pending_evidence,
        })
    }

    /// Execute a previously observed pending action. Token is one-shot.
    /// Uses the full click pipeline on the same bind (no rebind / no re-decide).
    pub fn execute_pending(
        &mut self,
        token: &str,
        goal: &TaskGoal,
        mode: GlanceMode,
    ) -> Result<SessionObservation, String> {
        ensure_winrt_thread();
        let now = monotonic_now_ms();
        let pending = self
            .pending
            .take_for_execute(token, &self.run_id, now)
            .map_err(|e| match e {
                PendingError::Expired => {
                    format!("{}{}", crate::PRE_INPUT_REOBSERVE_PREFIX, e.reason_code())
                }
                _ => e.reason_code().to_string(),
            })?;
        let observed = self
            .last_observed
            .take()
            .ok_or_else(|| "NO_OBSERVATION_FOR_PENDING".to_string())?;

        if pending.source_frame_id() != observed.frame.frame_id {
            self.last_observed = Some(observed);
            return Err("SOURCE_FRAME_MISMATCH".into());
        }

        // Kind is already validated by token ownership. Pass the pending ActionPlan
        // so inject does not rebuild from Observed; verification grabs a new frame.
        let _ = &pending.kind;
        let source_navigation = match &observed.decision.intent {
            NextIntent::Navigate { label, expected_page, .. } => Some(NavigationWait {
                from: observed.result.snapshot.page,
                expected: *expected_page,
                label: label.clone(),
                deadline_ms: now.saturating_add(45_000),
            }),
            _ => None,
        };
        let (mut report, mut after) = match click_from_observed(
            &self.data_root,
            &mut self.bound,
            observed,
            goal,
            mode,
            Some(pending.plan),
        ) {
            Ok(v) => v,
            Err(e) => {
                // Token already consumed; caller must observe again.
                return Err(e);
            }
        };
        if let Some(wait) = source_navigation {
            let next = match &after.decision.intent {
                NextIntent::Navigate { label, .. } => Some(label.as_str()),
                _ => None,
            };
            if navigation_still_pending(wait.from, wait.expected, &wait.label, after.result.snapshot.page, next) {
                self.navigation_wait = Some(wait);
                self.apply_navigation_wait(&mut after);
                report = build_report(&self.bound.native.title, &after, true, mode)?;
            }
        }
        self.date_static_refreshes = 0;

        let source_frame_id = after.frame.frame_id.clone();
        let pending_evidence = if !mode.include_image {
            terminal_evidence_payload(&after).map(|(w, h, bgra, boxes)| PendingEvidenceFrame {
                width: w,
                height: h,
                bgra,
                boxes,
            })
        } else {
            None
        };
        let token = self.install_pending_from_observed(&after)?;
        self.last_observed = Some(after);
        Ok(SessionObservation {
            report,
            execution_token: token,
            bind_count: self.bind_count,
            wgc_open_count: self.wgc_open_count(),
            source_frame_id,
            pending_evidence,
        })
    }

    /// An accepted navigation that still shows the original control is awaiting
    /// its result. Observing it again must not immediately issue the same click.
    fn apply_navigation_wait(&mut self, observed: &mut Observed) {
        let Some(wait) = self.navigation_wait.as_ref() else { return; };
        let next = match &observed.decision.intent {
            NextIntent::Navigate { label, .. } => Some(label.as_str()),
            _ => None,
        };
        if !navigation_still_pending(wait.from, wait.expected, &wait.label, observed.result.snapshot.page, next) {
            self.navigation_wait = None;
            return;
        }
        if monotonic_now_ms() >= wait.deadline_ms {
            observed.decision.intent = NextIntent::Stop { reason: "NAVIGATION_TIMEOUT".into() };
            observed.decision.refuse_reason = Some("NAVIGATION_TIMEOUT".into());
            observed.decision.next_line = "下一步：暂停，导航未完成".into();
        } else {
            observed.decision.intent = NextIntent::Wait { reason: "NAVIGATION_PENDING".into(), approx_ms: 500 };
            observed.decision.refuse_reason = Some("NAVIGATION_PENDING".into());
            observed.decision.next_line = "下一步：等待已发送的导航完成".into();
        }
    }

    fn install_pending_from_observed(
        &mut self,
        observed: &Observed,
    ) -> Result<Option<String>, String> {
        self.pending.clear();
        let decision = &observed.decision;
        let created = monotonic_now_ms();
        let size = SizeI {
            width: observed.frame.pixel_width,
            height: observed.frame.pixel_height,
        };
        match &decision.intent {
            NextIntent::Navigate {
                label,
                anchor,
                expected_page,
            } => {
                let action_type = if label.contains("确认预约") || label.contains("提交") {
                    ActionType::SubmitOnce
                } else {
                    ActionType::Click
                };
                let fragile = if is_fragile_nav_label(label) {
                    Some(FRAGILE_NAV_STREAK_NEED)
                } else {
                    None
                };
                let kind = pending_kind_from_click(label, *expected_page, action_type, fragile);
                let plan = build_click_plan(
                    &self.run_id,
                    &self.bound.lease.lease_id,
                    self.bound.lease.generation,
                    &observed.frame.frame_id,
                    size,
                    &observed.result.snapshot,
                    anchor.clone(),
                    action_type,
                    &decision.expected_transition,
                    decision.confidence,
                    created,
                    DEFAULT_PLAN_TTL_MS,
                );
                let tok = self
                    .pending
                    .install(&self.run_id, kind, plan)
                    .map_err(|e| e.reason_code().to_string())?;
                Ok(Some(tok))
            }
            NextIntent::AtomicComposite {
                label,
                steps,
                expected_page,
            } => {
                let (anchor, transition) = match steps.first() {
                    Some(AtomicStep::Click { anchor, .. }) => {
                        (anchor.clone(), decision.expected_transition.as_str())
                    }
                    _ => return Ok(None),
                };
                let plan = build_click_plan(
                    &self.run_id,
                    &self.bound.lease.lease_id,
                    self.bound.lease.generation,
                    &observed.frame.frame_id,
                    size,
                    &observed.result.snapshot,
                    anchor,
                    ActionType::Click,
                    transition,
                    decision.confidence,
                    created,
                    DEFAULT_PLAN_TTL_MS,
                );
                let kind = PendingKind::AtomicComposite {
                    label: label.clone(),
                    steps: steps.clone(),
                    expected_page: *expected_page,
                };
                let tok = self
                    .pending
                    .install(&self.run_id, kind, plan)
                    .map_err(|e| e.reason_code().to_string())?;
                Ok(Some(tok))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(not(windows))]
pub struct RunSession;

#[cfg(not(windows))]
impl RunSession {
    pub fn open(_data_root: &Path, _run_id: &str) -> Result<Self, String> {
        Err("RunSession is Windows-only".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{needs_static_date_refresh, navigation_still_pending, Page};

    #[test]
    fn accepted_navigation_waits_without_repeating_original_control() {
        assert!(navigation_still_pending(Page::Home, Page::VenueSportList, "场馆预约", Page::Home, Some("场馆预约")));
        assert!(!navigation_still_pending(Page::Home, Page::VenueSportList, "场馆预约", Page::BookingNotice, Some("我知道了")));
        assert!(!navigation_still_pending(Page::ConfirmOrder, Page::ConfirmOrder, "captcha_image", Page::ConfirmOrder, Some("captcha_image")));
        assert!(!navigation_still_pending(Page::ConfirmOrder, Page::MyReservations, "确定预约", Page::ConfirmOrder, None));
    }

    #[test]
    fn weak_date_diagnostic_is_scoped_to_venue_detail() {
        assert!(needs_static_date_refresh(Page::VenueDetail, Some("DATE_EVIDENCE_WEAK")));
        assert!(!needs_static_date_refresh(Page::VenueDetail, Some("DATE_UNAVAILABLE")));
        assert!(!needs_static_date_refresh(Page::VenueList, Some("DATE_EVIDENCE_WEAK")));
    }
}

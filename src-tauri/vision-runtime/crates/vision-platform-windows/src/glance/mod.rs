//! One-shot observe for the desktop UI. Optional single-step click is separate.
//!
//! Split by invariant (ADR / Plan P5):
//! - [`report`] — GlanceReport framing / evidence BMP
//! - [`bind`] — window lease + WGC grab
//! - [`observe`] — OCR + shadow + decide_next
//! - [`inject`] — Guard + SendInput + post-click poll orchestration

use std::path::Path;

use vision_core::TaskGoal;

#[cfg(windows)]
mod bind;
#[cfg(windows)]
mod inject;
#[cfg(windows)]
mod observe;
mod report;

pub use report::{DateCardSummary, DateEvidenceSummary, GlanceMode, GlanceReport};

#[cfg(windows)]
pub(crate) use bind::{bind_wechat, bind_wechat_with_generation, ensure_winrt_thread, BoundWechat, Observed};
#[cfg(windows)]
pub(crate) use inject::click_from_observed;
#[cfg(windows)]
pub(crate) use observe::observe_bound_timeout;
#[cfg(windows)]
pub(crate) use report::{build_report, terminal_evidence_payload};

pub fn glance_wechat(data_root: &Path, goal: TaskGoal) -> Result<GlanceReport, String> {
    glance_wechat_with_mode(data_root, goal, GlanceMode::ui())
}

/// Native run loop: same observe path, no evidence BMP encode.
pub fn glance_wechat_run(data_root: &Path, goal: TaskGoal) -> Result<GlanceReport, String> {
    glance_wechat_with_mode(data_root, goal, GlanceMode::run())
}

fn glance_wechat_with_mode(
    data_root: &Path,
    goal: TaskGoal,
    mode: GlanceMode,
) -> Result<GlanceReport, String> {
    #[cfg(windows)]
    {
        inject::glance_wechat_win(data_root, goal, false, mode)
    }
    #[cfg(not(windows))]
    {
        let _ = (data_root, goal, mode);
        Err("核对当前只在 Windows 上读微信窗".into())
    }
}

/// Observe → if Navigate, Guard + one click → settle → observe again.
pub fn click_wechat_step(data_root: &Path, goal: TaskGoal) -> Result<GlanceReport, String> {
    click_wechat_step_with_mode(data_root, goal, GlanceMode::ui())
}

pub fn click_wechat_step_run(data_root: &Path, goal: TaskGoal) -> Result<GlanceReport, String> {
    click_wechat_step_with_mode(data_root, goal, GlanceMode::run())
}

fn click_wechat_step_with_mode(
    data_root: &Path,
    goal: TaskGoal,
    mode: GlanceMode,
) -> Result<GlanceReport, String> {
    #[cfg(windows)]
    {
        inject::click_wechat_step_win(data_root, goal, mode)
    }
    #[cfg(not(windows))]
    {
        let _ = (data_root, goal, mode);
        Err("核对点击当前只在 Windows 上可用".into())
    }
}

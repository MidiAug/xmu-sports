//! Per-page intent policies. Only decide_next dispatches into these.

mod auth;
mod confirm_order;
mod nav;
mod select_slot;
mod venue_detail;
mod wechat_shell;

pub(crate) use auth::xmu_wx_authorize;
pub(crate) use confirm_order::confirm_order;
pub use confirm_order::{plan_captcha_keyboard, CaptchaKeyStroke, CaptchaKeyboardPlan};
pub(crate) use nav::{nav_goal_label, nav_sport_icon, nav_text, notice_ack};
pub(crate) use select_slot::select_slot;
pub(crate) use venue_detail::venue_detail;
pub(crate) use wechat_shell::{mp_panel, wechat_discover, wechat_shell_or_unknown};

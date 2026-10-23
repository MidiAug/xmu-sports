//! Fragile navigate confirm: multi-frame page+label streak before clicking.
//!
//! After miniprogram open, a login gate（立即登录）can flash for one frame while
//! the session is validated, then the real home（场馆预约）appears. Clicking the
//! flash sends the flow into auth. Same pattern as UiAutomator waitForStable +
//! state condition: require the actionable label on the same page for N frames.

use crate::state::Page;

/// Consecutive identical (page, label) samples before arming the click.
pub const FRAGILE_NAV_STREAK_NEED: u32 = 3;

/// Labels that must not be clicked on a single-frame recognition.
pub fn is_fragile_nav_label(label: &str) -> bool {
    matches!(label.trim(), "立即登录" | "场馆预约")
}

/// Confirm was armed for `wanted`, but the latest frame wants a *different*
/// fragile label (e.g. login flash → home「场馆预约」). Abort this confirm
/// immediately — do not burn the streak budget waiting for `wanted` to return.
pub fn fragile_nav_target_changed(wanted: &str, current_label: &str) -> bool {
    let wanted = wanted.trim();
    let current = current_label.trim();
    is_fragile_nav_label(current) && !current.is_empty() && current != wanted
}

/// Tracks consecutive (page, navigate-label) hits.
#[derive(Debug, Clone)]
pub struct NavLabelStreak {
    last_page: Option<Page>,
    last_label: Option<String>,
    streak: u32,
    need: u32,
}

impl Default for NavLabelStreak {
    fn default() -> Self {
        Self::new(FRAGILE_NAV_STREAK_NEED)
    }
}

impl NavLabelStreak {
    pub fn new(need: u32) -> Self {
        Self {
            last_page: None,
            last_label: None,
            streak: 0,
            need: need.max(1),
        }
    }

    pub fn streak(&self) -> u32 {
        self.streak
    }

    pub fn last_label(&self) -> Option<&str> {
        self.last_label.as_deref()
    }

    pub fn last_page(&self) -> Option<Page> {
        self.last_page
    }

    /// Feed one observe. Returns true when `need` consecutive matches.
    pub fn push(&mut self, page: Page, label: &str) -> bool {
        let label = label.trim();
        let same = self.last_page == Some(page)
            && self
                .last_label
                .as_deref()
                .is_some_and(|l| l == label);
        if same {
            self.streak = self.streak.saturating_add(1);
        } else {
            self.last_page = Some(page);
            self.last_label = Some(label.to_string());
            self.streak = 1;
        }
        self.streak >= self.need
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragile_labels() {
        assert!(is_fragile_nav_label("立即登录"));
        assert!(is_fragile_nav_label("场馆预约"));
        assert!(!is_fragile_nav_label("乒乓球"));
    }

    #[test]
    fn login_flash_to_home_is_target_change() {
        assert!(fragile_nav_target_changed("立即登录", "场馆预约"));
        assert!(!fragile_nav_target_changed("场馆预约", "场馆预约"));
        assert!(!fragile_nav_target_changed("立即登录", "立即登录"));
        assert!(!fragile_nav_target_changed("立即登录", "乒乓球"));
    }

    #[test]
    fn login_flash_then_home_does_not_arm_login() {
        let mut t = NavLabelStreak::new(3);
        assert!(!t.push(Page::XmuLoginGate, "立即登录"));
        // Flash ends — home wins; login never reaches streak 3.
        assert!(!t.push(Page::Home, "场馆预约"));
        assert!(!t.push(Page::Home, "场馆预约"));
        assert!(t.push(Page::Home, "场馆预约"));
        assert_eq!(t.last_label(), Some("场馆预约"));
    }

    #[test]
    fn sticky_login_arms_after_three() {
        let mut t = NavLabelStreak::new(3);
        assert!(!t.push(Page::XmuLoginGate, "立即登录"));
        assert!(!t.push(Page::XmuLoginGate, "立即登录"));
        assert!(t.push(Page::XmuLoginGate, "立即登录"));
    }

    #[test]
    fn label_change_on_same_page_resets() {
        let mut t = NavLabelStreak::new(2);
        assert!(!t.push(Page::Home, "场馆预约"));
        assert!(!t.push(Page::Home, "立即登录")); // reset
        assert!(t.push(Page::Home, "立即登录"));
    }
}

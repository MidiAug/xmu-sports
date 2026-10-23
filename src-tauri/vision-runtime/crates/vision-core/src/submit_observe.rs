//! Post-「确定预约」SubmitObserve — latch ephemeral toasts + structural fallback.
//! Spec: docs/superpowers/specs/2026-09-23-captcha-submit-observe-design.md

use crate::state::Page;

/// Evidence locked during post-submit poll (first hit sticks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubmitLatch {
    #[default]
    None,
    Wrong,
    Empty,
    Success,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    Success,
    Wrong,
    Empty,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub struct SubmitPre {
    pub filled: usize,
    pub tile_fp: u64,
}

/// Scan OCR / candidate labels (+ page) for a latchable submit signal.
pub fn latch_from_texts<'a>(
    texts: impl IntoIterator<Item = &'a str>,
    page: Page,
) -> Option<SubmitLatch> {
    if matches!(
        page,
        Page::MyReservations | Page::Success | Page::OrderDetail
    ) {
        return Some(SubmitLatch::Success);
    }
    let mut saw_wrong = false;
    let mut saw_empty = false;
    for t in texts {
        let t = t.trim();
        if t.contains("验证码错误") || t.contains("验证错误") {
            saw_wrong = true;
        }
        if t.contains("请输入验证码") || t == "请输入验证" {
            saw_empty = true;
        }
    }
    if saw_wrong {
        Some(SubmitLatch::Wrong)
    } else if saw_empty {
        Some(SubmitLatch::Empty)
    } else {
        None
    }
}

/// First non-None latch wins for the whole submit poll.
pub fn merge_latch(prev: SubmitLatch, next: Option<SubmitLatch>) -> SubmitLatch {
    if prev != SubmitLatch::None {
        return prev;
    }
    next.unwrap_or(SubmitLatch::None)
}

/// Evidence ladder after poll ends.
pub fn classify_outcome(
    latch: SubmitLatch,
    page: Page,
    pre: SubmitPre,
    now_filled: usize,
    tile_changed: bool,
) -> SubmitOutcome {
    if matches!(latch, SubmitLatch::Success) || matches!(page, Page::MyReservations | Page::Success)
    {
        return SubmitOutcome::Success;
    }
    if matches!(latch, SubmitLatch::Wrong) {
        return SubmitOutcome::Wrong;
    }
    if matches!(latch, SubmitLatch::Empty) {
        return SubmitOutcome::Empty;
    }
    // Structural fallback: still on confirm, had 4 chars, now empty, tile refreshed.
    let still_confirm = matches!(page, Page::ConfirmOrder);
    if still_confirm && pre.filled >= 4 && now_filled == 0 && tile_changed {
        return SubmitOutcome::Wrong;
    }
    // Loading/blank/wrong-page transitions do not prove an order was created.
    SubmitOutcome::Unknown
}

pub fn refuse_for_outcome(outcome: SubmitOutcome) -> Option<&'static str> {
    match outcome {
        // Host `submit_success` treats ALREADY_DONE as submitted — needed when
        // the post-submit page is misclassified (e.g. my_reservations → venue_list).
        SubmitOutcome::Success => Some("ALREADY_DONE"),
        SubmitOutcome::Wrong => Some("CAPTCHA_ERROR_SETTLE"),
        SubmitOutcome::Empty => Some("CAPTCHA_NEED_INPUT"),
        SubmitOutcome::Unknown => Some("CAPTCHA_SUBMIT_UNKNOWN"),
    }
}

pub fn should_clear_code(outcome: SubmitOutcome) -> bool {
    matches!(outcome, SubmitOutcome::Wrong)
}

/// Legacy / docs upper bound for WRONG settle (AC: ≤5s; default tick below).
pub const CAPTCHA_WRONG_WAIT_MS: u64 = 2_500;
/// One re-glance tick while toast may still be fading (desired-state, not fixed 5s).
pub const CAPTCHA_WRONG_TICK_MS: u64 = 800;
/// Tile already refreshed or input already empty — brief settle only.
pub const CAPTCHA_WRONG_SETTLED_MS: u64 = 400;

/// Desired-state wait after WRONG (Spec §6): short tick; shorter when already settled.
pub fn wrong_wait_ms(tile_changed: bool, input_empty: bool) -> u64 {
    if tile_changed || input_empty {
        CAPTCHA_WRONG_SETTLED_MS
    } else {
        CAPTCHA_WRONG_TICK_MS
    }
}

/// Mid-poll structural WRONG (toast miss): still confirm ∧ pre≥4 ∧ now empty ∧ tile changed.
pub fn structural_wrong_latch(
    page: Page,
    pre: SubmitPre,
    now_filled: usize,
    tile_changed: bool,
) -> Option<SubmitLatch> {
    if matches!(page, Page::ConfirmOrder) && pre.filled >= 4 && now_filled == 0 && tile_changed {
        Some(SubmitLatch::Wrong)
    } else {
        None
    }
}

pub fn outcome_as_str(outcome: SubmitOutcome) -> &'static str {
    match outcome {
        SubmitOutcome::Success => "SUCCESS",
        SubmitOutcome::Wrong => "WRONG",
        SubmitOutcome::Empty => "EMPTY",
        SubmitOutcome::Unknown => "UNKNOWN",
    }
}

pub fn latch_as_str(latch: SubmitLatch) -> &'static str {
    match latch {
        SubmitLatch::None => "none",
        SubmitLatch::Wrong => "WRONG",
        SubmitLatch::Empty => "EMPTY",
        SubmitLatch::Success => "SUCCESS",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latch_wrong_sticks_across_frames() {
        let mut latch = SubmitLatch::None;
        latch = merge_latch(
            latch,
            latch_from_texts(["验证码", "验证码错误"], Page::ConfirmOrder),
        );
        assert_eq!(latch, SubmitLatch::Wrong);
        // Toast gone — empty texts must not clear.
        latch = merge_latch(
            latch,
            latch_from_texts(["验证码", "确定预约"], Page::ConfirmOrder),
        );
        assert_eq!(latch, SubmitLatch::Wrong);
        let outcome = classify_outcome(
            latch,
            Page::ConfirmOrder,
            SubmitPre {
                filled: 4,
                tile_fp: 1,
            },
            0,
            true,
        );
        assert_eq!(outcome, SubmitOutcome::Wrong);
        assert!(should_clear_code(outcome));
        assert_eq!(refuse_for_outcome(outcome), Some("CAPTCHA_ERROR_SETTLE"));
    }

    #[test]
    fn structural_triplet_without_toast() {
        let outcome = classify_outcome(
            SubmitLatch::None,
            Page::ConfirmOrder,
            SubmitPre {
                filled: 4,
                tile_fp: 0xaaa,
            },
            0,
            true,
        );
        assert_eq!(outcome, SubmitOutcome::Wrong);
        assert_eq!(
            structural_wrong_latch(
                Page::ConfirmOrder,
                SubmitPre {
                    filled: 4,
                    tile_fp: 0xaaa,
                },
                0,
                true,
            ),
            Some(SubmitLatch::Wrong)
        );
    }

    #[test]
    fn wrong_wait_shortens_when_settled() {
        assert_eq!(wrong_wait_ms(true, false), CAPTCHA_WRONG_SETTLED_MS);
        assert_eq!(wrong_wait_ms(false, true), CAPTCHA_WRONG_SETTLED_MS);
        assert_eq!(wrong_wait_ms(false, false), CAPTCHA_WRONG_TICK_MS);
        assert!(wrong_wait_ms(false, false) <= CAPTCHA_WRONG_WAIT_MS);
        assert!(CAPTCHA_WRONG_WAIT_MS <= 5_000);
    }

    #[test]
    fn success_refuse_stops_host() {
        assert_eq!(
            refuse_for_outcome(SubmitOutcome::Success),
            Some("ALREADY_DONE")
        );
    }

    #[test]
    fn empty_toast_does_not_upgrade_to_wrong() {
        let latch = merge_latch(
            SubmitLatch::None,
            latch_from_texts(["请输入验证码"], Page::ConfirmOrder),
        );
        assert_eq!(latch, SubmitLatch::Empty);
        let outcome = classify_outcome(
            latch,
            Page::ConfirmOrder,
            SubmitPre {
                filled: 0,
                tile_fp: 1,
            },
            0,
            true, // tile may change for other reasons — EMPTY still wins
        );
        assert_eq!(outcome, SubmitOutcome::Empty);
        assert!(!should_clear_code(outcome));
        assert_eq!(refuse_for_outcome(outcome), Some("CAPTCHA_NEED_INPUT"));
    }

    #[test]
    fn no_signal_stays_unknown_on_confirm() {
        let outcome = classify_outcome(
            SubmitLatch::None,
            Page::ConfirmOrder,
            SubmitPre {
                filled: 4,
                tile_fp: 1,
            },
            4,
            false,
        );
        assert_eq!(outcome, SubmitOutcome::Unknown);
        assert!(!should_clear_code(outcome));
    }

    #[test]
    fn left_confirm_is_success() {
        let outcome = classify_outcome(
            SubmitLatch::None,
            Page::MyReservations,
            SubmitPre {
                filled: 4,
                tile_fp: 1,
            },
            0,
            true,
        );
        assert_eq!(outcome, SubmitOutcome::Success);
        assert_eq!(refuse_for_outcome(outcome), Some("ALREADY_DONE"));
    }

    #[test]
    fn leaving_confirm_without_result_evidence_stays_unknown() {
        let outcome = classify_outcome(
            SubmitLatch::None,
            Page::VenueList,
            SubmitPre {
                filled: 4,
                tile_fp: 1,
            },
            0,
            false,
        );
        assert_eq!(outcome, SubmitOutcome::Unknown);
        assert_eq!(
            classify_outcome(
                SubmitLatch::None,
                Page::Unknown,
                SubmitPre {
                    filled: 4,
                    tile_fp: 1
                },
                0,
                false
            ),
            SubmitOutcome::Unknown
        );
    }
}

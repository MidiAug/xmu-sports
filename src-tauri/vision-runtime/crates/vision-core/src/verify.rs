use crate::state::{Page, UiSnapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmitObserveOutcome {
    Success,
    CaptchaEmpty,
    CaptchaWrong,
    UnknownAfterSubmit,
    LeftConfirm,
}

/// Post-submit observation — never auto-retry non-idempotent submit.
pub fn after_submit_outcome(snaps: &[UiSnapshot]) -> SubmitObserveOutcome {
    for s in snaps {
        if s.page == Page::Success || s.page == Page::MyReservations || s.page == Page::OrderDetail
        {
            return SubmitObserveOutcome::Success;
        }
        if s.toast_need_input {
            return SubmitObserveOutcome::CaptchaEmpty;
        }
        if s.toast_error {
            return SubmitObserveOutcome::CaptchaWrong;
        }
        if s.page != Page::ConfirmOrder && s.page != Page::Unknown {
            return SubmitObserveOutcome::LeftConfirm;
        }
    }
    SubmitObserveOutcome::UnknownAfterSubmit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_when_ambiguous() {
        let snaps = vec![UiSnapshot {
            page: Page::ConfirmOrder,
            ..UiSnapshot::default()
        }];
        assert_eq!(
            after_submit_outcome(&snaps),
            SubmitObserveOutcome::UnknownAfterSubmit
        );
    }

    #[test]
    fn success_on_my_list() {
        let snaps = vec![UiSnapshot {
            page: Page::MyReservations,
            ..UiSnapshot::default()
        }];
        assert_eq!(after_submit_outcome(&snaps), SubmitObserveOutcome::Success);
    }
}

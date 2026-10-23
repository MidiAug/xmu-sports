use crate::state::Page;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowStep {
    Activate,
    Discover,
    Miniprogram,
    Search,
    OpenApp,
    VenueBooking,
    DismissNotice,
    SelectSport,
    SelectVenue,
    SelectDate,
    SelectSlot,
    ConfirmSlot,
    FillCaptcha,
    Done,
}

pub const STEP_ORDER: &[FlowStep] = &[
    FlowStep::Activate,
    FlowStep::Discover,
    FlowStep::Miniprogram,
    FlowStep::Search,
    FlowStep::OpenApp,
    FlowStep::VenueBooking,
    FlowStep::DismissNotice,
    FlowStep::SelectSport,
    FlowStep::SelectVenue,
    FlowStep::SelectDate,
    FlowStep::SelectSlot,
    FlowStep::ConfirmSlot,
    FlowStep::FillCaptcha,
    FlowStep::Done,
];

/// Map page → resume step (mirrors Python PAGE_RESUME_STEP).
pub fn page_resume_step(page: Page, slot_ready: bool) -> FlowStep {
    if page == Page::SelectSlot && slot_ready {
        return FlowStep::ConfirmSlot;
    }
    match page {
        Page::Success | Page::MyReservations => FlowStep::Done,
        Page::ConfirmOrder => FlowStep::FillCaptcha,
        Page::SelectSlot => FlowStep::SelectSlot,
        Page::VenueDetail => FlowStep::SelectDate,
        Page::VenueList => FlowStep::SelectVenue,
        Page::VenueSportList => FlowStep::SelectSport,
        Page::Home => FlowStep::VenueBooking,
        Page::BookingNotice => FlowStep::DismissNotice,
        Page::XmuLoginGate | Page::XmuWxAuthorize | Page::XmuAuthWaiting => FlowStep::OpenApp,
        Page::WechatDiscover => FlowStep::Miniprogram,
        Page::WechatMpPanel => FlowStep::OpenApp,
        Page::WechatMpSearch => FlowStep::OpenApp,
        Page::WechatMain => FlowStep::Discover,
        _ => FlowStep::Activate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_confirm_when_slot_ready() {
        assert_eq!(
            page_resume_step(Page::SelectSlot, true),
            FlowStep::ConfirmSlot
        );
        assert_eq!(
            page_resume_step(Page::SelectSlot, false),
            FlowStep::SelectSlot
        );
        assert_eq!(
            page_resume_step(Page::WechatMpPanel, false),
            FlowStep::OpenApp
        );
        assert_ne!(
            page_resume_step(Page::WechatMpPanel, false),
            FlowStep::Search
        );
    }

    #[test]
    fn booking_pages_do_not_resume_at_shell_navigation() {
        for page in [
            Page::Home,
            Page::VenueSportList,
            Page::VenueList,
            Page::VenueDetail,
            Page::SelectSlot,
            Page::ConfirmOrder,
            Page::MyReservations,
            Page::Success,
        ] {
            let step = page_resume_step(page, false);
            assert_ne!(step, FlowStep::Discover, "{page:?}");
            assert_ne!(step, FlowStep::Miniprogram, "{page:?}");
            assert_ne!(step, FlowStep::Activate, "{page:?}");
        }
    }
}

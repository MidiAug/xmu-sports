//! Interactive-element candidates. Decision code may only click boxes in this set.
//!
//! Sources: detect ∪ template ∪ color cells ∪ OCR labels. OCR does not invent clicks.

mod color;
mod keyboard;
mod ncc;
mod needles;
mod parse;
mod template_match;
mod types;

pub use color::{
    analyze_captcha_input, count_input_glyphs, find_captcha_patch, merge_typed_proxy, opencv_hsv,
    pixel_is_mint, CaptchaInputState,
};
pub use needles::{bundled_shell_templates, load_shell_templates};
pub use parse::parse_candidates;
pub use template_match::template_band_best_score;
pub use types::{
    box_iou, detections_from_xyxy, BBox, Candidate, CandidateKind, CandidateSet, CandidateSource,
    ColorRules, DetectBox, ElementObservation, ParseInput, PerceptionFeed, ShellVisualState,
    SlotMeta, TemplateNeedle, DEDUP_IOU_DEFAULT, ELEMENT_AUTH_LOGIN, ELEMENT_DISCOVER,
    ELEMENT_MINIPROGRAM, ELEMENT_MP_SEARCH, ELEMENT_NOTICE_ACK, ELEMENT_SPORT_ICON,
    ELEMENT_VENUE_BOOKING, ELEMENT_XMU_SPORTS, LABEL_IOU_DEFAULT, NAV_IOU_MIN, SHELL_STATE_MARGIN,
};

#[cfg(test)]
mod tests;

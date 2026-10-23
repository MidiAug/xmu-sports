//! Confirm-order / captcha keyboard policy.

use serde::{Deserialize, Serialize};
use vision_contracts::AnchorBox;

use crate::candidates::{Candidate, CandidateKind, CandidateSet};
use crate::state::{CaptchaPhase, Overlay, Page, UiSnapshot};

use super::super::{box_of, finish, navigate, stop, FlowDomain, NextIntent, ObservationDecision, TaskGoal};
use super::nav::nav_text;

/// One soft-key (or「完成」) stroke on the confirm-order keyboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptchaKeyStroke {
    pub label: String,
    pub anchor: AnchorBox,
}

/// Soft-key plan for captcha typing.
///
/// - [`Self::all_keys`]: full 4-char preview (green boxes always show these +「完成」)
/// - [`Self::remaining_keys`]: still-to-type suffix (actual click / poll progress)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptchaKeyboardPlan {
    pub all_keys: Vec<CaptchaKeyStroke>,
    pub remaining_keys: Vec<CaptchaKeyStroke>,
    pub done: Option<CaptchaKeyStroke>,
}

/// Plan captcha typing from keyboard-region glyphs only.
/// Body OCR (dates, venue names) is never a soft key — see `promote_keyboard`.
///
/// Green preview always marks the **full** code. Click target is
/// `remaining_keys.first()` (or「完成」when filled ≥ 4).
pub fn plan_captcha_keyboard(
    candidates: &CandidateSet,
    code: &str,
    filled: usize,
) -> Option<CaptchaKeyboardPlan> {
    let target: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(4)
        .collect::<String>()
        .to_ascii_uppercase();
    if target.chars().count() < 4 {
        return None;
    }
    let filled = filled.min(4);
    let done = find_keyboard_done(candidates).map(|hit| CaptchaKeyStroke {
        label: hit.label.clone(),
        anchor: box_of(hit),
    });

    // Collect every localized key for the full code (preview).
    let mut all_keys = Vec::new();
    for ch in target.chars() {
        match find_soft_key(candidates, ch) {
            Some(hit) => all_keys.push(CaptchaKeyStroke {
                label: hit.label.clone(),
                anchor: box_of(hit),
            }),
            None => break,
        }
    }

    if filled >= 4 {
        return done.map(|done| CaptchaKeyboardPlan {
            all_keys,
            remaining_keys: Vec::new(),
            done: Some(done),
        });
    }

    // Next key to type must be present.
    let need = target.chars().nth(filled)?;
    if find_soft_key(candidates, need).is_none() {
        return None;
    }

    let mut remaining_keys: Vec<CaptchaKeyStroke> =
        all_keys.iter().skip(filled).cloned().collect();
    if remaining_keys.is_empty() {
        // all_keys stopped early before `filled`; rebuild remaining from filled..
        for ch in target.chars().skip(filled) {
            match find_soft_key(candidates, ch) {
                Some(hit) => remaining_keys.push(CaptchaKeyStroke {
                    label: hit.label.clone(),
                    anchor: box_of(hit),
                }),
                None if remaining_keys.is_empty() => return None,
                None => break,
            }
        }
    }

    Some(CaptchaKeyboardPlan {
        all_keys,
        remaining_keys,
        done,
    })
}

pub(crate) fn confirm_order(
    snap: &UiSnapshot,
    goal: &TaskGoal,
    candidates: &CandidateSet,
) -> ObservationDecision {
    let keyboard = snap.keyboard_open
        || snap.overlay == Overlay::Keyboard
        || (candidates.has_label("完成")
            && candidates.has_label("取消")
            && candidates.has_label("验证码"));
    // Progress: chars already in the input band (OCR/ink), not per-underline slots.
    let filled = snap
        .input_chars
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == 'X')
        .count()
        .min(4);

    if let Some(code) = goal.recognized_captcha() {
        let target: String = code
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(4)
            .collect::<String>()
            .to_ascii_uppercase();
        if target.chars().count() >= 4 {
            if keyboard {
                match plan_captcha_keyboard(candidates, &target, filled) {
                    Some(plan) => {
                        // Green preview: always full code keys +「完成」, not only remaining.
                        let mut boxes: Vec<AnchorBox> = plan
                            .all_keys
                            .iter()
                            .map(|k| k.anchor.clone())
                            .collect();
                        if let Some(done) = &plan.done {
                            boxes.push(done.anchor.clone());
                        }
                        let (label, anchor) = if let Some(first) = plan.remaining_keys.first() {
                            (first.label.clone(), first.anchor.clone())
                        } else if let Some(done) = plan.done {
                            (done.label, done.anchor)
                        } else {
                            return stop(
                                snap,
                                goal,
                                FlowDomain::XmuBooking,
                                "CAPTCHA_KEY_NOT_FOUND",
                                "暂停，软键盘上没有下一键",
                            );
                        };
                        return finish(
                            FlowDomain::XmuBooking,
                            Page::ConfirmOrder,
                            snap,
                            goal,
                            "现在：确认预约，键盘已开",
                            &format!("下一步：输入验证码{{{target}}}并点击完成"),
                            NextIntent::Navigate {
                                label,
                                anchor: anchor.clone(),
                                expected_page: Page::ConfirmOrder,
                            },
                            boxes,
                            "confirm_order",
                            None,
                        );
                    }
                    None => {
                        let need = target.chars().nth(filled).unwrap_or('?');
                        return finish(
                            FlowDomain::XmuBooking,
                            Page::ConfirmOrder,
                            snap,
                            goal,
                            "现在：确认预约，键盘已开",
                            &format!(
                                "下一步：软键盘上没有键「{need}」（码已带，等键盘字母 OCR）"
                            ),
                            NextIntent::Stop {
                                reason: "CAPTCHA_KEY_NOT_FOUND".into(),
                            },
                            Vec::new(),
                            "confirm_order",
                            Some("CAPTCHA_KEY_NOT_FOUND".into()),
                        );
                    }
                }
            }
            if !keyboard && filled >= 4 {
                return nav_text(
                    snap,
                    goal,
                    candidates,
                    FlowDomain::XmuBooking,
                    "现在：确认预约",
                    "确定预约",
                    "下一步：确定预约",
                    Page::MyReservations,
                    "my_reservations",
                );
            }
        }
    }

    // Soft keyboard open: OCR + type +「完成」happen in the host click path.
    // Policy only waits when the code is still missing (thin tile / OCR flake).
    if keyboard {
        if filled >= 4 {
            if let Some(hit) = find_keyboard_done(candidates) {
                let code = goal
                    .recognized_captcha()
                    .map(|c| {
                        c.chars()
                            .filter(|ch| ch.is_ascii_alphanumeric())
                            .take(4)
                            .collect::<String>()
                            .to_ascii_uppercase()
                    })
                    .filter(|c| c.chars().count() >= 4)
                    .unwrap_or_else(|| "????".into());
                return finish(
                    FlowDomain::XmuBooking,
                    Page::ConfirmOrder,
                    snap,
                    goal,
                    "现在：确认预约，键盘已开",
                    &format!("下一步：输入验证码{{{code}}}并点击完成"),
                    NextIntent::Navigate {
                        label: hit.label.clone(),
                        anchor: box_of(hit),
                        expected_page: Page::ConfirmOrder,
                    },
                    vec![box_of(hit)],
                    "confirm_order",
                    None,
                );
            }
        }
        return finish(
            FlowDomain::XmuBooking,
            Page::ConfirmOrder,
            snap,
            goal,
            "现在：确认预约，键盘已开",
            "下一步：识别验证码、输入四位并点击完成",
            NextIntent::Wait {
                reason: "CAPTCHA_OCR_PENDING".into(),
                approx_ms: 700,
            },
            Vec::new(),
            "confirm_order",
            Some("CAPTCHA_OCR_PENDING".into()),
        );
    }

    if snap.captcha == CaptchaPhase::IdleFull || filled >= 4 {
        return nav_text(
            snap,
            goal,
            candidates,
            FlowDomain::XmuBooking,
            "现在：确认预约",
            "确定预约",
            "下一步：确定预约",
            Page::MyReservations,
            "my_reservations",
        );
    }

    // Keyboard closed: decide *before* opening the soft keyboard.
    // Valid 4-char code → open input. Missing/short OCR → click the tile to
    // refresh (never thrash keyboard open→settle→refresh→reopen).
    // See docs/research/wechat-booking-flow.md §验证码.
    let code_ok = goal.recognized_captcha().is_some_and(|c| {
        c.chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .take(4)
            .count()
            >= 4
    });
    if !code_ok {
        if let Some(hit) = candidates
            .items
            .iter()
            .find(|c| c.label.trim() == "captcha_image")
        {
            return navigate(
                snap,
                goal,
                FlowDomain::XmuBooking,
                "现在：确认预约",
                "下一步：刷新验证码图",
                "captcha_image",
                hit,
                Page::ConfirmOrder,
                "confirm_order",
            );
        }
    }
    if let Some(hit) = candidates.find_kind(CandidateKind::CaptchaHit) {
        return navigate(
            snap,
            goal,
            FlowDomain::XmuBooking,
            "现在：确认预约",
            "下一步：点击输入区域唤起键盘",
            "captcha_input",
            hit,
            Page::ConfirmOrder,
            "confirm_order",
        );
    }
    stop(
        snap,
        goal,
        FlowDomain::XmuBooking,
        "CAPTCHA_HIT_NOT_LOCALIZED",
        "暂停，验证码输入区没有定位",
    )
}

pub(crate) fn find_soft_key(candidates: &CandidateSet, ch: char) -> Option<&Candidate> {
    // Soft keys live only in the keyboard band (`promote_keyboard` → KeyboardKey).
    // Never match body OCR via substring `find_click` (e.g. 「6」⊂「2026-09-22」).
    candidates
        .items
        .iter()
        .filter(|c| {
            if !c.actionable || c.kind != CandidateKind::KeyboardKey {
                return false;
            }
            let t = c.label.trim();
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(k), None) => k.eq_ignore_ascii_case(&ch),
                _ => false,
            }
        })
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

pub(crate) fn find_keyboard_done(candidates: &CandidateSet) -> Option<&Candidate> {
    candidates
        .items
        .iter()
        .filter(|c| {
            c.actionable
                && c.kind == CandidateKind::KeyboardKey
                && c.label.trim() == "完成"
        })
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}


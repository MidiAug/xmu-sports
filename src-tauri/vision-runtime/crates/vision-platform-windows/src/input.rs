//! Guarded SendInput — only accepts GuardedAction; never raw coordinates from UI.
//!
//! Multi-monitor + per-monitor DPI (e.g. WeChat on a 200% secondary):
//! - Map frame pixels with the frame's `screen_transform` (physical under PMv2).
//! - Normalize against the **virtual desktop** with `MOUSEEVENTF_VIRTUALDESK`.
//! - Never use `SM_CXSCREEN` alone — that only covers the primary monitor.

use vision_contracts::{ActionType, Frame, GuardedAction, ReasonCode, TargetSurfaceLease};
use vision_platform_api::{InputError, InputReceipt};

#[cfg(windows)]
pub fn inject_guarded(
    action: &GuardedAction,
    lease: &TargetSurfaceLease,
    fresh_frame: &Frame,
) -> Result<InputReceipt, InputError> {
    if fresh_frame.lease_id != lease.lease_id || fresh_frame.generation != lease.generation {
        return Err(InputError::Message(ReasonCode::LEASE_INVALID.into()));
    }
    if action.verified_generation != lease.generation {
        return Err(InputError::Message(
            ReasonCode::LEASE_REBIND_REQUIRED.into(),
        ));
    }
    if action.plan.lease_id != lease.lease_id {
        return Err(InputError::Message(ReasonCode::LEASE_INVALID.into()));
    }

    match action.plan.action_type {
        ActionType::Click | ActionType::SubmitOnce => inject_click(action, fresh_frame),
        ActionType::TypeText => inject_type_text(action),
        ActionType::Wait | ActionType::Stop | ActionType::Rebind => Ok(InputReceipt {
            accepted: true,
            detail: format!("no_input:{:?}", action.plan.action_type),
        }),
        other => Err(InputError::Message(format!(
            "action_type not implemented for live input: {other:?}"
        ))),
    }
}

#[cfg(windows)]
fn inject_click(action: &GuardedAction, fresh_frame: &Frame) -> Result<InputReceipt, InputError> {
    use windows::Win32::UI::HiDpi::{
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    let box_ = action
        .plan
        .anchor_box
        .as_ref()
        .ok_or_else(|| InputError::Message("click requires anchor_box".into()))?;
    let (px, py) = box_.center();
    // Content pixel → screen (transform must already be physical-space under PMv2).
    let screen_x =
        fresh_frame.screen_transform.origin_x + (px as f64) * fresh_frame.screen_transform.scale_x;
    let screen_y =
        fresh_frame.screen_transform.origin_y + (py as f64) * fresh_frame.screen_transform.scale_y;

    unsafe {
        let prev = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN) as f64;
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN) as f64;
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as f64;
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as f64;
        // MSDN / community: subtract virtual origin, then normalize to 0..65535 over virtual size.
        let abs_x = (((screen_x - vx) * 65535.0) / (vw - 1.0).max(1.0)).round() as i32;
        let abs_y = (((screen_y - vy) * 65535.0) / (vh - 1.0).max(1.0)).round() as i32;
        let desk = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;

        let mut inputs = [
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: abs_x,
                        dy: abs_y,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_MOVE | desk,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: abs_x,
                        dy: abs_y,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTDOWN | desk,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: abs_x,
                        dy: abs_y,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTUP | desk,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        ];
        let sent = SendInput(&mut inputs, std::mem::size_of::<INPUT>() as i32);
        let _ = SetThreadDpiAwarenessContext(prev);
        if sent as usize != inputs.len() {
            return Err(InputError::Message("SendInput incomplete".into()));
        }

        log::info!(
            target: "input",
            "[click] frame=({px},{py}) origin=({:.1},{:.1}) scale=({:.3},{:.3}) screen=({screen_x:.1},{screen_y:.1}) virt=({vx},{vy},{vw}x{vh}) abs=({abs_x},{abs_y}) plan={}",
            fresh_frame.screen_transform.origin_x,
            fresh_frame.screen_transform.origin_y,
            fresh_frame.screen_transform.scale_x,
            fresh_frame.screen_transform.scale_y,
            action.plan.action_plan_id
        );
    }

    Ok(InputReceipt {
        accepted: true,
        detail: format!(
            "sendinput click screen=({screen_x:.1},{screen_y:.1}) frame=({px},{py}) scale=({:.3},{:.3}) plan={}",
            fresh_frame.screen_transform.scale_x,
            fresh_frame.screen_transform.scale_y,
            action.plan.action_plan_id
        ),
    })
}

#[cfg(windows)]
fn inject_type_text(action: &GuardedAction) -> Result<InputReceipt, InputError> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };

    let text = action
        .plan
        .text
        .as_deref()
        .ok_or_else(|| InputError::Message("type_text requires plan.text".into()))?;
    if text.is_empty() {
        return Err(InputError::Message("type_text empty".into()));
    }

    let mut inputs: Vec<INPUT> = Vec::with_capacity(text.chars().count() * 2);
    for ch in text.chars() {
        let code = ch as u32;
        if code > 0xFFFF {
            return Err(InputError::Message(format!(
                "type_text only BMP for now: U+{code:X}"
            )));
        }
        let scan = code as u16;
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: Default::default(),
                    wScan: scan,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: Default::default(),
                    wScan: scan,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }

    unsafe {
        let sent = SendInput(inputs.as_mut_slice(), std::mem::size_of::<INPUT>() as i32);
        if sent as usize != inputs.len() {
            return Err(InputError::Message("SendInput type incomplete".into()));
        }
    }
    log::info!(
        target: "input",
        "[type] chars={} plan={}",
        text.chars().count(),
        action.plan.action_plan_id
    );
    Ok(InputReceipt {
        accepted: true,
        detail: format!(
            "sendinput type chars={} plan={}",
            text.chars().count(),
            action.plan.action_plan_id
        ),
    })
}

#[cfg(not(windows))]
pub fn inject_guarded(
    _action: &GuardedAction,
    _lease: &TargetSurfaceLease,
    _fresh_frame: &Frame,
) -> Result<InputReceipt, InputError> {
    Err(InputError::Message("windows only".into()))
}

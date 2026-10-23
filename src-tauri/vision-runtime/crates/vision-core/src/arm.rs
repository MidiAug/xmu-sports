//! Armed clicks are opt-in. Shadow / 「看这一页」 never set these flags.
//! 核对页「点一下」是人工单步：命令内临时 `input_enabled`，不写挂机 arm 文件。

use crate::candidates::BBox;
use crate::state::Page;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmPreconditions {
    pub input_enabled: bool,
    pub pack_ready: bool,
    pub frames_stable: bool,
    pub guard_passed: bool,
}

pub fn may_arm(pre: &ArmPreconditions) -> bool {
    pre.input_enabled && pre.pack_ready && pre.frames_stable && pre.guard_passed
}

/// Two frames of the same hit box. Drift is center-to-center, in frame pixels.
pub fn hit_boxes_stable(a: &BBox, b: &BBox, max_drift_px: i32) -> bool {
    (a.cx() - b.cx()).abs() <= max_drift_px && (a.cy() - b.cy()).abs() <= max_drift_px
}

pub fn safe_click_center(hit: &BBox) -> (i32, i32) {
    (hit.cx(), hit.cy())
}

pub fn transition_holds(expected: Page, actual: Page) -> bool {
    actual == expected
}

/// After a click, the new page must be the one the intent promised.
/// A miss is a stop — never a second blind click.
pub fn after_click(expected: Page, actual: Page) -> Result<(), &'static str> {
    if transition_holds(expected, actual) {
        Ok(())
    } else {
        Err("TRANSITION_FAILED")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_at(x: i32, y: i32) -> BBox {
        BBox {
            x0: x,
            y0: y,
            x1: x + 20,
            y1: y + 20,
        }
    }

    #[test]
    fn arm_requires_every_gate() {
        let ok = ArmPreconditions {
            input_enabled: true,
            pack_ready: true,
            frames_stable: true,
            guard_passed: true,
        };
        assert!(may_arm(&ok));
        assert!(!may_arm(&ArmPreconditions {
            input_enabled: false,
            ..ok
        }));
        assert!(!may_arm(&ArmPreconditions {
            pack_ready: false,
            ..ok
        }));
        assert!(!may_arm(&ArmPreconditions {
            frames_stable: false,
            ..ok
        }));
    }

    #[test]
    fn drift_blocks_stable_click() {
        let a = box_at(10, 10);
        assert!(hit_boxes_stable(&a, &box_at(12, 11), 4));
        assert!(!hit_boxes_stable(&a, &box_at(40, 10), 4));
        assert_eq!(safe_click_center(&a), (20, 20));
    }

    #[test]
    fn wrong_page_does_not_retry() {
        assert!(after_click(Page::WechatDiscover, Page::WechatDiscover).is_ok());
        assert_eq!(
            after_click(Page::Home, Page::WechatDiscover),
            Err("TRANSITION_FAILED")
        );
    }
}

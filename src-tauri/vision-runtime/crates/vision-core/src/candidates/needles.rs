//! Bundled and on-disk gray needles.

use super::types::{BBox, Candidate, CandidateKind, TemplateNeedle};

pub fn load_shell_templates(dir: &std::path::Path) -> Vec<TemplateNeedle> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("gray") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some((id, wh)) = stem.rsplit_once('_') else {
            continue;
        };
        let Some((ws, hs)) = wh.split_once('x') else {
            continue;
        };
        let (Ok(w), Ok(h)) = (ws.parse::<u32>(), hs.parse::<u32>()) else {
            continue;
        };
        if w == 0 || h == 0 || w * h > 64 * 64 {
            continue;
        }
        let Ok(gray) = std::fs::read(&path) else {
            continue;
        };
        if gray.len() != (w * h) as usize {
            continue;
        }
        let label = Candidate {
            template_id: Some(id.to_string()),
            ..empty_for_alias()
        }
        .shell_alias()
        .unwrap_or(id)
        .to_string();
        out.push(TemplateNeedle {
            id: id.to_string(),
            label,
            width: w,
            height: h,
            gray,
        });
    }
    out
}

pub(super) const CENTERED_ICON: u32 = 48;
pub(super) const SHELL_NCC_MIN: f32 = 0.68;

pub(super) struct DiscoverCrop {
    pub(super) id: &'static str,
    pub(super) bytes: &'static [u8],
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(super) fn discover_crops() -> [DiscoverCrop; 4] {
    [
        DiscoverCrop {
            id: "discover_compass_inactive_light",
            bytes: include_bytes!("../../fixtures/discover_tab/inactive_light_84x80.gray"),
            width: 84,
            height: 80,
        },
        DiscoverCrop {
            id: "discover_compass_inactive_dark",
            bytes: include_bytes!("../../fixtures/discover_tab/inactive_dark_103x90.gray"),
            width: 103,
            height: 90,
        },
        DiscoverCrop {
            id: "discover_compass_active_light",
            bytes: include_bytes!("../../fixtures/discover_tab/active_light_106x105.gray"),
            width: 106,
            height: 105,
        },
        DiscoverCrop {
            id: "discover_compass_active_dark",
            bytes: include_bytes!("../../fixtures/discover_tab/active_dark_78x89.gray"),
            width: 78,
            height: 89,
        },
    ]
}

pub(super) fn border_median(src: &[u8], sw: u32, sh: u32) -> u8 {
    if sw == 0 || sh == 0 || src.len() < (sw * sh) as usize {
        return 0;
    }
    let mut border = Vec::with_capacity((sw as usize) * 2 + (sh as usize) * 2);
    for x in 0..sw {
        border.push(src[x as usize]);
        if sh > 1 {
            border.push(src[((sh - 1) * sw + x) as usize]);
        }
    }
    for y in 1..sh.saturating_sub(1) {
        border.push(src[(y * sw) as usize]);
        if sw > 1 {
            border.push(src[(y * sw + sw - 1) as usize]);
        }
    }
    if border.is_empty() {
        return 0;
    }
    border.sort_unstable();
    border[border.len() / 2]
}

/// Weighted centroid of pixels whose neighbor-difference is at least 8.
/// Flat padding does not pull the mass; `None` when the crop has no edges.
pub(super) fn icon_energy_centroid(src: &[u8], sw: u32, sh: u32) -> Option<(f32, f32)> {
    if sw < 3 || sh < 3 || src.len() < (sw * sh) as usize {
        return None;
    }
    let mut energy = vec![0u16; (sw * sh) as usize];
    for y in 1..sh {
        for x in 1..sw {
            let i = (y * sw + x) as usize;
            let g = src[i] as i32;
            let d = (g - src[i - 1] as i32).unsigned_abs()
                + (g - src[i - sw as usize] as i32).unsigned_abs();
            energy[i] = d as u16;
        }
    }
    let mut ranked = energy.clone();
    ranked.sort_unstable();
    let p85 = ranked[(ranked.len() * 85) / 100];
    let thr = p85.max(8);
    let mut sx = 0.0f64;
    let mut sy = 0.0f64;
    let mut wsum = 0.0f64;
    for y in 0..sh {
        for x in 0..sw {
            let e = energy[(y * sw + x) as usize];
            if e >= thr {
                let w = f64::from(e);
                sx += f64::from(x) * w;
                sy += f64::from(y) * w;
                wsum += w;
            }
        }
    }
    if wsum <= 0.0 {
        None
    } else {
        Some(((sx / wsum) as f32, (sy / wsum) as f32))
    }
}

/// Move the high-energy glyph to the middle of a 48×48 needle.
///
/// Hand crops are not framed on the icon.  The shift is an integer, so a
/// second call on an already-centered needle does not walk the glyph again.
pub(super) fn center_icon_gray(src: &[u8], sw: u32, sh: u32) -> Option<Vec<u8>> {
    let (cx, cy) = icon_energy_centroid(src, sw, sh)?;
    let fill = border_median(src, sw, sh);
    let side = CENTERED_ICON;
    let mid = (side as f32 - 1.0) / 2.0;
    let shift_x = (cx - mid).round() as i32;
    let shift_y = (cy - mid).round() as i32;
    let mut out = vec![fill; (side * side) as usize];
    for oy in 0..side {
        for ox in 0..side {
            let sx = ox as i32 + shift_x;
            let sy = oy as i32 + shift_y;
            if sx >= 0 && sy >= 0 && (sx as u32) < sw && (sy as u32) < sh {
                out[(oy * side + ox) as usize] = src[(sy as u32 * sw + sx as u32) as usize];
            }
        }
    }
    Some(out)
}

/// Four theme needles for the PC WeChat Discover compass.
///
/// Source crops keep their original framing (the glyph is not centered).
/// Each needle is centered before match.  Light and dark stay separate
/// because their edge polarity is opposite; signed correlation must not
/// treat that inversion as a hit.  A pack template may still outscore these.
pub fn bundled_shell_templates() -> Vec<TemplateNeedle> {
    discover_crops()
        .into_iter()
        .filter_map(|crop| {
            if crop.bytes.len() != (crop.width * crop.height) as usize {
                return None;
            }
            let gray = center_icon_gray(crop.bytes, crop.width, crop.height)?;
            if gray.len() != (CENTERED_ICON * CENTERED_ICON) as usize {
                return None;
            }
            Some(TemplateNeedle {
                id: crop.id.into(),
                label: "发现".into(),
                width: CENTERED_ICON,
                height: CENTERED_ICON,
                gray,
            })
        })
        .collect()
}

/// Glyph needles for the Discover-list mini-program row.
///
/// Built from the four reference crops: icon only, centered to 48×48.
/// Active light and active dark share the same white-ring luminance, so
/// signed correlation separates Inactive from Active, not the two greens.
/// Hover is not a fifth needle.
pub(super) fn bundled_miniprogram_templates() -> Vec<TemplateNeedle> {
    const SIDE: u32 = 48;
    const N: usize = (SIDE * SIDE) as usize;
    [
        (
            "miniprogram_entry_inactive_light",
            include_bytes!("../../fixtures/miniprogram_entry/inactive_light_48x48.gray"),
        ),
        (
            "miniprogram_entry_inactive_dark",
            include_bytes!("../../fixtures/miniprogram_entry/inactive_dark_48x48.gray"),
        ),
        (
            "miniprogram_entry_active_light",
            include_bytes!("../../fixtures/miniprogram_entry/active_light_48x48.gray"),
        ),
        (
            "miniprogram_entry_active_dark",
            include_bytes!("../../fixtures/miniprogram_entry/active_dark_48x48.gray"),
        ),
    ]
    .into_iter()
    .filter_map(|(id, bytes)| {
        if bytes.len() != N {
            return None;
        }
        Some(TemplateNeedle {
            id: id.into(),
            label: "小程序".into(),
            width: SIDE,
            height: SIDE,
            gray: bytes.to_vec(),
        })
    })
    .collect()
}

/// Crest needles for 「厦大体育」. Dark is the white disc; light is the mark on white.
/// The caption under the disc is not part of the needle — the hit box grows to it.
pub(super) fn bundled_xmu_templates() -> Vec<TemplateNeedle> {
    bundled_gray_needles(&[
        (
            "xmu_sports_icon_dark",
            "厦大体育",
            48,
            include_bytes!("../../fixtures/xmu_sports/icon_dark_48x48.gray").as_slice(),
        ),
        (
            "xmu_sports_icon_light",
            "厦大体育",
            48,
            include_bytes!("../../fixtures/xmu_sports/icon_light_48x48.gray").as_slice(),
        ),
    ])
}

/// Panel header magnifier. Light and dark stay separate: the stroke polarity flips.
pub(super) fn bundled_mp_search_templates() -> Vec<TemplateNeedle> {
    bundled_gray_needles(&[
        (
            "mp_search_light",
            "搜索",
            32,
            include_bytes!("../../fixtures/mp_search/light_32x32.gray").as_slice(),
        ),
        (
            "mp_search_dark",
            "搜索",
            32,
            include_bytes!("../../fixtures/mp_search/dark_32x32.gray").as_slice(),
        ),
    ])
}

pub(super) fn bundled_gray_needles(rows: &[(&str, &str, u32, &[u8])]) -> Vec<TemplateNeedle> {
    rows.iter()
        .filter_map(|(id, label, side, bytes)| {
            let n = (*side * *side) as usize;
            if bytes.len() != n {
                return None;
            }
            Some(TemplateNeedle {
                id: (*id).into(),
                label: (*label).into(),
                width: *side,
                height: *side,
                gray: bytes.to_vec(),
            })
        })
        .collect()
}

pub(super) fn empty_for_alias() -> Candidate {
    Candidate {
        id: String::new(),
        bbox: BBox {
            x0: 0,
            y0: 0,
            x1: 1,
            y1: 1,
        },
        kind: CandidateKind::Icon,
        label: String::new(),
        score: 0.0,
        sources: Vec::new(),
        actionable: false,
        template_id: None,
        slot_meta: None,
        evidence: Vec::new(),
        element_id: None,
        element_state: None,
        theme: None,
        visual_bbox: None,
        hit_bbox: None,
    }
}

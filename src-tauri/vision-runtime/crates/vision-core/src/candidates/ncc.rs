//! Pixel correlation. No page layout and no candidate identity.
//!
//! Callers pass a rectangle and a needle. Where to look — sidebar, discover
//! list, panel — stays in `template_match`.

use super::types::{BBox, TemplateNeedle};

pub(super) const COARSE_FACTOR: u32 = 4;

pub(super) fn region_gray(
    bgra: &[u8],
    stride: u32,
    x0: u32,
    width: u32,
    y0: u32,
    height: u32,
) -> Vec<u8> {
    let mut out = vec![0u8; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let i = (((y0 + y) * stride + x0 + x) * 4) as usize;
            if i + 2 >= bgra.len() {
                continue;
            }
            let g =
                0.114 * bgra[i] as f32 + 0.587 * bgra[i + 1] as f32 + 0.299 * bgra[i + 2] as f32;
            out[(y * width + x) as usize] = g as u8;
        }
    }
    out
}

pub(super) fn best_coarse_at_scale(
    small: &[u8],
    sw: u32,
    sh: u32,
    needle: &TemplateNeedle,
    scale: f32,
    cx0: u32,
    cx1: u32,
    cy0: u32,
    cy1: u32,
) -> Option<(u32, u32)> {
    let tw = (needle.width as f32 * scale).round().max(12.0) as u32;
    let th = (needle.height as f32 * scale).round().max(12.0) as u32;
    let (coarse, ctw, cth) = coarse_needle(&needle.gray, needle.width, needle.height, tw, th);
    if ctw + 1 >= sw || cth + 1 >= sh || cx0 >= cx1 || cy0 >= cy1 || coarse.is_empty() {
        return None;
    }
    let x_end = cx1.min(sw.saturating_sub(ctw));
    let y_end = cy1.min(sh.saturating_sub(cth));
    if cx0 >= x_end || cy0 >= y_end {
        return None;
    }
    let mut best_score = -1.0f32;
    let mut best_xy: Option<(u32, u32)> = None;
    for y in cy0..y_end {
        for x in cx0..x_end {
            let score = zncc_gray(small, sw, x, y, &coarse, ctw, cth);
            if score > best_score {
                best_score = score;
                best_xy = Some((x, y));
            }
        }
    }
    best_xy
}

pub(super) fn coarse_needle(src: &[u8], sw: u32, sh: u32, tw: u32, th: u32) -> (Vec<u8>, u32, u32) {
    let full = resize_gray(src, sw, sh, tw, th);
    down4(&full, tw, th)
}

pub(super) fn refine_signed(
    bgra: &[u8],
    frame_w: u32,
    frame_h: u32,
    x_limit: u32,
    y_start: u32,
    y_limit: u32,
    needle: &TemplateNeedle,
    tw: u32,
    th: u32,
    x: u32,
    y: u32,
) -> Option<(BBox, f32)> {
    if tw == 0 || th == 0 || x_limit <= tw || y_limit <= y_start.saturating_add(th) {
        return None;
    }
    let resized = resize_gray(&needle.gray, needle.width, needle.height, tw, th);
    let rad = 20u32;
    let x0 = x.saturating_sub(rad);
    let y0 = y.saturating_sub(rad).max(y_start);
    let x1 = (x + rad).min(x_limit - tw - 1);
    let y1 = (y + rad)
        .min(y_limit - th - 1)
        .min(frame_h.saturating_sub(th + 1));
    if x0 > x1 || y0 > y1 {
        return None;
    }
    let mut best = -1.0f32;
    let mut best_xy = (x0, y0);
    let mut yy = y0;
    while yy <= y1 {
        let mut xx = x0;
        while xx <= x1 {
            let score = zncc_score_sparse(bgra, frame_w, xx, yy, &resized, tw, th);
            if score > best {
                best = score;
                best_xy = (xx, yy);
            }
            xx = xx.saturating_add(2);
            if xx == 0 {
                break;
            }
        }
        yy = yy.saturating_add(2);
        if yy == 0 {
            break;
        }
    }
    // Step 2 can sit one pixel off an exact paint.  A 3×3 polish keeps the
    // signed score from dropping under the acceptance floor.
    let (bx, by) = best_xy;
    for yy in by.saturating_sub(1)..=(by + 1).min(y1) {
        for xx in bx.saturating_sub(1)..=(bx + 1).min(x1) {
            let score = zncc_score_sparse(bgra, frame_w, xx, yy, &resized, tw, th);
            if score > best {
                best = score;
                best_xy = (xx, yy);
            }
        }
    }
    Some((
        BBox {
            x0: best_xy.0 as i32,
            y0: best_xy.1 as i32,
            x1: (best_xy.0 + tw) as i32,
            y1: (best_xy.1 + th) as i32,
        },
        best,
    ))
}

pub(super) fn sidebar_gray(bgra: &[u8], stride: u32, width: u32, y0: u32, y1: u32) -> Vec<u8> {
    let height = y1 - y0;
    let mut out = vec![0u8; (width * height) as usize];
    for y in 0..height {
        let src = ((y0 + y) * stride) as usize * 4;
        let dst = (y * width) as usize;
        for x in 0..width {
            let i = src + (x as usize) * 4;
            let g =
                0.114 * bgra[i] as f32 + 0.587 * bgra[i + 1] as f32 + 0.299 * bgra[i + 2] as f32;
            out[dst + x as usize] = g as u8;
        }
    }
    out
}

pub(super) fn down4(src: &[u8], sw: u32, sh: u32) -> (Vec<u8>, u32, u32) {
    let dw = sw / COARSE_FACTOR;
    let dh = sh / COARSE_FACTOR;
    if dw == 0 || dh == 0 || src.len() < (sw * sh) as usize {
        return (Vec::new(), 0, 0);
    }
    let mut out = vec![0u8; (dw * dh) as usize];
    for y in 0..dh {
        for x in 0..dw {
            let mut acc = 0u32;
            let y0 = y * COARSE_FACTOR;
            let x0 = x * COARSE_FACTOR;
            for dy in 0..COARSE_FACTOR {
                let row = ((y0 + dy) * sw + x0) as usize;
                for dx in 0..COARSE_FACTOR {
                    acc += src[row + dx as usize] as u32;
                }
            }
            out[(y * dw + x) as usize] = (acc / 16) as u8;
        }
    }
    (out, dw, dh)
}

pub(super) fn zncc_gray(
    hay: &[u8],
    stride: u32,
    x: u32,
    y: u32,
    needle: &[u8],
    tw: u32,
    th: u32,
) -> f32 {
    let mut sum_i = 0.0f32;
    let mut sum_t = 0.0f32;
    let mut n = 0.0f32;
    for ty in (0..th).step_by(2) {
        let row = ((y + ty) * stride + x) as usize;
        let nrow = (ty * tw) as usize;
        for tx in (0..tw).step_by(2) {
            sum_i += hay[row + tx as usize] as f32;
            sum_t += needle[nrow + tx as usize] as f32;
            n += 1.0;
        }
    }
    if n < 2.0 {
        return -1.0;
    }
    let (mean_i, mean_t) = (sum_i / n, sum_t / n);
    let mut cov = 0.0f32;
    let mut var_i = 0.0f32;
    let mut var_t = 0.0f32;
    for ty in (0..th).step_by(2) {
        let row = ((y + ty) * stride + x) as usize;
        let nrow = (ty * tw) as usize;
        for tx in (0..tw).step_by(2) {
            let a = hay[row + tx as usize] as f32 - mean_i;
            let b = needle[nrow + tx as usize] as f32 - mean_t;
            cov += a * b;
            var_i += a * a;
            var_t += b * b;
        }
    }
    if var_i <= 1.0 || var_t <= 1.0 {
        -1.0
    } else {
        cov / (var_i * var_t).sqrt()
    }
}

pub(super) fn resize_gray(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0; (dw * dh) as usize];
    for y in 0..dh {
        for x in 0..dw {
            let sx = (x * sw / dw).min(sw - 1);
            let sy = (y * sh / dh).min(sh - 1);
            out[(y * dw + x) as usize] = src[(sy * sw + sx) as usize];
        }
    }
    out
}

pub(super) fn zncc_score_sparse(
    bgra: &[u8],
    stride: u32,
    x: u32,
    y: u32,
    needle: &[u8],
    tw: u32,
    th: u32,
) -> f32 {
    let mut sum_i = 0.0_f32;
    let mut sum_t = 0.0_f32;
    let mut n = 0.0_f32;
    for ty in (0..th).step_by(3) {
        for tx in (0..tw).step_by(3) {
            let i = (((y + ty) * stride + x + tx) * 4) as usize;
            sum_i +=
                0.114 * bgra[i] as f32 + 0.587 * bgra[i + 1] as f32 + 0.299 * bgra[i + 2] as f32;
            sum_t += needle[(ty * tw + tx) as usize] as f32;
            n += 1.0;
        }
    }
    if n < 2.0 {
        return -1.0;
    }
    let (mean_i, mean_t) = (sum_i / n, sum_t / n);
    let mut cov = 0.0_f32;
    let mut var_i = 0.0_f32;
    let mut var_t = 0.0_f32;
    for ty in (0..th).step_by(3) {
        for tx in (0..tw).step_by(3) {
            let i = (((y + ty) * stride + x + tx) * 4) as usize;
            let a =
                (0.114 * bgra[i] as f32 + 0.587 * bgra[i + 1] as f32 + 0.299 * bgra[i + 2] as f32)
                    - mean_i;
            let b = needle[(ty * tw + tx) as usize] as f32 - mean_t;
            cov += a * b;
            var_i += a * a;
            var_t += b * b;
        }
    }
    if var_i <= 1.0 || var_t <= 1.0 {
        -1.0
    } else {
        cov / (var_i * var_t).sqrt()
    }
}

pub(super) fn sad_score(bgra: &[u8], stride: u32, x: u32, y: u32, t: &TemplateNeedle) -> f32 {
    let mut acc = 0u32;
    for ty in 0..t.height {
        for tx in 0..t.width {
            let i = (((y + ty) * stride + (x + tx)) * 4) as usize;
            let g = (0.114 * bgra[i] as f32
                + 0.587 * bgra[i + 1] as f32
                + 0.299 * bgra[i + 2] as f32) as u8;
            acc += t.gray[(ty * t.width + tx) as usize].abs_diff(g) as u32;
        }
    }
    let n = (t.width * t.height) as f32;
    1.0 - (acc as f32 / n) / 255.0
}

//! Draw candidate boxes onto BGRA and write BMP evidence (no PII scrub beyond caller crops).

use std::fs;
use std::path::Path;

use vision_core::CandidateBox;

/// Wide buttons keep a few pixels of pad so the stroke stays visible when scaled.
/// Icon-sized targets are already the measured control, so the stroke sits on that edge.
const NEXT_OUTSET: i32 = 6;
const NEXT_STROKE: i32 = 3;

fn click_outset(b: &CandidateBox) -> i32 {
    if b.x1 - b.x0 > 160 {
        NEXT_OUTSET
    } else {
        0
    }
}

pub fn draw_boxes_bgra(width: u32, height: u32, bgra: &mut [u8], boxes: &[CandidateBox]) {
    // Orange = reference / what this step sees (`actionable=false`).
    // Green = the decision's next click (`actionable=true`). Same in 核对 and runtime.
    for b in boxes {
        if b.actionable {
            let pad = click_outset(b);
            draw_rect(
                width,
                height,
                bgra,
                b.x0 - pad,
                b.y0 - pad,
                b.x1 + pad,
                b.y1 + pad,
                [0, 255, 48, 255],
                NEXT_STROKE,
            );
        } else {
            draw_rect(
                width,
                height,
                bgra,
                b.x0,
                b.y0,
                b.x1,
                b.y1,
                [0, 165, 255, 255],
                2,
            );
        }
    }
}

fn draw_rect(
    width: u32,
    height: u32,
    bgra: &mut [u8],
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: [u8; 4],
    stroke: i32,
) {
    let stroke = stroke.max(1);
    for t in 0..stroke {
        stroke_once(width, height, bgra, x0 + t, y0 + t, x1 - t, y1 - t, color);
    }
}

fn stroke_once(
    width: u32,
    height: u32,
    bgra: &mut [u8],
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: [u8; 4],
) {
    let x0 = x0.clamp(0, width as i32 - 1) as u32;
    let y0 = y0.clamp(0, height as i32 - 1) as u32;
    let x1 = x1.clamp(0, width as i32) as u32;
    let y1 = y1.clamp(0, height as i32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for x in x0..x1 {
        put(width, height, bgra, x, y0, color);
        put(width, height, bgra, x, y1.saturating_sub(1), color);
    }
    for y in y0..y1 {
        put(width, height, bgra, x0, y, color);
        put(width, height, bgra, x1.saturating_sub(1), y, color);
    }
}

fn put(width: u32, height: u32, bgra: &mut [u8], x: u32, y: u32, color: [u8; 4]) {
    if x >= width || y >= height {
        return;
    }
    let i = ((y * width + x) * 4) as usize;
    if i + 3 < bgra.len() {
        bgra[i..i + 4].copy_from_slice(&color);
    }
}

pub fn encode_bgra_bmp(width: u32, height: u32, bgra: &[u8]) -> Result<Vec<u8>, String> {
    let row_stride = width * 4;
    let pixel_bytes = (row_stride * height) as usize;
    if bgra.len() < pixel_bytes {
        return Err("pixel buffer too small".into());
    }
    let file_header_size = 14u32;
    let info_size = 40u32;
    let off = file_header_size + info_size;
    let size = off + pixel_bytes as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&off.to_le_bytes());
    out.extend_from_slice(&info_size.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(-(height as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&bgra[..pixel_bytes]);
    Ok(out)
}

pub fn write_bgra_bmp(path: &Path, width: u32, height: u32, bgra: &[u8]) -> Result<(), String> {
    let bytes = encode_bgra_bmp(width, height, bgra)?;
    fs::write(path, bytes).map_err(|e| e.to_string())
}

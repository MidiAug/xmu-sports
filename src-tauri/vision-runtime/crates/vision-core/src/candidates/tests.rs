use super::color::*;
use super::needles::*;
use super::parse::*;
use super::template_match::*;
use super::types::*;
use crate::ocr::{OcrHit, OcrRegionCache};
use crate::slots::SlotState;

fn hit(text: &str, x: i32, y: i32, w: i32, h: i32) -> OcrHit {
    OcrHit {
        text: text.into(),
        x,
        y,
        w,
        h,
        score: 0.9,
    }
}

fn paint_rect(buf: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, bgr: [u8; 3]) {
    for yy in y..y + h {
        for xx in x..x + w {
            let i = ((yy * stride + xx) * 4) as usize;
            buf[i] = bgr[0];
            buf[i + 1] = bgr[1];
            buf[i + 2] = bgr[2];
            buf[i + 3] = 255;
        }
    }
}

fn paint_gray(buf: &mut [u8], stride: u32, x: u32, y: u32, gray: &[u8], w: u32, h: u32) {
    for yy in 0..h {
        for xx in 0..w {
            let i = (((y + yy) * stride + x + xx) * 4) as usize;
            let v = gray[(yy * w + xx) as usize];
            buf[i] = v;
            buf[i + 1] = v;
            buf[i + 2] = v;
            buf[i + 3] = 255;
        }
    }
}

#[test]
fn pure_green_is_mint() {
    assert!(pixel_is_mint(0, 255, 0, &ColorRules::default()));
}

#[test]
fn ocr_label_alone_is_not_the_miniprogram_icon() {
    let ocr = OcrRegionCache::from_hits(vec![hit("小程序", 40, 220, 48, 16)]);
    let mut bgra = vec![40u8; 120 * 400 * 4];
    paint_rect(&mut bgra, 120, 20, 210, 90, 36, [48, 48, 48]);
    let set = parse_candidates(&ParseInput {
        frame_w: 120,
        frame_h: 400,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    assert!(set.shell_state("小程序").is_none());
    assert!(set
        .items
        .iter()
        .all(|c| c.element_id.as_deref() != Some(ELEMENT_MINIPROGRAM)));
}

fn paint_active_discover(bgra: &mut [u8], fw: i32, fh: i32) {
    if let Ok(mut guard) = compass_hint().lock() {
        *guard = None;
    }
    let templates = bundled_shell_templates();
    let compass = templates
        .iter()
        .find(|t| t.id == "discover_compass_active_dark")
        .or_else(|| {
            templates
                .iter()
                .find(|t| t.id.contains("discover_compass_active"))
        })
        .expect("active compass");
    let fill = compass.gray[0];
    // Paint the sidebar strip in the needle's fill so polarity matches.
    for y in 0..fh as u32 {
        for x in 0..(fw as u32 / 8).clamp(48, 140) {
            let i = ((y * fw as u32 + x) * 4) as usize;
            if i + 3 < bgra.len() {
                bgra[i] = fill;
                bgra[i + 1] = fill;
                bgra[i + 2] = fill;
                bgra[i + 3] = 255;
            }
        }
    }
    paint_gray(
        bgra,
        fw as u32,
        4,
        80,
        &compass.gray,
        compass.width,
        compass.height,
    );
}

#[test]
fn glyph_needle_finds_miniprogram_when_ocr_blanks() {
    let (fw, fh) = (600i32, 700i32);
    let needles = bundled_miniprogram_templates();
    let light = needles
        .iter()
        .find(|t| t.id == "miniprogram_entry_inactive_light")
        .expect("needle");
    let mut bgra = vec![235u8; (fw * fh * 4) as usize];
    paint_active_discover(&mut bgra, fw, fh);
    // Moments selected: a green row must not outscore the glyph needle.
    paint_rect(&mut bgra, fw as u32, 80, 80, 120, 36, [80, 180, 40]);
    paint_gray(
        &mut bgra,
        fw as u32,
        90,
        400,
        &light.gray,
        light.width,
        light.height,
    );
    let ocr = OcrRegionCache::from_hits(vec![hit("朋友圈", 120, 88, 48, 16)]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &bundled_shell_templates(),
        rules: &ColorRules::default(),
    });
    assert_eq!(set.shell_state("发现"), Some(ShellVisualState::Active));
    let c = set.find_click("小程序").expect("glyph needle");
    assert_eq!(c.element_id.as_deref(), Some(ELEMENT_MINIPROGRAM));
    assert_eq!(c.shell_visual_state(), Some(ShellVisualState::Inactive));
    assert!(
        c.bbox.cy() > 300,
        "should sit below Moments, got {}",
        c.bbox.cy()
    );
    assert!(c.evidence.iter().any(|e| e.contains("inactive_light")));
}

#[test]
fn miniprogram_row_stops_at_the_feed() {
    let (fw, fh) = (800i32, 700i32);
    let needles = bundled_miniprogram_templates();
    let light = needles
        .iter()
        .find(|t| t.id == "miniprogram_entry_inactive_light")
        .expect("needle");
    let mut bgra = vec![235u8; (fw * fh * 4) as usize];
    paint_active_discover(&mut bgra, fw, fh);
    paint_gray(
        &mut bgra,
        fw as u32,
        110,
        200,
        &light.gray,
        light.width,
        light.height,
    );
    // Feed starts here. The search ROI continues past this edge.
    paint_rect(
        &mut bgra,
        fw as u32,
        220,
        0,
        (fw as u32) - 220,
        fh as u32,
        [30, 40, 180],
    );
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &OcrRegionCache::default(),
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &bundled_shell_templates(),
        rules: &ColorRules::default(),
    });
    let c = set.find_click("小程序").expect("row");
    let visual = c.visual_bbox.as_ref().expect("glyph");
    let hit = c.click_bbox();
    let right = visual.x0 + visual.w() * 3;
    assert!(
        (hit.x1 - right).abs() <= 2,
        "right should end at the label, hit {} visual {} expect {right}",
        hit.x1,
        visual.x1
    );
    assert!(hit.x1 < 280, "still using the search roi, x1={}", hit.x1);
}

#[test]
fn active_glyph_needle_is_not_clicked_as_inactive() {
    let (fw, fh) = (600i32, 700i32);
    let needles = bundled_miniprogram_templates();
    let active = needles
        .iter()
        .find(|t| t.id == "miniprogram_entry_active_dark")
        .expect("needle");
    let mut bgra = vec![40u8; (fw * fh * 4) as usize];
    paint_active_discover(&mut bgra, fw, fh);
    paint_gray(
        &mut bgra,
        fw as u32,
        90,
        200,
        &active.gray,
        active.width,
        active.height,
    );
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &OcrRegionCache::default(),
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &bundled_shell_templates(),
        rules: &ColorRules::default(),
    });
    assert_eq!(set.shell_state("发现"), Some(ShellVisualState::Active));
    let c = set
        .items
        .iter()
        .find(|c| c.shell_alias() == Some("小程序"))
        .expect("active glyph");
    assert_eq!(c.shell_visual_state(), Some(ShellVisualState::Active));
}

#[test]
fn chat_list_does_not_search_miniprogram_without_discover() {
    let (fw, fh) = (600i32, 700i32);
    let needles = bundled_miniprogram_templates();
    let active = needles
        .iter()
        .find(|t| t.id == "miniprogram_entry_active_dark")
        .expect("needle");
    let mut bgra = vec![40u8; (fw * fh * 4) as usize];
    if let Ok(mut guard) = compass_hint().lock() {
        *guard = None;
    }
    // No Discover hit. A chat-list glyph must not become the mini-program row.
    paint_gray(
        &mut bgra,
        fw as u32,
        90,
        200,
        &active.gray,
        active.width,
        active.height,
    );
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &OcrRegionCache::default(),
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &bundled_shell_templates(),
        rules: &ColorRules::default(),
    });
    assert_ne!(set.shell_state("发现"), Some(ShellVisualState::Active));
    assert!(set.shell_state("小程序").is_none());
    assert!(set
        .items
        .iter()
        .all(|c| c.element_id.as_deref() != Some(ELEMENT_MINIPROGRAM)));
}

#[test]
fn fuzzy_glyph_is_not_a_click_label() {
    let ocr = OcrRegionCache::from_hits(vec![hit("小程孛", 10, 10, 40, 16)]);
    let set = parse_candidates(&ParseInput {
        frame_w: 200,
        frame_h: 200,
        ocr: &ocr,
        pixels_bgra: None,
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    assert!(set.find_click("小程序").is_none());
}

#[test]
fn template_alias_survives_wrong_ocr() {
    let ocr = OcrRegionCache::from_hits(vec![hit("小程孛", 4, 4, 8, 8)]);
    let gray = vec![40u8; 64];
    let mut bgra = vec![0u8; 80 * 80 * 4];
    paint_rect(&mut bgra, 80, 10, 12, 8, 8, [40, 40, 40]);
    let templates = [TemplateNeedle {
        id: "miniprogram_entry".into(),
        label: "小程序".into(),
        width: 8,
        height: 8,
        gray,
    }];
    let set = parse_candidates(&ParseInput {
        frame_w: 80,
        frame_h: 80,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &templates,
        rules: &ColorRules::default(),
    });
    let c = set.find_click("小程序").expect("alias");
    assert_eq!(c.shell_alias(), Some("小程序"));
    assert_eq!(c.shell_visual_state(), Some(ShellVisualState::Inactive));
    assert!(c.sources.contains(&CandidateSource::Template));
    assert_eq!(c.element_id.as_deref(), Some(ELEMENT_MINIPROGRAM));
    assert!(c
        .hit_bbox
        .as_ref()
        .is_some_and(|h| h.w() > c.visual_bbox.as_ref().map(|v| v.w()).unwrap_or(1)));
}

#[test]
fn themed_miniprogram_state_ignores_ocr_label() {
    let mut c = blank(
        CandidateKind::Icon,
        BBox {
            x0: 10,
            y0: 20,
            x1: 40,
            y1: 50,
        },
        "小程孛".into(),
        0.9,
        CandidateSource::Template,
        true,
    );
    c.template_id = Some("miniprogram_entry_active_dark".into());
    stamp_shell_identity(&mut c);
    assert_eq!(c.shell_alias(), Some("小程序"));
    assert_eq!(c.element_state, Some(ShellVisualState::Active));
    assert_eq!(c.theme.as_deref(), Some("dark"));
    assert_eq!(c.shell_visual_state(), Some(ShellVisualState::Active));
}

#[test]
fn close_runner_up_does_not_force_shell_state() {
    let mut active = blank(
        CandidateKind::Icon,
        BBox {
            x0: 0,
            y0: 0,
            x1: 10,
            y1: 10,
        },
        "发现".into(),
        0.90,
        CandidateSource::Template,
        true,
    );
    active.template_id = Some("discover_compass_active_light".into());
    stamp_shell_identity(&mut active);
    let mut inactive = active.clone();
    inactive.score = 0.88;
    inactive.template_id = Some("discover_compass_inactive_dark".into());
    inactive.element_state = None;
    stamp_shell_identity(&mut inactive);
    let set = CandidateSet {
        items: vec![active, inactive],
        dedup_iou: 0.3,
    };
    assert!(set.shell_state("发现").is_none());
}

#[test]
fn offcenter_glyph_moves_to_center_and_stays() {
    let (w, h) = (100u32, 90u32);
    let mut src = vec![210u8; (w * h) as usize];
    let (x0, y0, side) = (63u32, 53u32, 18u32);
    for y in y0..y0 + side {
        for x in x0..x0 + side {
            let edge = x == x0 || y == y0 || x + 1 == x0 + side || y + 1 == y0 + side;
            if edge {
                src[(y * w + x) as usize] = 30;
            }
        }
    }
    let (cx, cy) = icon_energy_centroid(&src, w, h).expect("glyph");
    let frame_cx = (w as f32 - 1.0) / 2.0;
    let frame_cy = (h as f32 - 1.0) / 2.0;
    assert!(
        (cx - frame_cx).hypot(cy - frame_cy) > 8.0,
        "synthetic glyph should start off-center, got {cx},{cy}"
    );

    let once = center_icon_gray(&src, w, h).expect("center");
    let (ox, oy) = icon_energy_centroid(&once, CENTERED_ICON, CENTERED_ICON).expect("centered");
    assert!(
        (ox - 23.5).abs() <= 2.0 && (oy - 23.5).abs() <= 2.0,
        "centroid {ox},{oy}"
    );
    let twice = center_icon_gray(&once, CENTERED_ICON, CENTERED_ICON).expect("again");
    let changed = once.iter().zip(&twice).filter(|(a, b)| a != b).count();
    assert_eq!(changed, 0, "second center moved {changed} pixels");
}

#[test]
fn discover_crops_center_after_energy_shift() {
    let mut saw_dark_active = false;
    for crop in discover_crops() {
        let (cx, cy) = icon_energy_centroid(crop.bytes, crop.width, crop.height).expect(crop.id);
        let dx = cx - (crop.width as f32 - 1.0) / 2.0;
        let dy = cy - (crop.height as f32 - 1.0) / 2.0;
        if crop.id.ends_with("_active_dark") {
            assert!(
                dx.hypot(dy) > 4.0,
                "dark active crop should be visibly off-center, offset {dx},{dy}"
            );
            saw_dark_active = true;
        }
        let centered = center_icon_gray(crop.bytes, crop.width, crop.height).expect(crop.id);
        let (ox, oy) =
            icon_energy_centroid(&centered, CENTERED_ICON, CENTERED_ICON).expect(crop.id);
        assert!(
            (ox - 23.5).abs() <= 2.0 && (oy - 23.5).abs() <= 2.0,
            "{} centroid {ox},{oy}",
            crop.id
        );
    }
    assert!(saw_dark_active);
}

#[test]
fn wide_sidebar_compass_stays_fast() {
    let templates = bundled_shell_templates();
    let light = templates
        .iter()
        .find(|t| t.id == "discover_compass_inactive_light")
        .unwrap();
    let (w, h) = (1914i32, 1029i32);
    let mut bgra = vec![light.gray[0]; (w as usize) * (h as usize) * 4];
    paint_gray(
        &mut bgra,
        w as u32,
        8,
        220,
        &light.gray,
        light.width,
        light.height,
    );
    if let Ok(mut guard) = compass_hint().lock() {
        *guard = None;
    }
    let started = std::time::Instant::now();
    let found = match_templates(&bgra, w, h, &templates, None, false);
    let ms = started.elapsed().as_millis();
    assert!(
        found
            .iter()
            .any(|c| c.template_id.as_deref() == Some(light.id.as_str())),
        "missed the compass"
    );
    // Debug builds are several times slower; release should stay well under 100ms.
    let budget_ms = if cfg!(debug_assertions) { 1600 } else { 200 };
    assert!(
        ms < budget_ms,
        "sidebar match took {ms}ms (budget {budget_ms})"
    );
}

#[test]
fn themed_compass_matches_at_sidebar_scales() {
    let templates = bundled_shell_templates();
    assert_eq!(templates.len(), 4);
    for template in &templates {
        let wanted = if template.id.contains("_active_") {
            ShellVisualState::Active
        } else {
            ShellVisualState::Inactive
        };
        for scale in [0.8_f32, 1.0, 1.4] {
            if let Ok(mut guard) = compass_hint().lock() {
                *guard = None;
            }
            let tw = (template.width as f32 * scale).round() as u32;
            let th = (template.height as f32 * scale).round() as u32;
            let scaled = resize_gray(&template.gray, template.width, template.height, tw, th);
            let mut bgra = vec![template.gray[0]; 600 * 420 * 4];
            paint_gray(&mut bgra, 600, 4, 80, &scaled, tw, th);
            if wanted == ShellVisualState::Inactive {
                paint_rect(&mut bgra, 600, 4 + tw - 8, 76, 10, 10, [40, 40, 255]);
            }
            let set = parse_candidates(&ParseInput {
                frame_w: 600,
                frame_h: 420,
                ocr: &OcrRegionCache::default(),
                pixels_bgra: Some(&bgra),
                detections: &[],
                templates: &templates,
                rules: &ColorRules::default(),
            });
            assert_eq!(
                set.shell_state("发现"),
                Some(wanted),
                "{} @ {scale}",
                template.id
            );
            if wanted == ShellVisualState::Inactive {
                assert!(
                    set.find_click("发现").is_some(),
                    "{} @ {scale}",
                    template.id
                );
            }
        }
    }
}

#[test]
fn matching_theme_outranks_inverted_polarity() {
    let templates = bundled_shell_templates();
    for (id, other_id) in [
        (
            "discover_compass_inactive_dark",
            "discover_compass_inactive_light",
        ),
        (
            "discover_compass_inactive_light",
            "discover_compass_inactive_dark",
        ),
        (
            "discover_compass_active_dark",
            "discover_compass_active_light",
        ),
        (
            "discover_compass_active_light",
            "discover_compass_active_dark",
        ),
    ] {
        let mine = templates.iter().find(|t| t.id == id).unwrap();
        let other = templates.iter().find(|t| t.id == other_id).unwrap();
        let mut bgra = vec![mine.gray[0]; 600 * 420 * 4];
        paint_gray(&mut bgra, 600, 4, 80, &mine.gray, mine.width, mine.height);
        if let Ok(mut guard) = compass_hint().lock() {
            *guard = None;
        }
        let mine_score = match_shell_template_ncc(&bgra, 600, 420, mine)
            .map(|(_, score)| score)
            .unwrap_or(-1.0);
        let other_score = match_shell_template_ncc(&bgra, 600, 420, other)
            .map(|(_, score)| score)
            .unwrap_or(-1.0);
        assert!(
            mine_score > other_score,
            "{id} {mine_score} should beat {other_id} {other_score}"
        );
    }
}

#[test]
fn plain_green_disk_loses_to_compass_needle() {
    let templates = bundled_shell_templates();
    let compass = templates
        .iter()
        .find(|t| t.id == "discover_compass_active_light")
        .unwrap();
    if let Ok(mut guard) = compass_hint().lock() {
        *guard = None;
    }
    let mut needle_frame = vec![180u8; 600 * 420 * 4];
    paint_gray(
        &mut needle_frame,
        600,
        4,
        80,
        &compass.gray,
        compass.width,
        compass.height,
    );
    let needle_score = match_shell_template_ncc(&needle_frame, 600, 420, compass)
        .map(|(_, score)| score)
        .expect("needle");

    if let Ok(mut guard) = compass_hint().lock() {
        *guard = None;
    }
    let mut disk_frame = vec![180u8; 600 * 420 * 4];
    paint_disk(&mut disk_frame, 600, 28, 220, 18, [96, 193, 7]);
    let disk_score = match_shell_template_ncc(&disk_frame, 600, 420, compass)
        .map(|(_, score)| score)
        .unwrap_or(-1.0);
    assert!(
        disk_score < needle_score,
        "disk {disk_score} must lose to needle {needle_score}"
    );

    if let Ok(mut guard) = compass_hint().lock() {
        *guard = None;
    }
    paint_disk(&mut needle_frame, 600, 28, 220, 18, [96, 193, 7]);
    let set = parse_candidates(&ParseInput {
        frame_w: 600,
        frame_h: 420,
        ocr: &OcrRegionCache::default(),
        pixels_bgra: Some(&needle_frame),
        detections: &[],
        templates: &templates,
        rules: &ColorRules::default(),
    });
    let hit = set
        .items
        .iter()
        .find(|c| c.shell_alias() == Some("发现"))
        .expect("compass still wins");
    let cy = (hit.bbox.y0 + hit.bbox.y1) / 2;
    assert!(
        (cy - 104).abs() < (cy - 220).abs(),
        "picked the disk at y={cy}"
    );
}

fn paint_disk(buf: &mut [u8], stride: u32, cx: i32, cy: i32, r: i32, bgr: [u8; 3]) {
    let r2 = r * r;
    let y0 = (cy - r).max(0) as u32;
    let x0 = (cx - r).max(0) as u32;
    let y1 = (cy + r) as u32;
    let x1 = (cx + r) as u32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as i32 - cx;
            let dy = y as i32 - cy;
            if dx * dx + dy * dy > r2 {
                continue;
            }
            let i = ((y * stride + x) * 4) as usize;
            if i + 3 >= buf.len() {
                continue;
            }
            buf[i] = bgr[0];
            buf[i + 1] = bgr[1];
            buf[i + 2] = bgr[2];
            buf[i + 3] = 255;
        }
    }
}

#[test]
fn detect_box_takes_ocr_label() {
    let ocr = OcrRegionCache::from_hits(vec![hit("厦大体育", 20, 20, 40, 16)]);
    let det = [DetectBox {
        x0: 18,
        y0: 16,
        x1: 70,
        y1: 48,
        score: 0.8,
        class_name: "icon".into(),
    }];
    let set = parse_candidates(&ParseInput {
        frame_w: 200,
        frame_h: 200,
        ocr: &ocr,
        pixels_bgra: None,
        detections: &det,
        templates: &[],
        rules: &ColorRules::default(),
    });
    let icon = set
        .items
        .iter()
        .find(|c| c.kind == CandidateKind::Icon)
        .unwrap();
    assert_eq!(icon.label, "厦大体育");
    assert!(icon.sources.contains(&CandidateSource::Detect));
    assert!(icon.sources.contains(&CandidateSource::Ocr));
}

#[test]
fn overlapping_detect_boxes_dedup() {
    let ocr = OcrRegionCache::from_hits(vec![]);
    let det = [
        DetectBox {
            x0: 0,
            y0: 0,
            x1: 20,
            y1: 20,
            score: 0.4,
            class_name: "icon".into(),
        },
        DetectBox {
            x0: 1,
            y0: 1,
            x1: 19,
            y1: 19,
            score: 0.9,
            class_name: "icon".into(),
        },
    ];
    let set = parse_candidates(&ParseInput {
        frame_w: 40,
        frame_h: 40,
        ocr: &ocr,
        pixels_bgra: None,
        detections: &det,
        templates: &[],
        rules: &ColorRules::default(),
    });
    assert_eq!(
        set.items
            .iter()
            .filter(|c| c.kind == CandidateKind::Icon)
            .count(),
        1
    );
    assert!(set.items.iter().any(|c| (c.score - 0.9).abs() < 0.01));
}

#[test]
fn mint_cell_aligns_to_time_not_legend() {
    let rules = ColorRules::default();
    let w = 200u32;
    let h = 400u32;
    let mut bgra = vec![255u8; (w * h * 4) as usize];
    paint_rect(&mut bgra, w, 80, 180, 24, 16, [0, 255, 0]);
    let ocr = OcrRegionCache::from_hits(vec![
        hit("19:30", 8, 176, 36, 16),
        hit("可预约", 8, 40, 40, 16),
    ]);
    let set = parse_candidates(&ParseInput {
        frame_w: w as i32,
        frame_h: h as i32,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &rules,
    });
    let cell = set.best_available_slot(Some("19:30")).expect("cell");
    assert_eq!(cell.slot_meta.as_ref().unwrap().state, SlotState::Available);
    assert!(cell.evidence.iter().any(|e| e.contains("mint_aligned")));
    assert!(set.find_click("可预约").is_none());
}

#[test]
fn legend_overlap_drops_mint() {
    let w = 200u32;
    let h = 400u32;
    let mut bgra = vec![255u8; (w * h * 4) as usize];
    paint_rect(&mut bgra, w, 80, 180, 24, 16, [0, 255, 0]);
    let ocr = OcrRegionCache::from_hits(vec![
        hit("19:30", 8, 176, 36, 16),
        hit("可预约", 78, 178, 30, 18),
    ]);
    let set = parse_candidates(&ParseInput {
        frame_w: w as i32,
        frame_h: h as i32,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    assert!(set.best_available_slot(Some("19:30")).is_none());
}

#[test]
fn captcha_hit_sits_right_of_noisy_patch() {
    let w = 280u32;
    let h = 120u32;
    let mut bgra = vec![235u8; (w * h * 4) as usize];
    // Captcha tile (noisy).
    for y in 40..70 {
        for x in 90..140 {
            let i = ((y * w + x) * 4) as usize;
            let n = ((x * 17 + y * 13) % 80) as u8;
            bgra[i] = 140 + n;
            bgra[i + 1] = 150;
            bgra[i + 2] = 160;
        }
    }
    let ocr = OcrRegionCache::from_hits(vec![hit("验证码", 10, 48, 50, 16)]);
    let set = parse_candidates(&ParseInput {
        frame_w: w as i32,
        frame_h: h as i32,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let cap = set.find_kind(CandidateKind::CaptchaHit).expect("hit");
    assert!(
        cap.bbox.x0 >= 140,
        "click hit must sit right of the captcha tile, x0={}",
        cap.bbox.x0
    );
    assert!(cap.evidence.iter().any(|e| e == "right_of_captcha_image"));
    // Orange reference markers: captcha tile + input band (not four underline slots).
    let image = set
        .items
        .iter()
        .find(|c| c.label == "captcha_image")
        .expect("captcha_image marker");
    assert!(
        !image.actionable,
        "captcha_image is an orange reference, not a click"
    );
    assert!(
        image.bbox.x1 <= cap.bbox.x0 + 8,
        "tile must end before the click hit, tile.x1={} hit.x0={}",
        image.bbox.x1,
        cap.bbox.x0
    );
    let band = set
        .items
        .iter()
        .find(|c| c.label == "captcha_input_band")
        .expect("input band");
    assert!(!band.actionable);
    assert!(
        band.bbox.x0 >= image.bbox.x1 - 2,
        "input band must start at/after tile, band.x0={} tile.x1={}",
        band.bbox.x0,
        image.bbox.x1
    );
    assert!(
        band.bbox.x1 == w as i32,
        "input band must reach the viewport edge, x1={}",
        band.bbox.x1
    );
}

#[test]
fn geometric_qwerty_fills_missing_letter_p() {
    // Chrome「取消」only — no letter OCR. Geometry must still offer「p」.
    let ocr = OcrRegionCache::from_hits(vec![
        hit("验证码", 40, 200, 50, 16),
        hit("取消", 24, 430, 40, 20),
        hit("完成", 440, 430, 40, 20),
    ]);
    let set = parse_candidates(&ParseInput {
        frame_w: 520,
        frame_h: 719,
        ocr: &ocr,
        pixels_bgra: None,
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let p = set.items.iter().find(|c| {
        c.kind == CandidateKind::KeyboardKey
            && c.label.eq_ignore_ascii_case("p")
            && c.actionable
    });
    assert!(p.is_some(), "geometric keymap must expose soft key p");
    assert!(
        set.items
            .iter()
            .any(|c| c.kind == CandidateKind::KeyboardKey && c.label == "完成"),
        "toolbar 完成 must be present"
    );
    let letters = set
        .items
        .iter()
        .filter(|c| c.kind == CandidateKind::KeyboardKey && is_soft_key_glyph(&c.label))
        .count();
    assert!(letters >= 26, "expected full alphabet, got {letters}");
}

#[test]
fn white_key_caps_label_qwerty_rows() {
    // Realistic WeChat sheet (~240) + white caps (~252). Old thr v>=232 merges
    // sheet+keys; v>=248 must still separate and label by QWERTY order.
    let w = 400i32;
    let h = 700i32;
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    for y in 400..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            bgra[i] = 240;
            bgra[i + 1] = 240;
            bgra[i + 2] = 240;
            bgra[i + 3] = 255;
        }
    }
    let rows: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"];
    let body_top = 460i32;
    let row_h = 50i32;
    let mut expected_p_cx = 0i32;
    for (ri, row) in rows.iter().enumerate() {
        let n = row.chars().count() as i32;
        let cell = 36i32;
        let row_w = cell * n;
        let x0 = (w - row_w) / 2;
        let y0 = body_top + ri as i32 * row_h;
        for (i, ch) in row.chars().enumerate() {
            let kx = x0 + i as i32 * cell + 4;
            let ky = y0 + 6;
            if ch == 'p' {
                expected_p_cx = kx + 14;
            }
            for yy in ky..ky + 28 {
                for xx in kx..kx + 28 {
                    if xx < 0 || yy < 0 || xx >= w || yy >= h {
                        continue;
                    }
                    let i = ((yy * w + xx) * 4) as usize;
                    bgra[i] = 252;
                    bgra[i + 1] = 252;
                    bgra[i + 2] = 252;
                    bgra[i + 3] = 255;
                }
            }
        }
    }
    let ocr = OcrRegionCache::from_hits(vec![
        hit("取消", 20, 420, 40, 18),
        hit("完成", 340, 420, 40, 18),
    ]);
    let set = parse_candidates(&ParseInput {
        frame_w: w,
        frame_h: h,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let letters = set
        .items
        .iter()
        .filter(|c| c.kind == CandidateKind::KeyboardKey && is_soft_key_glyph(&c.label))
        .count();
    assert!(letters >= 36, "expected full qwerty, got {letters}");
    let p = set
        .items
        .iter()
        .find(|c| c.kind == CandidateKind::KeyboardKey && c.label == "p")
        .expect("p from white caps");
    assert!(
        p.evidence.iter().any(|e| e == "keyboard_cap"),
        "expected edge/cap evidence, got {:?}",
        p.evidence
    );
    assert!(p.bbox.cx() > w / 2, "p cx={} should be right half", p.bbox.cx());
    assert!(
        (p.bbox.cx() - expected_p_cx).abs() < 20,
        "p cx={} expected near {}",
        p.bbox.cx(),
        expected_p_cx
    );
}

#[test]
fn white_key_caps_overwrite_drifted_ocr_glyph_box() {
    // OCR places 「l」in the gap between k and l; caps must overwrite the hit box.
    let w = 400i32;
    let h = 700i32;
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    for y in 400..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            bgra[i] = 240;
            bgra[i + 1] = 240;
            bgra[i + 2] = 240;
            bgra[i + 3] = 255;
        }
    }
    let rows: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"];
    let body_top = 460i32;
    let row_h = 50i32;
    let mut true_l = BBox {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };
    for (ri, row) in rows.iter().enumerate() {
        let n = row.chars().count() as i32;
        let cell = 36i32;
        let row_w = cell * n;
        let x0 = (w - row_w) / 2;
        let y0 = body_top + ri as i32 * row_h;
        for (i, ch) in row.chars().enumerate() {
            let kx = x0 + i as i32 * cell + 4;
            let ky = y0 + 6;
            if ch == 'l' {
                true_l = BBox {
                    x0: kx,
                    y0: ky,
                    x1: kx + 28,
                    y1: ky + 28,
                };
            }
            for yy in ky..ky + 28 {
                for xx in kx..kx + 28 {
                    if xx < 0 || yy < 0 || xx >= w || yy >= h {
                        continue;
                    }
                    let i = ((yy * w + xx) * 4) as usize;
                    bgra[i] = 252;
                    bgra[i + 1] = 252;
                    bgra[i + 2] = 252;
                    bgra[i + 3] = 255;
                }
            }
        }
    }
    // Drifted OCR glyph: tiny box in the gap left of the real「l」cap.
    let drifted = hit("l", true_l.x0 - 18, true_l.y0 + 8, 10, 12);
    let ocr = OcrRegionCache::from_hits(vec![
        hit("取消", 20, 420, 40, 18),
        hit("完成", 340, 420, 40, 18),
        drifted,
    ]);
    let set = parse_candidates(&ParseInput {
        frame_w: w,
        frame_h: h,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let l = set
        .items
        .iter()
        .find(|c| c.kind == CandidateKind::KeyboardKey && c.label.eq_ignore_ascii_case("l"))
        .expect("l key");
    assert!(
        l.evidence.iter().any(|e| e == "keyboard_cap"),
        "cap must overwrite OCR, got {:?}",
        l.evidence
    );
    assert!(
        (l.bbox.cx() - true_l.cx()).abs() < 16,
        "l still drifted: got cx={} true={}",
        l.bbox.cx(),
        true_l.cx()
    );
}

#[test]
fn white_key_caps_ignore_delete_glyph_on_z_row() {
    // Z-row: 7 letter caps + a far-right white blob (delete-key icon). Without
    // truncating to n=7, snap pitch stretches so 「m」lands on the delete key.
    let w = 400i32;
    let h = 700i32;
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    for y in 400..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            bgra[i] = 240;
            bgra[i + 1] = 240;
            bgra[i + 2] = 240;
            bgra[i + 3] = 255;
        }
    }
    let rows: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"];
    let body_top = 460i32;
    let row_h = 50i32;
    let mut expected_m_cx = 0i32;
    for (ri, row) in rows.iter().enumerate() {
        let n = row.chars().count() as i32;
        let cell = 36i32;
        let row_w = cell * n;
        let x0 = (w - row_w) / 2;
        let y0 = body_top + ri as i32 * row_h;
        for (i, ch) in row.chars().enumerate() {
            let kx = x0 + i as i32 * cell + 4;
            let ky = y0 + 6;
            if ch == 'm' {
                expected_m_cx = kx + 14;
            }
            for yy in ky..ky + 28 {
                for xx in kx..kx + 28 {
                    if xx < 0 || yy < 0 || xx >= w || yy >= h {
                        continue;
                    }
                    let i = ((yy * w + xx) * 4) as usize;
                    bgra[i] = 252;
                    bgra[i + 1] = 252;
                    bgra[i + 2] = 252;
                    bgra[i + 3] = 255;
                }
            }
        }
    }
    // Extra white blob on the Z row, far right — mimics delete-key icon.
    let z_y0 = body_top + 3 * row_h + 6;
    for yy in z_y0..z_y0 + 28 {
        for xx in 350..378 {
            let i = ((yy * w + xx) * 4) as usize;
            bgra[i] = 252;
            bgra[i + 1] = 252;
            bgra[i + 2] = 252;
            bgra[i + 3] = 255;
        }
    }
    let ocr = OcrRegionCache::from_hits(vec![
        hit("取消", 20, 420, 40, 18),
        hit("完成", 340, 420, 40, 18),
    ]);
    let set = parse_candidates(&ParseInput {
        frame_w: w,
        frame_h: h,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let m = set
        .items
        .iter()
        .find(|c| c.kind == CandidateKind::KeyboardKey && c.label == "m")
        .expect("m from white caps");
    assert!(
        (m.bbox.cx() - expected_m_cx).abs() < 20,
        "m cx={} expected near {} (delete glyph must not stretch Z-row)",
        m.bbox.cx(),
        expected_m_cx
    );
    assert!(
        m.bbox.cx() < 320,
        "m must stay left of delete blob; got cx={}",
        m.bbox.cx()
    );
}

#[test]
fn keyboard_done_snaps_to_right_chrome_not_left_ocr() {
    // OCR returns a leftish half-glyph for「完成」; green ink on the right must win,
    // so the click center lands on the toolbar green word (not mid-frame).
    let w = 400i32;
    let h = 700i32;
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    for y in 400..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            bgra[i] = 240;
            bgra[i + 1] = 240;
            bgra[i + 2] = 240;
            bgra[i + 3] = 255;
        }
    }
    // Green「完成」ink on the right chrome.
    for y in 418..438 {
        for x in 330..380 {
            let i = ((y * w + x) * 4) as usize;
            bgra[i] = 40; // B
            bgra[i + 1] = 180; // G
            bgra[i + 2] = 50; // R
            bgra[i + 3] = 255;
        }
    }
    let rows: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"];
    let body_top = 460i32;
    let row_h = 50i32;
    for (ri, row) in rows.iter().enumerate() {
        let n = row.chars().count() as i32;
        let cell = 36i32;
        let row_w = cell * n;
        let x0 = (w - row_w) / 2;
        let y0 = body_top + ri as i32 * row_h;
        for (i, _) in row.chars().enumerate() {
            let kx = x0 + i as i32 * cell + 4;
            let ky = y0 + 6;
            for yy in ky..ky + 28 {
                for xx in kx..kx + 28 {
                    if xx < 0 || yy < 0 || xx >= w || yy >= h {
                        continue;
                    }
                    let i = ((yy * w + xx) * 4) as usize;
                    bgra[i] = 252;
                    bgra[i + 1] = 252;
                    bgra[i + 2] = 252;
                    bgra[i + 3] = 255;
                }
            }
        }
    }
    let ocr = OcrRegionCache::from_hits(vec![
        hit("取消", 20, 420, 40, 18),
        // Left-biased OCR half-glyph — must NOT become the click target.
        hit("完成", 200, 420, 20, 18),
    ]);
    let set = parse_candidates(&ParseInput {
        frame_w: w,
        frame_h: h,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let done = set
        .items
        .iter()
        .find(|c| c.kind == CandidateKind::KeyboardKey && c.label == "完成")
        .expect("完成 key");
    assert!(
        done.bbox.cx() > w * 3 / 4,
        "完成 cx={} must be right chrome, not OCR left={}",
        done.bbox.cx(),
        200
    );
    assert!(
        done.evidence.iter().any(|e| e == "keyboard_done_green"),
        "expected green evidence, got {:?}",
        done.evidence
    );
}

#[test]
fn captcha_tinted_background_excludes_left_gap_and_input_reaches_edge() {
    let w = 320u32;
    let h = 120u32;
    let mut bgra = vec![255u8; (w * h * 4) as usize];
    // Confirmation-page captcha: pale blue, not white. The glyph noise makes
    // the broad variance search intentionally start before its left edge.
    for y in 38..76 {
        for x in 120..180 {
            let i = ((y * w + x) * 4) as usize;
            bgra[i] = 254; // B
            bgra[i + 1] = 251; // G
            bgra[i + 2] = 243; // R
            if (x * 17 + y * 13) % 17 == 0 {
                bgra[i + 2] = 150;
            }
        }
    }
    // An opaque captcha character covers a short vertical strip. This must not
    // split one pale-blue captcha image into two independent boxes.
    for y in 38..76 {
        for x in 143..155 {
            let i = ((y * w + x) * 4) as usize;
            bgra[i] = 120;
            bgra[i + 1] = 50;
            bgra[i + 2] = 170;
        }
    }
    let ocr = OcrRegionCache::from_hits(vec![hit("验证码", 10, 50, 48, 16)]);
    let set = parse_candidates(&ParseInput {
        frame_w: w as i32,
        frame_h: h as i32,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let image = set.items.iter().find(|c| c.label == "captcha_image").expect("image");
    assert_eq!(image.bbox.x0, 120, "must not include the left white gap");
    assert_eq!(image.bbox.x1, 180);
    let band = set.items.iter().find(|c| c.label == "captcha_input_band").expect("band");
    assert_eq!(band.bbox.x1, w as i32);
    assert_eq!(band.bbox.y0, image.bbox.y0);
    assert_eq!(band.bbox.y1, image.bbox.y1);
}

#[test]
fn merge_typed_proxy_prefers_ink_when_ocr_lags() {
    // After a soft-key click, full-frame OCR may still miss the new glyph while
    // ink columns already show progress.
    assert_eq!(super::merge_typed_proxy("", 2), "XX");
    assert_eq!(super::merge_typed_proxy("P", 2), "XX");
    assert_eq!(super::merge_typed_proxy("PL", 1), "PL");
    assert_eq!(super::merge_typed_proxy("plra", 0), "PLRA");
    assert_eq!(super::merge_typed_proxy("", 0), "");
}

#[test]
fn captcha_keyboard_keeps_full_pale_blue_tile_with_ink_on_left() {
    let (w, h) = (400u32, 130u32);
    let mut bgra = vec![255u8; (w * h * 4) as usize];
    for y in 42..82 {
        for x in 120..220 {
            let i = ((y * w + x) * 4) as usize;
            bgra[i..i + 4].copy_from_slice(&[254, 251, 243, 255]);
            // Character strokes cover much of the left half, but leave some
            // background visible above/below each glyph.
            if (130..185).contains(&x) && (49..73).contains(&y) && (x / 8) % 2 == 0 {
                bgra[i..i + 4].copy_from_slice(&[124, 98, 103, 255]);
            }
        }
    }
    let ocr = OcrRegionCache::from_hits(vec![hit("验证码", 10, 54, 48, 16)]);
    let state = super::analyze_captcha_input(&bgra, w as i32, h as i32, &ocr).expect("captcha");
    assert_eq!(state.image.x0, 120);
    assert_eq!(state.image.x1, 220, "all four glyphs must remain in the image");
    assert_eq!(state.filled_count, 0, "captcha ink is not typed input");
    assert!(state.input_region.x0 >= 220);
}

#[test]
fn panel_crest_tile_beats_games_strip() {
    let (fw, fh) = (900i32, 800i32);
    // Two OCR captions: one in Favorites, one under 小游戏. Only the upper
    // tile may become the click target.
    let ocr = OcrRegionCache::from_hits(vec![
        hit("最近使用", 300, 120, 64, 16),
        hit("厦大体育", 420, 260, 72, 18),
        hit("小游戏", 300, 560, 48, 16),
        hit("厦大体育", 420, 640, 72, 18),
    ]);
    let bgra = vec![40u8; (fw * fh * 4) as usize];
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let hit = set.find_click("厦大体育").expect("crest tile");
    assert_eq!(hit.element_id.as_deref(), Some(ELEMENT_XMU_SPORTS));
    assert!(
        hit.bbox.cy() < 400,
        "must not pick the games crest, cy={}",
        hit.bbox.cy()
    );
    assert!(hit.click_bbox().h() > hit.visual_bbox.as_ref().unwrap().h());
}

#[test]
fn panel_search_magnifier_when_no_crest() {
    let (fw, fh) = (900i32, 800i32);
    let search = bundled_mp_search_templates();
    let dark = search
        .iter()
        .find(|t| t.id == "mp_search_dark")
        .expect("search");
    let mut bgra = vec![40u8; (fw * fh * 4) as usize];
    paint_active_discover(&mut bgra, fw, fh);
    paint_gray(
        &mut bgra,
        fw as u32,
        fw as u32 - 80,
        36,
        &dark.gray,
        dark.width,
        dark.height,
    );
    let ocr = OcrRegionCache::from_hits(vec![hit("最近使用", 300, 120, 64, 16)]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &bundled_shell_templates(),
        rules: &ColorRules::default(),
    });
    assert!(set.find_click("厦大体育").is_none());
    let hit = set.find_click("搜索").expect("magnifier");
    assert_eq!(hit.element_id.as_deref(), Some(ELEMENT_MP_SEARCH));
}

#[test]
fn panel_caption_ocr_expands_to_crest_tile() {
    let (fw, fh) = (900i32, 800i32);
    let ocr = OcrRegionCache::from_hits(vec![
        hit("最近使用", 300, 120, 64, 16),
        hit("厦大体育", 340, 260, 72, 18),
        hit("小游戏", 300, 560, 48, 16),
    ]);
    let bgra = vec![40u8; (fw * fh * 4) as usize];
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let hit = set.find_click("厦大体育").expect("tile");
    assert_eq!(hit.element_id.as_deref(), Some(ELEMENT_XMU_SPORTS));
    assert!(
        hit.click_bbox().h() > 40,
        "hit must cover crest+caption, h={}",
        hit.click_bbox().h()
    );
    assert!(
        hit.click_bbox().y0 < 260,
        "hit must reach above the caption, y0={}",
        hit.click_bbox().y0
    );
    assert!(hit
        .evidence
        .iter()
        .any(|e| e.contains("xmu_tile_from_caption")));
}

#[test]
fn venue_caption_over_purple_disc_is_one_tile() {
    let (fw, fh) = (514i32, 780i32);
    let ocr = OcrRegionCache::from_hits(vec![hit("场馆预约", 180, 420, 72, 18)]);
    let mut bgra = vec![255u8; (fw * fh * 4) as usize];
    // Disc the promoter will look for: caption width, sitting just above the words.
    paint_rect(&mut bgra, fw as u32, 180, 341, 72, 72, [251, 120, 178]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let hit = set.find_click("场馆预约").expect("tile");
    assert_eq!(hit.element_id.as_deref(), Some(ELEMENT_VENUE_BOOKING));
    let b = hit.click_bbox();
    assert!(b.y0 < 360, "top edge is the icon, y0={}", b.y0);
    assert!(b.y1 >= 430, "bottom edge includes the caption, y1={}", b.y1);
    assert!(b.x0 <= 182 && b.x1 >= 250, "covers icon and words");
    assert!(b.w() < 100, "no column padding, w={}", b.w());
    assert!(hit
        .evidence
        .iter()
        .any(|e| e.contains("venue_disc_measured")));
}

#[test]
fn venue_disc_box_follows_the_purple_not_the_caption_center() {
    let (fw, fh) = (514i32, 780i32);
    let ocr = OcrRegionCache::from_hits(vec![hit("场馆预约", 220, 420, 80, 18)]);
    let mut bgra = vec![255u8; (fw * fh * 4) as usize];
    paint_rect(&mut bgra, fw as u32, 190, 360, 48, 48, [251, 120, 178]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let b = set.find_click("场馆预约").expect("tile").click_bbox();
    assert!(b.x0 <= 192 && b.x1 >= 236, "includes the disc");
    assert!(b.x1 >= 298, "includes the caption");
    assert!(b.y0 < 370 && b.y1 > 430);
}

#[test]
fn venue_caption_without_purple_disc_stays_text() {
    let (fw, fh) = (514i32, 780i32);
    let ocr = OcrRegionCache::from_hits(vec![hit("场馆预约", 180, 420, 72, 18)]);
    let bgra = vec![255u8; (fw * fh * 4) as usize];
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let hit = set.find_click("场馆预约").expect("caption");
    assert_ne!(hit.element_id.as_deref(), Some(ELEMENT_VENUE_BOOKING));
}

#[test]
fn notice_ack_is_the_blue_button_under_the_title() {
    let (fw, fh) = (514i32, 780i32);
    let ocr = OcrRegionCache::from_hits(vec![
        hit("【预约须知】", 160, 180, 120, 22),
        hit("我知道了", 200, 520, 64, 20),
    ]);
    let mut bgra = vec![255u8; (fw * fh * 4) as usize];
    paint_rect(&mut bgra, fw as u32, 160, 500, 160, 60, [136, 60, 0]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let hit = set.find_click("我知道了").expect("button");
    assert_eq!(hit.element_id.as_deref(), Some(ELEMENT_NOTICE_ACK));
    let b = hit.click_bbox();
    assert!(
        (b.x0 - 160).abs() <= 4 && (b.y0 - 500).abs() <= 4,
        "box sits on the blue fill, got ({},{}) {}x{}",
        b.x0,
        b.y0,
        b.w(),
        b.h()
    );
    assert!((b.w() - 160).abs() <= 8 && (b.h() - 60).abs() <= 8);
    assert!(hit
        .evidence
        .iter()
        .any(|e| e.contains("notice_ack_on_blue")));
}

#[test]
fn ack_without_notice_title_is_not_the_popup_button() {
    let (fw, fh) = (514i32, 780i32);
    let ocr = OcrRegionCache::from_hits(vec![hit("我知道了", 200, 520, 64, 20)]);
    let mut bgra = vec![255u8; (fw * fh * 4) as usize];
    paint_rect(&mut bgra, fw as u32, 160, 500, 160, 60, [136, 60, 0]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    assert!(set
        .items
        .iter()
        .all(|c| c.element_id.as_deref() != Some(ELEMENT_NOTICE_ACK)));
}

#[test]
fn badminton_caption_over_disc_is_the_sport_tile() {
    let (fw, fh) = (524i32, 780i32);
    let ocr = OcrRegionCache::from_hits(vec![
        hit("我的预约", 340, 36, 64, 16),
        hit("羽毛球", 160, 280, 48, 16),
    ]);
    let mut bgra = vec![255u8; (fw * fh * 4) as usize];
    paint_rect(&mut bgra, fw as u32, 160, 228, 48, 48, [30, 28, 24]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let hit = set.find_click("羽毛球").expect("sport");
    assert_eq!(hit.element_id.as_deref(), Some(ELEMENT_SPORT_ICON));
    let b = hit.click_bbox();
    assert!(b.y0 < 240, "top is the icon, y0={}", b.y0);
    assert!(b.y1 > 280, "bottom includes the caption, y1={}", b.y1);
    assert!(b.w() < 80, "hugs icon plus words, w={}", b.w());
    assert!(set
        .items
        .iter()
        .all(|c| c.element_id.as_deref() != Some(ELEMENT_SPORT_ICON) || c.label == "羽毛球"));
}

#[test]
fn short_and_long_sport_captions_share_one_tile_size() {
    let (fw, fh) = (1054i32, 1563i32);
    let ocr = OcrRegionCache::from_hits(vec![
        hit("篮球", 80, 420, 36, 22),
        hit("动感单车室", 520, 420, 120, 22),
    ]);
    let mut bgra = vec![255u8; (fw * fh * 4) as usize];
    paint_rect(&mut bgra, fw as u32, 40, 250, 160, 160, [30, 28, 24]);
    paint_rect(&mut bgra, fw as u32, 500, 250, 200, 160, [30, 28, 24]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let a = set.find_click("篮球").expect("short");
    let b = set.find_click("动感单车室").expect("long");
    assert!(a.click_bbox().w() > 140, "short caption still uses the icon, w={}", a.click_bbox().w());
    assert!(b.click_bbox().w() > 180, "long icon keeps its own width, w={}", b.click_bbox().w());
    assert!(a.click_bbox().y1 > 420 && b.click_bbox().y1 > 420);
    assert!(a.click_bbox().y0 < 270 && b.click_bbox().y0 < 270);
}

#[test]
fn light_gym_seal_still_promotes_on_a_small_frame() {
    let (fw, fh) = (524i32, 780i32);
    let ocr = OcrRegionCache::from_hits(vec![hit("健身房", 300, 360, 40, 14)]);
    let mut bgra = vec![255u8; (fw * fh * 4) as usize];
    // Thin dark seal, not a solid disc. Most of the circle stays white.
    paint_rect(&mut bgra, fw as u32, 292, 292, 22, 22, [20, 20, 20]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let hit = set.find_click("健身房").expect("gym");
    assert_eq!(hit.element_id.as_deref(), Some(ELEMENT_SPORT_ICON));
    assert!(hit.click_bbox().y0 < 360);
}

#[test]
fn notice_blue_button_without_white_glyphs() {
    let (fw, fh) = (524i32, 780i32);
    let ocr = OcrRegionCache::from_hits(vec![
        hit("【预约须知】", 140, 180, 120, 20),
        hit("羽毛球", 80, 240, 48, 16),
        hit("乒乓球", 160, 240, 48, 16),
    ]);
    let mut bgra = vec![248u8; (fw * fh * 4) as usize];
    paint_rect(&mut bgra, fw as u32, 180, 480, 140, 36, [136, 60, 0]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let hit = set.find_click("我知道了").expect("blue button");
    assert_eq!(hit.element_id.as_deref(), Some(ELEMENT_NOTICE_ACK));
    assert!(hit.click_bbox().y0 >= 460, "button is the blue block, y0={}", hit.click_bbox().y0);
    assert!(hit
        .evidence
        .iter()
        .any(|e| e.contains("notice_blue_button")));
}

#[test]
fn gray_auth_login_stays_disabled_until_the_agreement_is_checked() {
    let (fw, fh) = (520i32, 720i32);
    let ocr = OcrRegionCache::from_hits(vec![
        hit("阅读并同意以下协议", 70, 340, 180, 18),
        hit("授权登录", 210, 420, 72, 18),
    ]);
    let mut bgra = vec![250u8; (fw * fh * 4) as usize];
    paint_rect(&mut bgra, fw as u32, 30, 396, 460, 56, [242, 243, 243]);
    paint_rect(&mut bgra, fw as u32, 230, 400, 24, 24, [136, 60, 0]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    assert!(set
        .items
        .iter()
        .all(|c| c.element_id.as_deref() != Some(ELEMENT_AUTH_LOGIN)));
    let mut frame = crate::state::UiSnapshot::default();
    frame.page = crate::state::Page::XmuWxAuthorize;
    frame.confidence = 0.9;
    let d = crate::intent::decide_next(&frame, &set, &crate::intent::TaskGoal::default());
    assert_eq!(d.next_line, "下一步：勾选协议");
}

#[test]
fn blue_auth_login_is_clickable_after_the_agreement_is_checked() {
    let (fw, fh) = (520i32, 720i32);
    let ocr = OcrRegionCache::from_hits(vec![
        hit("阅读并同意以下协议", 70, 340, 180, 18),
        hit("授权登录", 210, 420, 72, 18),
    ]);
    let mut bgra = vec![250u8; (fw * fh * 4) as usize];
    paint_rect(&mut bgra, fw as u32, 30, 396, 460, 56, [136, 60, 0]);
    let set = parse_candidates(&ParseInput {
        frame_w: fw,
        frame_h: fh,
        ocr: &ocr,
        pixels_bgra: Some(&bgra),
        detections: &[],
        templates: &[],
        rules: &ColorRules::default(),
    });
    let hit = set.find_click("授权登录").expect("button");
    assert_eq!(hit.element_id.as_deref(), Some(ELEMENT_AUTH_LOGIN));
    assert!(hit.click_bbox().w() > 200, "click the blue fill, w={}", hit.click_bbox().w());
    assert!(
        hit.evidence.iter().any(|e| e == "auth_login_enabled"),
        "evidence={:?}",
        hit.evidence
    );
    let mut frame = crate::state::UiSnapshot::default();
    frame.page = crate::state::Page::XmuWxAuthorize;
    frame.confidence = 0.9;
    let d = crate::intent::decide_next(&frame, &set, &crate::intent::TaskGoal::default());
    match d.intent {
        crate::intent::NextIntent::Navigate { ref label, ref anchor, .. } => {
            assert_eq!(label, "授权登录");
            assert!(anchor.x1 - anchor.x0 > 200);
        }
        other => panic!("expected authorize click, got {other:?}"),
    }
}

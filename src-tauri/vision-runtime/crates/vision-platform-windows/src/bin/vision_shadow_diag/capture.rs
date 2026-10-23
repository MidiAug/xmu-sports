//! Interactive page-by-page capture: user navigates WeChat; tool never clicks.
//!
//! Capture never writes ground-truth `page_label` / `expected` from the classifier.
//! Use `confirm` after eye-checking frame.bmp so eval cannot compare prediction to itself.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use vision_core::{
    bundled_shell_templates, load_shell_templates, observe_shadow, suggest_key_boxes, FrameSize,
    LabelSource, OcrSource, PageMapSample, PerceptionFeed, SampleSplit, ShadowObserveInput,
    ShadowTiming, CAPTURE_CHECKLIST,
};
use vision_pack::active_pack_dir;
use vision_platform_windows::{
    discover_wechat_candidates,
    evidence::{draw_boxes_bgra, write_bgra_bmp},
    is_miniprogram_title, is_wechat_title,
    ocr::ocr_bgra_windows_media,
    NativeSurface, WgcCaptureSession,
};

use crate::common::{
    chrono_stamp, data_root, load_page_rules_min_confidence, page_map_root, process_rss_bytes,
    prompt_line,
};

pub fn cmd_capture(args: &[String]) -> Result<(), String> {
    let session_name = crate::common::parse_flag(args, "--session")
        .unwrap_or_else(|| format!("session_{}", chrono_stamp()));
    let out_root = crate::common::parse_flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(page_map_root);
    let session_dir = out_root.join(&session_name);
    let delay_secs: u64 = crate::common::parse_flag(args, "--delay")
        .and_then(|s| s.parse().ok())
        .unwrap_or(8)
        .clamp(3, 30);
    fs::create_dir_all(session_dir.join("samples")).map_err(|e| e.to_string())?;

    let root = data_root();
    if active_pack_dir(&root).is_none() {
        return Err(format!(
            "PACK_NOT_INSTALLED: install recognition pack under {} before capture",
            root.display()
        ));
    }
    let min_conf = load_page_rules_min_confidence(&root);

    let session_meta = json!({
        "session": session_name,
        "created_ms": chrono_stamp(),
        "platform": "windows",
        "input_invoked": false,
        "input_enabled": false,
        "ocr_backend": "windows_media_system_not_bundled",
        "checklist": CAPTURE_CHECKLIST,
        "target_switch": "wechat_main..wechat_mp_search stay on WeChat; from xmu_login_gate rebind to the live mini-program window",
        "label_policy": "capture writes page_label_suggested only; run `confirm` after eye-check",
        "notes": [
            "Manual navigation only. Tool never clicks or types.",
            "Yellow border follows the bound window. Re-opening the mini-program creates a new window; the next shot rebinds.",
            "Do NOT bulk-set human_confirmed=true — use confirm subcommand.",
        ],
    });
    fs::write(
        session_dir.join("session.json"),
        serde_json::to_string_pretty(&session_meta).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    println!("page-map capture session: {}", session_dir.display());
    println!("checklist (core path first):");
    for (i, p) in CAPTURE_CHECKLIST.iter().enumerate() {
        println!("  {:>2}. {p}", i + 1);
    }
    println!();
    println!("Commands each step:");
    println!("  <Enter>     capture now (label stays pending)");
    println!("  w           countdown {delay_secs}s, then capture — use when the page closes if this window is clicked");
    println!("  h           holdout, capture now");
    println!("  hw          holdout + countdown");
    println!("  s / skip    skip this checklist item");
    println!("  q / quit    finish session");
    println!();
    println!("Search popup closes when WeChat loses focus. On `wechat_mp_search`, Enter/h already wait {delay_secs}s.");
    println!("Press the key FIRST, then click WeChat and open search. Do not click this terminal again until the shot is saved.");
    println!();
    println!("After capture: vision-shadow-diag confirm --dir <session>");
    println!("From `xmu_login_gate` the yellow border moves to the mini-program window (步道乐跑 / 厦大体育).");
    println!("If you close and reopen it, the next shot binds the new window.");
    println!();

    let mut seq = 0u32;
    let mut captured_ids: Vec<String> = Vec::new();
    let mut live: Option<LiveBind> = None;
    if let Err(e) = ensure_bind(&mut live, "wechat_main") {
        println!("  启动时没绑上微信，黄框还没有: {e}");
    } else if let Some(b) = live.as_ref() {
        println!(
            "  黄框应在「{}」上。没有的话看是不是微信被挡住了。",
            b.title
        );
    }
    println!();

    for want in CAPTURE_CHECKLIST {
        loop {
            if wants_miniprogram(want) && live.as_ref().map(|b| !b.mini).unwrap_or(true) {
                println!("  从这一页起改拍小程序窗口，不再拍微信主窗口。请先打开「步道乐跑」。");
            }
            if loses_focus(want) {
                println!("  这一页点终端会关掉（搜索框/弹层）。");
                println!("  先在这里按回车，再立刻点回微信把搜索点开，倒计时结束前不要再点终端。");
            }
            let hint = format!("[{}] `{want}`  Enter / w / h / hw / s / q > ", seq + 1);
            let line = prompt_line(&hint)?;
            let Some(action) = parse_step(&line, want, delay_secs) else {
                println!("  看不懂。用 Enter、w、h、hw、s、q");
                continue;
            };
            match action {
                Step::Quit => {
                    write_manifest(&session_dir, &captured_ids)?;
                    println!("stopped. session={}", session_dir.display());
                    return Ok(());
                }
                Step::Skip => {
                    println!("skipped {want}");
                    break;
                }
                Step::Shoot {
                    split,
                    delay_secs: wait,
                } => {
                    if wait > 0 {
                        countdown(wait);
                    }
                    if let Err(e) = ensure_bind(&mut live, want) {
                        println!("  {e}");
                        continue;
                    }
                    let bind = live.as_mut().expect("bound");
                    match capture_one(bind, &session_dir, seq + 1, want, split, min_conf) {
                        Ok(sample) => {
                            seq += 1;
                            captured_ids.push(sample.id.clone());
                            println!(
                                "  saved {} window={} suggested={} label=PENDING split={:?} -> confirm later",
                                sample.id,
                                bind.title,
                                sample.page_label_suggested.as_deref().unwrap_or("-"),
                                sample.split,
                            );
                            break;
                        }
                        Err(e) => {
                            println!("  拍摄失败: {e}");
                            continue;
                        }
                    }
                }
            }
        }
    }

    loop {
        let line = prompt_line("extra frame? Enter / w / h / hw / q > ")?;
        if line.trim().is_empty() {
            break;
        }
        let Some(action) = parse_step(&line, "extra", delay_secs) else {
            println!("  看不懂。用 Enter、w、h、hw、q");
            continue;
        };
        match action {
            Step::Quit | Step::Skip => break,
            Step::Shoot {
                split,
                delay_secs: wait,
            } => {
                if wait > 0 {
                    countdown(wait);
                }
                if let Err(e) = ensure_bind(&mut live, "extra") {
                    println!("  {e}");
                    continue;
                }
                let bind = live.as_mut().expect("bound");
                match capture_one(bind, &session_dir, seq + 1, "extra", split, min_conf) {
                    Ok(sample) => {
                        seq += 1;
                        captured_ids.push(sample.id.clone());
                        println!("  saved {} window={} (PENDING)", sample.id, bind.title);
                    }
                    Err(e) => println!("  拍摄失败: {e}"),
                }
            }
        }
    }

    write_manifest(&session_dir, &captured_ids)?;
    println!(
        "done. Eye-check BMPs, then:\n  vision-shadow-diag confirm --dir {}\n  vision-shadow-diag eval --dir {} --real-ocr-only\n  vision-shadow-diag eval --dir {} --split holdout --real-ocr-only",
        session_dir.display(),
        session_dir.display(),
        session_dir.display()
    );
    Ok(())
}

fn wants_miniprogram(hint: &str) -> bool {
    !matches!(
        hint,
        "wechat_main" | "wechat_discover" | "wechat_mp_panel" | "wechat_mp_search"
    )
}

struct LiveBind {
    session: WgcCaptureSession,
    notes: Vec<String>,
    lease: vision_contracts::TargetSurfaceLease,
    hwnd_u64: u64,
    title: String,
    mini: bool,
}

fn pick_native(mini: bool) -> Result<NativeSurface, String> {
    let mut found = discover_wechat_candidates()?;
    found.retain(|n| {
        if mini {
            is_miniprogram_title(&n.title)
        } else {
            is_wechat_title(&n.title)
        }
    });
    found.sort_by_key(|n| std::cmp::Reverse(n.area));
    found.into_iter().next().ok_or_else(|| {
        if mini {
            "没找到小程序窗口（标题要含「步道乐跑」或「厦大体育」）。先打开它再按回车，这次不会拍微信主窗口。".into()
        } else {
            "没找到微信主窗口".into()
        }
    })
}

fn ensure_bind(live: &mut Option<LiveBind>, hint: &str) -> Result<(), String> {
    let mini = wants_miniprogram(hint);
    let native = pick_native(mini)?;
    if let Some(cur) = live.as_ref() {
        if cur.hwnd_u64 == native.hwnd_u64 && cur.mini == mini {
            return Ok(());
        }
        println!("  黄框切换: 「{}」 -> 「{}」", cur.title, native.title);
    } else {
        println!("  黄框绑到: 「{}」", native.title);
    }
    let (session, notes) = WgcCaptureSession::open(&native)?;
    let lease = native.to_lease(1);
    *live = Some(LiveBind {
        session,
        notes,
        lease,
        hwnd_u64: native.hwnd_u64,
        title: native.title.clone(),
        mini,
    });
    Ok(())
}

fn loses_focus(hint: &str) -> bool {
    // WeChat dismisses the mini-program search box (and similar layers) on deactivate.
    matches!(hint, "wechat_mp_search")
}

enum Step {
    Quit,
    Skip,
    Shoot { split: SampleSplit, delay_secs: u64 },
}

fn parse_step(line: &str, hint: &str, default_delay: u64) -> Option<Step> {
    let lower = line.trim().to_lowercase();
    if lower == "q" || lower == "quit" {
        return Some(Step::Quit);
    }
    if lower == "s" || lower == "skip" {
        return Some(Step::Skip);
    }
    let (holdout, wait) = match lower.as_str() {
        "" => (false, loses_focus(hint)),
        "h" => (true, loses_focus(hint)),
        "w" => (false, true),
        "hw" | "wh" => (true, true),
        _ => return None,
    };
    Some(Step::Shoot {
        split: if holdout {
            SampleSplit::Holdout
        } else {
            SampleSplit::Dev
        },
        delay_secs: if wait { default_delay } else { 0 },
    })
}

fn countdown(secs: u64) {
    println!("  现在去点微信，把要拍的画面点出来。倒计时结束前不要再点这个终端。");
    let _ = io::stdout().flush();
    for left in (1..=secs).rev() {
        println!("  {left}");
        let _ = io::stdout().flush();
        thread::sleep(Duration::from_secs(1));
    }
    println!("  拍摄");
    let _ = io::stdout().flush();
}

fn write_manifest(session_dir: &PathBuf, ids: &[String]) -> Result<(), String> {
    let man = json!({
        "samples": ids.len(),
        "ids": ids,
        "input_invoked": false,
        "pixels_local_only": true,
        "next": "confirm --dir <session> then eval --real-ocr-only (dev + holdout)",
        "import_note": "import scans OCR for sensitive text; --ack-ocr-pii-reviewed required if flagged. No BMP copy.",
    });
    fs::write(
        session_dir.join("manifest.json"),
        serde_json::to_string_pretty(&man).unwrap(),
    )
    .map_err(|e| e.to_string())
}

fn capture_one(
    bind: &mut LiveBind,
    session_dir: &PathBuf,
    seq: u32,
    checklist_hint: &str,
    split: SampleSplit,
    min_conf: f32,
) -> Result<PageMapSample, String> {
    let lease = bind.lease.clone();
    let frame_t0 = Instant::now();
    let (frame, mut notes) = bind.session.grab_frame(&lease)?;
    let frame_ms = frame_t0.elapsed().as_millis() as u64;
    notes.append(&mut bind.notes);
    let device_creates = bind.session.device_creates();
    let frames_grabbed = bind.session.frames_grabbed();

    if !frame.is_usable() {
        return Err(format!("CONTENT_UNUSABLE: status={:?}", frame.status));
    }
    let px = frame
        .pixels
        .as_ref()
        .ok_or_else(|| "no pixels".to_string())?;

    let ocr_t0 = Instant::now();
    let (ocr, ocr_notes) =
        ocr_bgra_windows_media(frame.pixel_width, frame.pixel_height, &px.bytes)?;
    let ocr_ms = ocr_t0.elapsed().as_millis() as u64;
    notes.extend(ocr_notes.iter().cloned());

    let ocr_lang = ocr_notes
        .iter()
        .find_map(|n| n.strip_prefix("ocr:lang=").map(|s| s.to_string()));

    let mut templates = bundled_shell_templates();
    templates.extend(
        active_pack_dir(&data_root())
            .map(|p| load_shell_templates(&p.join("templates").join("wechat_shell")))
            .unwrap_or_default(),
    );
    let result = observe_shadow(ShadowObserveInput {
        run_id: "page-map-capture",
        lease_id: &lease.lease_id,
        generation: lease.generation,
        frame_id: &frame.frame_id,
        frame_w: frame.pixel_width as i32,
        frame_h: frame.pixel_height as i32,
        ocr: &ocr,
        title_hint: lease.title_hint.clone(),
        min_confidence: min_conf,
        target_slot_time: None,
        timing: ShadowTiming {
            frame_ms,
            ocr_ms,
            ocr_full_ms: ocr_ms,
            ocr_roi_ms: 0,
            classify_ms: 0,
            candidates_ms: 0,
            total_ms: 0,
            rss_bytes: process_rss_bytes(),
            cpu_pct: None,
        },
        perception: PerceptionFeed {
            pixels_bgra: Some(px.bytes.as_slice()),
            detections: &[],
            templates: &templates,
        },
    });
    assert!(!result.input_invoked);

    let suggested = result.snapshot.page.as_str().to_string();
    let key_boxes_suggested = suggest_key_boxes(&ocr, result.snapshot.page);

    let sample_id = format!("cap_{seq:03}_{checklist_hint}");
    let sample_dir = session_dir.join("samples").join(&sample_id);
    fs::create_dir_all(&sample_dir).map_err(|e| e.to_string())?;

    let raw_bmp = sample_dir.join("frame.bmp");
    write_bgra_bmp(&raw_bmp, frame.pixel_width, frame.pixel_height, &px.bytes)?;

    let mut evidence = px.bytes.clone();
    draw_boxes_bgra(
        frame.pixel_width,
        frame.pixel_height,
        &mut evidence,
        &result.candidate_boxes,
    );
    let evidence_bmp = sample_dir.join("evidence.bmp");
    write_bgra_bmp(
        &evidence_bmp,
        frame.pixel_width,
        frame.pixel_height,
        &evidence,
    )?;

    let sample = PageMapSample {
        id: sample_id.clone(),
        platform: "windows".into(),
        kind: "page_map_capture".into(),
        split,
        ocr_source: OcrSource::WindowsMedia,
        page_label_suggested: Some(suggested.clone()),
        // Ground truth stays empty until `confirm`.
        page_label: None,
        checklist_hint: Some(checklist_hint.to_string()),
        label_source: LabelSource::Pending,
        human_confirmed: false,
        source: Some(format!("wgc_live:{}", lease.lease_id)),
        nav_note: Some(format!(
            "checklist_hint={checklist_hint}; suggested={suggested}; confirm required"
        )),
        frame: FrameSize {
            width: frame.pixel_width as i32,
            height: frame.pixel_height as i32,
        },
        ocr_hits: ocr.hits().to_vec(),
        key_boxes_suggested,
        key_boxes: Vec::new(),
        expected: None,
        venue_name: None,
        ocr_backend: Some("windows_media_system_not_bundled".into()),
        ocr_language: ocr_lang,
        window: Some(json!({
            "title_hint": lease.title_hint,
            "lease_id": lease.lease_id,
            "generation": lease.generation,
            "surface_fingerprint": lease.surface_fingerprint,
            "pid": lease.owner.pid,
            "app_identity": lease.owner.app_identity,
            "bounds": lease.bounds_in_screen_points,
            "scale_x": lease.scale_x,
            "scale_y": lease.scale_y,
        })),
        capture_meta: Some(json!({
            "frame_id": frame.frame_id,
            "frame_bmp": raw_bmp.display().to_string(),
            "evidence_bmp": evidence_bmp.display().to_string(),
            "shadow_page": result.snapshot.page.as_str(),
            "shadow_confidence": result.snapshot.confidence,
            "shadow_evidence": result.snapshot.evidence,
            "refuse_reason": result.refuse_reason,
            "action_type": "stop",
            "input_invoked": false,
            "wgc_device_creates": device_creates,
            "wgc_frames_grabbed": frames_grabbed,
            "timing": result.timing,
            "notes": notes,
            "redact_before_import": true,
            "min_confidence_at_capture": min_conf,
        })),
    };

    let sample_json = sample_dir.join("sample.json");
    fs::write(&sample_json, serde_json::to_string_pretty(&sample).unwrap())
        .map_err(|e| e.to_string())?;

    Ok(sample)
}

//! vision-shadow-diag — Phase 4 Shadow Mode (observe only, never inject input).
//!
//! Subcommands:
//!   replay   — single annotated/OCR fixture (offline)
//!   live     — continuous WGC + OCR (no annotation UX)
//!   capture  — interactive page-map real capture (manual nav, no clicks)
//!   confirm  — human types page label while looking at frame.bmp
//!   eval     — directory replay + confusion matrix / wrong examples
//!   import   — OCR+labels only; scans sensitive OCR (no BMPs)
//!   promote  — failed run → replay case (P6; desensitize OCR)
//!   corpus   — offline observe_shadow + decide_next corpus metrics (P6)
//!   session  — persistent RunSession read-only static/DPI probe

mod capture;
mod common;
mod confirm;
mod corpus_cmd;
mod eval_cmd;
mod import;

use std::env;
use std::fs;
use std::time::Instant;

use serde_json::json;
use vision_core::{
    bundled_shell_templates, decide_next, load_shell_templates, observe_shadow, CandidateBox,
    OcrHit, PerceptionFeed, ShadowObserveInput, ShadowTiming, TaskGoal, DEFAULT_MIN_CONFIDENCE,
};
use vision_pack::active_pack_dir;
use vision_platform_api::PlatformPort;
use vision_platform_windows::{
    discover_wechat_candidates,
    evidence::{draw_boxes_bgra, write_bgra_bmp},
    is_miniprogram_title,
    ocr::{ocr_bgra_windows_media, ocr_from_annotated_hits},
    CapturePreference, NativeSurface, WgcCaptureSession, WindowsPlatform,
};

use common::{
    data_root, fixture_path, load_page_rules_min_confidence, process_rss_bytes, shadow_out_dir,
};

fn usage() {
    eprintln!(
        "usage:\n\
  vision-shadow-diag observe [--sport S] [--venue V] [--date D] [--time T]\n\
  vision-shadow-diag session [--frames N] [--gap-ms N] [--sport S] [--venue V] [--date D] [--time T]\n\
  vision-shadow-diag replay --fixture <name>\n\
  vision-shadow-diag live [--index N] [--frames N]\n\
  vision-shadow-diag capture [--index N] [--session NAME] [--out DIR]\n\
  vision-shadow-diag confirm --dir SESSION_DIR\n\
  vision-shadow-diag eval --dir DIR [--split dev|holdout|all] [--real-ocr-only] [--allow-unconfirmed]\n\
  vision-shadow-diag import --from SESSION_DIR [--to DIR] [--holdout-only] [--ack-ocr-pii-reviewed]\n\
  vision-shadow-diag promote --run-dir RUN [--case-id ID] [--page P] [--date D]\n\
             [--intent-kind K] [--intent-label L] [--intent-reason R] [--tag T]...\n\
             [--confirm] [--to-repo|--out DIR] [--step N]\n\
  vision-shadow-diag corpus [--dir testdata/replay]\n\
\n\
capture default out: %LOCALAPPDATA%/xmu-sports/diag/page_map/\n\
promote default out: %LOCALAPPDATA%/xmu-sports/diag/replay/<case_id>/\n\
gates: unconfirmed never scores; confirm required for human labels; holdout must cover core path;\n\
       import scans OCR PII (no BMP != redacted)\n\
invariant: input_invoked=false always; no auto navigate/click; Phase 5 forbidden"
    );
}

fn pick_observe_surface(list: &[NativeSurface]) -> Option<&NativeSurface> {
    list.iter()
        .find(|n| n.visible && is_miniprogram_title(&n.title))
        .or_else(|| list.iter().find(|n| n.visible))
        .or_else(|| list.first())
}

fn cmd_observe(args: &[String]) -> Result<(), String> {
    let root = data_root();
    if active_pack_dir(&root).is_none() {
        return Err(format!(
            "PACK_NOT_INSTALLED: install recognition pack under {} before observe",
            root.display()
        ));
    }
    let min_conf = load_page_rules_min_confidence(&root);
    let goal = TaskGoal {
        sport: common::parse_flag(args, "--sport"),
        venue: common::parse_flag(args, "--venue"),
        date: common::parse_flag(args, "--date"),
        slot_time: common::parse_flag(args, "--time"),
        prefer_court: common::parse_flag(args, "--court"),
        ..TaskGoal::default()
    };

    let mut plat = WindowsPlatform::new();
    plat.pack_ready = true;
    plat.input_enabled = false;
    plat.capture_preference = CapturePreference::WgcOnly;
    plat.discover_surfaces("wechat")
        .map_err(|e| e.to_string())?;

    let found = discover_wechat_candidates().map_err(|e| e.to_string())?;
    let native = pick_observe_surface(&found)
        .ok_or_else(|| "no wechat or miniprogram window".to_string())?
        .clone();
    if is_miniprogram_title(&native.title) {
        println!("绑定：独立小程序窗 {}", native.title);
    } else {
        println!("绑定：微信窗 {}", native.title);
    }

    let cand = native.to_candidate();
    let lease = plat
        .bind_surface(&cand.candidate_id)
        .map_err(|e| e.to_string())?;
    let (mut session, _) = WgcCaptureSession::open(&native)?;
    let (frame, notes) = session.grab_frame(&lease)?;
    if !frame.is_usable() {
        return Err(format!("CONTENT_UNUSABLE status={:?}", frame.status));
    }
    let px = frame
        .pixels
        .as_ref()
        .ok_or_else(|| "no pixels".to_string())?;
    let (ocr, ocr_notes) =
        ocr_bgra_windows_media(frame.pixel_width, frame.pixel_height, &px.bytes)?;
    let ocr_lines: Vec<String> = ocr
        .hits()
        .iter()
        .take(60)
        .map(|h| format!("{}@{},{}", h.text, h.x, h.y))
        .collect();

    let mut templates = bundled_shell_templates();
    templates.extend(
        active_pack_dir(&root)
            .map(|p| load_shell_templates(&p.join("templates").join("wechat_shell")))
            .unwrap_or_default(),
    );
    let result = observe_shadow(ShadowObserveInput {
        run_id: "shadow-observe",
        lease_id: &lease.lease_id,
        generation: lease.generation,
        frame_id: &frame.frame_id,
        frame_w: frame.pixel_width as i32,
        frame_h: frame.pixel_height as i32,
        ocr: &ocr,
        title_hint: lease.title_hint.clone(),
        min_confidence: min_conf,
        target_slot_time: goal.slot_time.as_deref(),
        timing: ShadowTiming::default(),
        perception: PerceptionFeed {
            pixels_bgra: Some(px.bytes.as_slice()),
            detections: &[],
            templates: &templates,
        },
    });
    assert!(!result.input_invoked);
    assert_eq!(
        result.action_plan.action_type,
        vision_contracts::ActionType::Stop
    );

    let decision = decide_next(&result.snapshot, &result.candidate_set, &goal);
    let mut boxes = result.candidate_boxes.clone();
    if let vision_core::NextIntent::Navigate { anchor, .. } = &decision.intent {
        boxes.push(CandidateBox {
            label: "next".into(),
            x0: anchor.x0,
            y0: anchor.y0,
            x1: anchor.x1,
            y1: anchor.y1,
            score: decision.confidence,
            actionable: true,
        });
    } else if let vision_core::NextIntent::AtomicComposite { steps, .. } = &decision.intent {
        if let Some(vision_core::AtomicStep::Click { anchor, .. }) = steps.first() {
            boxes.push(CandidateBox {
                label: "next".into(),
                x0: anchor.x0,
                y0: anchor.y0,
                x1: anchor.x1,
                y1: anchor.y1,
                score: decision.confidence,
                actionable: true,
            });
        }
    }

    let mut evidence_pixels = px.bytes.clone();
    draw_boxes_bgra(
        frame.pixel_width,
        frame.pixel_height,
        &mut evidence_pixels,
        &boxes,
    );
    let out = shadow_out_dir();
    let stamp = common::chrono_stamp();
    let bmp = out.join(format!("observe_{stamp}.bmp"));
    write_bgra_bmp(
        &bmp,
        frame.pixel_width,
        frame.pixel_height,
        &evidence_pixels,
    )?;
    let meta = out.join(format!("observe_{stamp}.json"));
    let basis = match &decision.intent {
        vision_core::NextIntent::Navigate { label, anchor, .. } => format!(
            "{label} 框 ({},{})-({},{})",
            anchor.x0, anchor.y0, anchor.x1, anchor.y1
        ),
        vision_core::NextIntent::AtomicComposite { label, steps, .. } => {
            if let Some(vision_core::AtomicStep::Click { anchor, .. }) = steps.first() {
                format!(
                    "{label} 框 ({},{})-({},{})",
                    anchor.x0, anchor.y0, anchor.x1, anchor.y1
                )
            } else {
                label.clone()
            }
        }
        vision_core::NextIntent::Wait { reason, approx_ms } => {
            format!("{reason} {approx_ms}ms")
        }
        vision_core::NextIntent::Stop { reason }
        | vision_core::NextIntent::NeedHuman { reason }
        | vision_core::NextIntent::Rebind { reason } => reason.clone(),
    };
    let report = json!({
        "mode": "observe",
        "bound_title": native.title,
        "now": decision.now_line,
        "next": decision.next_line,
        "basis": basis,
        "page": result.snapshot.page.as_str(),
        "domain": format!("{:?}", decision.domain),
        "decision": decision,
        "refuse_reason": result.refuse_reason,
        "action_plan_type": "stop",
        "input_invoked": false,
        "input_adapter_called": false,
        "evidence_bmp": bmp.display().to_string(),
        "notes": notes,
        "ocr_notes": ocr_notes,
        "ocr_lines": ocr_lines,
        "candidate_count": result.candidate_set.items.len(),
    });
    fs::write(&meta, serde_json::to_string_pretty(&report).unwrap()).map_err(|e| e.to_string())?;

    println!("{}", decision.now_line);
    println!("{}", decision.next_line);
    println!("依据：{basis}");
    println!(
        "不确定：{}",
        decision.refuse_reason.as_deref().unwrap_or("无")
    );
    if !ocr_lines.is_empty() {
        println!(
            "OCR样例：{}",
            ocr_lines
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }
    println!("证据图：{}", bmp.display());
    println!("报告：{}", meta.display());
    Ok(())
}

fn cmd_live(index: usize, frames: u32) -> Result<(), String> {
    let root = data_root();
    if active_pack_dir(&root).is_none() {
        return Err(format!(
            "PACK_NOT_INSTALLED: install recognition pack under {} before live shadow (models not in main package)",
            root.display()
        ));
    }
    let min_conf = load_page_rules_min_confidence(&root);

    let mut plat = WindowsPlatform::new();
    plat.pack_ready = true;
    plat.input_enabled = false;
    plat.capture_preference = CapturePreference::WgcOnly;

    let cands = plat
        .discover_surfaces("wechat")
        .map_err(|e| e.to_string())?;
    if cands.is_empty() {
        return Err("no wechat candidates".into());
    }
    let cand = cands
        .get(index)
        .ok_or_else(|| format!("index {index} out of range (n={})", cands.len()))?;
    let lease = plat
        .bind_surface(&cand.candidate_id)
        .map_err(|e| e.to_string())?;

    let found = discover_wechat_candidates().map_err(|e| e.to_string())?;
    let native = found
        .into_iter()
        .find(|n| n.to_candidate().candidate_id == cand.candidate_id)
        .ok_or_else(|| "native surface missing after bind".to_string())?;

    let (mut session, mut sess_notes) = WgcCaptureSession::open(&native)?;
    let out = shadow_out_dir();
    let mut reports = Vec::new();

    for i in 0..frames {
        let frame_t0 = Instant::now();
        let (frame, mut notes) = session.grab_frame(&lease)?;
        let frame_ms = frame_t0.elapsed().as_millis() as u64;
        notes.append(&mut sess_notes);
        sess_notes.clear();

        if !frame.is_usable() {
            return Err(format!(
                "CONTENT_UNUSABLE on frame {i}: status={:?}",
                frame.status
            ));
        }
        let px = frame
            .pixels
            .as_ref()
            .ok_or_else(|| "no pixels".to_string())?;

        let ocr_t0 = Instant::now();
        let (ocr, ocr_notes) =
            ocr_bgra_windows_media(frame.pixel_width, frame.pixel_height, &px.bytes)?;
        let ocr_ms = ocr_t0.elapsed().as_millis() as u64;
        notes.extend(ocr_notes);

        let mut templates = bundled_shell_templates();
        templates.extend(
            active_pack_dir(&root)
                .map(|p| load_shell_templates(&p.join("templates").join("wechat_shell")))
                .unwrap_or_default(),
        );
        let result = observe_shadow(ShadowObserveInput {
            run_id: "shadow-live",
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

        let mut evidence_pixels = px.bytes.clone();
        draw_boxes_bgra(
            frame.pixel_width,
            frame.pixel_height,
            &mut evidence_pixels,
            &result.candidate_boxes,
        );
        let stamp = common::chrono_stamp();
        let bmp = out.join(format!("live_f{i}_{stamp}.bmp"));
        write_bgra_bmp(
            &bmp,
            frame.pixel_width,
            frame.pixel_height,
            &evidence_pixels,
        )?;
        let meta = out.join(format!("live_f{i}_{stamp}.json"));
        let report = json!({
            "mode": "live",
            "frame_index": i,
            "page": result.snapshot.page.as_str(),
            "confidence": result.snapshot.confidence,
            "evidence": result.snapshot.evidence,
            "overlay": format!("{:?}", result.snapshot.overlay),
            "refuse_reason": result.refuse_reason,
            "action_plan": result.action_plan,
            "slots": result.slots,
            "candidate_boxes": result.candidate_boxes,
            "timing": result.timing,
            "wgc_device_creates": session.device_creates(),
            "wgc_frames_grabbed": session.frames_grabbed(),
            "notes": notes,
            "evidence_bmp": bmp.display().to_string(),
            "input_invoked": false,
            "input_adapter_called": false,
            "pack_required": true,
            "ocr_backend": "windows_media_system_not_bundled",
        });
        fs::write(&meta, serde_json::to_string_pretty(&report).unwrap())
            .map_err(|e| e.to_string())?;
        println!(
            "frame {i}: page={} refuse={:?} device_creates={} bmp={}",
            result.snapshot.page.as_str(),
            result.refuse_reason,
            session.device_creates(),
            bmp.display()
        );
        reports.push(meta.display().to_string());
    }

    if session.device_creates() != 1 {
        return Err(format!(
            "expected single D3D11 device create, got {}",
            session.device_creates()
        ));
    }
    println!("reports: {reports:?}");
    Ok(())
}

/// Legacy single-fixture path kept for shadow testdata shape without page_label.
fn cmd_replay_legacy_shadow(fixture: &str) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct AnnotatedFixture {
        id: String,
        page_label: Option<String>,
        frame: vision_core::FrameSize,
        ocr_hits: Vec<Hit>,
    }
    #[derive(serde::Deserialize)]
    struct Hit {
        text: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        score: f32,
    }

    let path = fixture_path(fixture);
    let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if serde_json::from_str::<vision_core::PageMapSample>(&raw).is_ok() {
        return eval_cmd::cmd_replay(fixture);
    }
    let f: AnnotatedFixture = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let hits: Vec<OcrHit> = f
        .ocr_hits
        .into_iter()
        .map(|h| OcrHit {
            text: h.text,
            x: h.x,
            y: h.y,
            w: h.w,
            h: h.h,
            score: h.score,
        })
        .collect();
    let ocr = ocr_from_annotated_hits(hits);
    let result = observe_shadow(ShadowObserveInput {
        run_id: "shadow-replay",
        lease_id: "lease-replay",
        generation: 1,
        frame_id: &f.id,
        frame_w: f.frame.width,
        frame_h: f.frame.height,
        ocr: &ocr,
        title_hint: f.page_label.clone().unwrap_or_default(),
        min_confidence: DEFAULT_MIN_CONFIDENCE,
        target_slot_time: None,
        timing: ShadowTiming::default(),
        perception: PerceptionFeed::default(),
    });
    assert!(!result.input_invoked);
    let out = shadow_out_dir();
    let stamp = common::chrono_stamp();
    let meta = out.join(format!("replay_{}_{stamp}.json", fixture.replace('.', "_")));
    let report = json!({
        "mode": "replay",
        "fixture": fixture,
        "fixture_id": f.id,
        "ocr_source": "annotated",
        "page": result.snapshot.page.as_str(),
        "confidence": result.snapshot.confidence,
        "evidence": result.snapshot.evidence,
        "overlay": format!("{:?}", result.snapshot.overlay),
        "refuse_reason": result.refuse_reason,
        "action_plan": result.action_plan,
        "slots": result.slots,
        "candidate_boxes": result.candidate_boxes,
        "timing": result.timing,
        "input_invoked": false,
        "input_adapter_called": false,
        "note": "Annotated OCR replay — not E2E real-screenshot accuracy.",
    });
    fs::write(&meta, serde_json::to_string_pretty(&report).unwrap()).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    println!("wrote {}", meta.display());
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        std::process::exit(2);
    }
    let result = match args[0].as_str() {
        "session" => cmd_session(&args[1..]),
        "observe" => cmd_observe(&args[1..]),
        "replay" => {
            let fixture = common::parse_flag(&args[1..], "--fixture")
                .unwrap_or_else(|| "select_slot_normal".to_string());
            cmd_replay_legacy_shadow(&fixture)
        }
        "live" => {
            let index = common::parse_flag(&args[1..], "--index")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let frames = common::parse_flag(&args[1..], "--frames")
                .and_then(|s| s.parse().ok())
                .unwrap_or(2u32)
                .max(1);
            cmd_live(index, frames)
        }
        "capture" => capture::cmd_capture(&args[1..]),
        "confirm" => confirm::cmd_confirm(&args[1..]),
        "eval" => eval_cmd::cmd_eval(&args[1..]),
        "import" => import::cmd_import(&args[1..]),
        "confirm-help" => {
            let dir = common::parse_flag(&args[1..], "--dir")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(common::page_map_root);
            confirm::cmd_confirm_hint(&dir)
        }
        "promote" => corpus_cmd::cmd_promote(&args[1..]),
        "corpus" => corpus_cmd::cmd_corpus(&args[1..]),
        _ => {
            usage();
            Err("unknown subcommand".into())
        }
    };
    if let Err(e) = result {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}

/// Read-only persistent-session probe. Useful for static pages and DPI changes;
/// deliberately never executes returned tokens or prints captcha/user OCR.
#[cfg(windows)]
fn cmd_session(args: &[String]) -> Result<(), String> {
    let frames = common::parse_flag(args, "--frames")
        .and_then(|s| s.parse::<u32>().ok()).unwrap_or(6).clamp(1, 120);
    let gap = common::parse_flag(args, "--gap-ms")
        .and_then(|s| s.parse::<u64>().ok()).unwrap_or(500).min(15_000);
    let goal = TaskGoal {
        sport: common::parse_flag(args, "--sport"),
        venue: common::parse_flag(args, "--venue"),
        date: common::parse_flag(args, "--date"),
        slot_time: common::parse_flag(args, "--time"),
        ..TaskGoal::default()
    };
    let mut session = vision_platform_windows::RunSession::open(&data_root(), "diag-session")?;
    println!("{}", vision_platform_windows::device_diagnostics());
    for index in 0..frames {
        if index > 0 { std::thread::sleep(std::time::Duration::from_millis(gap)); }
        let observed = session.observe(&goal, vision_platform_windows::GlanceMode::run())?;
        println!("{}", json!({"index": index, "page": observed.report.page,
            "reason": observed.report.refuse_reason, "can_click": observed.report.can_click,
            "frame_id": observed.source_frame_id, "bind_count": observed.bind_count,
            "wgc_open_count": observed.wgc_open_count,
            "size": [observed.report.frame_width, observed.report.frame_height],
            "surface": observed.report.surface_diagnostics, "input_invoked": false}));
    }
    Ok(())
}

#[cfg(not(windows))]
fn cmd_session(_args: &[String]) -> Result<(), String> {
    Err("session capture is Windows-only".into())
}

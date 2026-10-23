//! Windows single-frame diagnostics — **no input injection**.
//!
//! Usage:
//!   cargo run -p vision-platform-windows --bin vision-frame-diag -- list
//!   cargo run -p vision-platform-windows --bin vision-frame-diag -- capture --index 0 --backend gdi
//!   cargo run -p vision-platform-windows --bin vision-frame-diag -- scenario --index 0 --name move
//!
//! Outputs under `%LOCALAPPDATA%/xmu-sports/diag/frames/` (never the git repo).
//! Open WeChat yourself first. Visually confirm the BMP matches the bound window.
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use vision_platform_api::PlatformPort;
use vision_platform_windows::{CapturePreference, WindowsPlatform};

mod diag;
use diag::{
    capture_one, default_diag_root, list_candidates, parse_backend, scenario_after, FrameDiagReport,
};
fn usage() {
    eprintln!(
        "vision-frame-diag — Windows HWND-bound capture diagnostics (NO INPUT)\n\n\
         Commands:\n\
           device (anonymous display topology and DPI)\n\
           list [--selector wechat|miniprogram]\n\
           capture --index N | --id <candidate_id> [--backend gdi|wgc|auto] [--out DIR]\n\
           scenario --index N --name <label> [--backend gdi|wgc|auto]\n\
             Captures before, waits for Enter (you move/resize/minimize/close/switch),\n\
             then records: validate(original_lease) + old-lease capture attempt +\n\
             explicit rebind capture — separately in after JSON.\n\n\
         Never sets VISION_ALLOW_LIVE. Never calls inject_input."
    );
}
fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        std::process::exit(2);
    }
    let cmd = args.remove(0);
    let mut plat = WindowsPlatform::new();
    plat.pack_ready = true; // diagnostics don't require OCR pack
    plat.input_enabled = false;
    plat.capture_preference = CapturePreference::GdiOnly;
    match cmd.as_str() {
        "device" => {
            println!("{}", serde_json::to_string_pretty(&vision_platform_windows::device_diagnostics()).unwrap());
        }
        "list" => {
            let selector = flag_value(&args, "--selector").unwrap_or_else(|| "wechat".into());
            let _ = plat.discover_surfaces(&selector);
            match list_candidates(&mut plat) {
                Ok(list) if list.is_empty() => {
                    eprintln!("No WeChat / 厦大体育 windows found. Open and login WeChat first.");
                    std::process::exit(1);
                }
                Ok(list) => {
                    println!("{}", serde_json::to_string_pretty(&list).unwrap());
                }
                Err(e) => {
                    eprintln!("discover failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        "capture" => {
            if let Some(b) = flag_value(&args, "--backend") {
                plat.capture_preference = parse_backend(&b).unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(2);
                });
            }
            let out = flag_value(&args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(default_diag_root);
            let id = resolve_candidate_id(&mut plat, &args);
            match capture_one(&mut plat, &id, "capture", &out) {
                Ok(r) => print_report(&r),
                Err(e) => {
                    eprintln!("capture failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        "scenario" => {
            if let Some(b) = flag_value(&args, "--backend") {
                plat.capture_preference = parse_backend(&b).unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(2);
                });
            }
            let name = flag_value(&args, "--name").unwrap_or_else(|| "manual".into());
            let out = flag_value(&args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(default_diag_root);
            let id = resolve_candidate_id(&mut plat, &args);
            // Baseline — keep the exact lease object for post-change validation.
            let base =
                capture_one(&mut plat, &id, &format!("{name}_before"), &out).unwrap_or_else(|e| {
                    eprintln!("baseline capture failed: {e}");
                    std::process::exit(1);
                });
            print_report(&base);
            let original_lease = plat.bound_lease().cloned().unwrap_or_else(|| {
                eprintln!("no bound lease after baseline");
                std::process::exit(1);
            });
            eprintln!(
                "\n>>> Now perform the scenario on the BOUND window: {name}\n\
                 (move / resize / minimize / close / switch). Then press Enter…\n\
                 After Enter: validate(original) + capture(original) are recorded\n\
                 separately from an explicit rebind capture."
            );
            let _ = io::stdout().flush();
            let mut line = String::new();
            let _ = io::stdin().read_line(&mut line);
            match scenario_after(&mut plat, &id, &original_lease, &name, &out) {
                Ok(r) => {
                    println!("{}", serde_json::to_string_pretty(&r).unwrap());
                    eprintln!(
                        "evidence_ok_hint={} meta={} input_injected={}",
                        r.evidence_ok_hint, r.meta_path, r.input_injected
                    );
                    eprintln!(
                        "validate_after={}/{:?} gen={} bound_unchanged={}/{}",
                        r.validate_after.outcome,
                        r.validate_after.reason,
                        r.validate_after.original_generation,
                        r.validate_after.bound_lease_id_unchanged,
                        r.validate_after.bound_generation_unchanged
                    );
                    eprintln!(
                        "old_lease_capture ok={} reason={:?}",
                        r.old_lease_capture.ok, r.old_lease_capture.reason_code
                    );
                    if let Some(ref rb) = r.rebind_capture {
                        eprintln!(
                            "rebind_capture ok bmp={:?} gen={:?}",
                            rb.bmp_path,
                            rb.lease.as_ref().and_then(|l| l.get("generation").cloned())
                        );
                    } else {
                        eprintln!("rebind_capture failed: {:?}", r.rebind_error);
                    }
                }
                Err(e) => {
                    eprintln!("scenario after failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        "help" | "-h" | "--help" => usage(),
        other => {
            eprintln!("unknown command: {other}");
            usage();
            std::process::exit(2);
        }
    }
}
fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}
fn resolve_candidate_id(plat: &mut WindowsPlatform, args: &[String]) -> String {
    let list = list_candidates(plat).unwrap_or_else(|e| {
        eprintln!("discover failed: {e}");
        std::process::exit(1);
    });
    if list.is_empty() {
        eprintln!("No candidates. Open WeChat first.");
        std::process::exit(1);
    }
    if let Some(id) = flag_value(args, "--id") {
        return id;
    }
    let idx: usize = flag_value(args, "--index")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if idx >= list.len() {
        eprintln!("index {idx} out of range 0..{}", list.len() - 1);
        std::process::exit(2);
    }
    eprintln!(
        "Using candidate[{idx}] id={} title={:?} pid={}",
        list[idx].candidate_id, list[idx].title_hint, list[idx].pid
    );
    if list.len() > 1 {
        eprintln!(
            "NOTE: {} candidates share the list (same PID possible). Confirm title/bounds match the window you intend.",
            list.len()
        );
    }
    list[idx].candidate_id.clone()
}
fn print_report(r: &FrameDiagReport) {
    println!("{}", serde_json::to_string_pretty(r).unwrap());
    eprintln!(
        "input_injected={} bmp={:?} meta={}",
        r.input_injected, r.bmp_path, r.meta_path
    );
}

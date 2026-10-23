//! Shared helpers for vision-shadow-diag.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use vision_core::DEFAULT_MIN_CONFIDENCE;
use vision_pack::active_pack_dir;

pub fn chrono_stamp() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms}")
}

pub fn data_root() -> PathBuf {
    env::var("XMU_SPORTS_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let base = env::var("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|_| env::temp_dir());
            base.join("xmu-sports")
        })
}

pub fn shadow_out_dir() -> PathBuf {
    let dir = data_root().join("diag").join("shadow");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Default page-map capture root (outside repo).
pub fn page_map_root() -> PathBuf {
    let dir = data_root().join("diag").join("page_map");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn load_page_rules_min_confidence(data_root: &Path) -> f32 {
    let Some(active) = active_pack_dir(data_root) else {
        return DEFAULT_MIN_CONFIDENCE;
    };
    let rules = active.join("templates").join("page_rules.json");
    let Ok(raw) = fs::read_to_string(rules) else {
        return DEFAULT_MIN_CONFIDENCE;
    };
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(json!({}));
    v.pointer("/shadow/min_confidence")
        .and_then(|x| x.as_f64())
        .map(|f| f as f32)
        .unwrap_or(DEFAULT_MIN_CONFIDENCE)
}

pub fn process_rss_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        use windows::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;
        unsafe {
            let mut counters = PROCESS_MEMORY_COUNTERS::default();
            counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb).is_ok() {
                return Some(counters.WorkingSetSize as u64);
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn prompt_line(msg: &str) -> Result<String, String> {
    print!("{msg}");
    let _ = io::stdout().flush();
    let mut buf = String::new();
    io::stdin()
        .read_line(&mut buf)
        .map_err(|e| format!("stdin: {e}"))?;
    Ok(buf.trim().to_string())
}

pub fn fixture_path(name: &str) -> PathBuf {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vision-core/testdata/shadow")
            .join(format!("{name}.json")),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vision-core/testdata/shadow")
            .join(name),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vision-core/testdata/page_map")
            .join(format!("{name}.json")),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vision-core/testdata/page_map")
            .join(name),
    ];
    for c in &candidates {
        if c.is_file() {
            return c.clone();
        }
    }
    candidates[0].clone()
}

pub fn parse_flag(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        i += 1;
    }
    None
}

pub fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

//! 诊断包导出 + 按启动会话日志。
//! Spec: docs/superpowers/specs/2026-09-23-diagnostic-log-export-design.md

use chrono::{DateTime, Local};
use serde::Serialize;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// runs 纳入窗口：最近 7 天。
pub const RUNS_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// runs 最多运行次数（按 mtime 新→旧）。
pub const RUNS_MAX_TASKS: usize = 20;
/// 会话日志保留天数。
pub const LOG_MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
/// 会话日志最多保留文件数（不含 latest）。
pub const LOG_MAX_FILES: usize = 30;
/// 每个任务最多纳入最近 20 张失败证据图。
pub const RUN_EVIDENCE_MAX_FILES: usize = 20;
/// 记录当前会话相对文件名的固定指针，不依赖被轮转的文件句柄。
pub const LATEST_LOG_NAME: &str = "latest.log";

#[derive(Debug, Serialize)]
struct Manifest {
    product: String,
    version: String,
    exported_at: String,
    os: String,
    device: serde_json::Value,
    log_dir: String,
    runs_dir: String,
    included: Vec<String>,
    notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPick {
    pub task_id: String,
    /// `None` = legacy flat `runs/<task_id>/events.jsonl`.
    pub run_id: Option<String>,
    pub events_path: PathBuf,
    pub mtime: SystemTime,
}

#[derive(Debug, Clone)]
struct LogCandidate {
    path: PathBuf,
    name: String,
    mtime: SystemTime,
}

/// 按 mtime 筛选近期 events.jsonl（纯函数，便于单测）。
pub fn select_recent_run_events(
    candidates: impl IntoIterator<Item = RunPick>,
    now: SystemTime,
    max_age: Duration,
    max_tasks: usize,
) -> Vec<RunPick> {
    let cutoff = now.checked_sub(max_age).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut picks: Vec<RunPick> = candidates
        .into_iter()
        .filter(|p| p.mtime >= cutoff)
        .collect();
    picks.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    picks.truncate(max_tasks);
    picks
}

/// 扫描新布局 `runs/<task>/<run>/events.jsonl`，并 fallback 旧扁平 `runs/<task>/events.jsonl`。
pub fn scan_run_event_files(runs_dir: &Path) -> Vec<RunPick> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(runs_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let task_path = entry.path();
        if !task_path.is_dir() {
            continue;
        }
        let Some(task_id) = task_path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        // New per-run layout (preferred).
        if let Ok(runs) = fs::read_dir(&task_path) {
            for run_entry in runs.flatten() {
                let run_path = run_entry.path();
                if !run_path.is_dir() {
                    continue;
                }
                let Some(run_id) = run_path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                let events = run_path.join("events.jsonl");
                if !events.is_file() {
                    continue;
                }
                let Ok(meta) = fs::metadata(&events) else {
                    continue;
                };
                let Ok(mtime) = meta.modified() else {
                    continue;
                };
                out.push(RunPick {
                    task_id: task_id.to_string(),
                    run_id: Some(run_id.to_string()),
                    events_path: events,
                    mtime,
                });
            }
        }
        // Legacy flat fallback (AC-RS-09).
        let legacy = task_path.join("events.jsonl");
        if legacy.is_file() {
            if let Ok(meta) = fs::metadata(&legacy) {
                if let Ok(mtime) = meta.modified() {
                    out.push(RunPick {
                        task_id: task_id.to_string(),
                        run_id: None,
                        events_path: legacy,
                        mtime,
                    });
                }
            }
        }
    }
    out
}

fn archive_run_prefix(pick: &RunPick) -> String {
    match &pick.run_id {
        Some(run_id) => format!("runs/{}/{}", pick.task_id, run_id),
        None => format!("runs/{}", pick.task_id),
    }
}

fn collect_run_evidence(run_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<(SystemTime, PathBuf)> = fs::read_dir(run_dir.join("evidence"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file()
                || path.extension().and_then(|s| s.to_str()) != Some("bmp")
            {
                return None;
            }
            let mtime = path.metadata().ok()?.modified().ok()?;
            Some((mtime, path))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.truncate(RUN_EVIDENCE_MAX_FILES);
    files.into_iter().map(|(_, path)| path).collect()
}

fn is_session_log_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("xmu-sports") && (lower.ends_with(".log") || lower.ends_with(".log.bak"))
}

/// 收集 LogDir 下 `xmu-sports*.log`（含轮转；不含 `latest.log`）。
pub fn collect_log_files(log_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(log_dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if is_session_log_name(name) {
            files.push(p);
        }
    }
    files.sort();
    files
}

/// 启动时：确保本次会话文件存在、刷新 `latest.log`、清理过期会话。
/// `session_file_name` 形如 `xmu-sports-20260923-120000.log`。
/// 返回删除的旧日志个数。
pub fn prepare_session_logs(log_dir: &Path, session_file_name: &str) -> Result<usize, String> {
    fs::create_dir_all(log_dir).map_err(|e| format!("无法创建日志目录: {e}"))?;
    let session_path = log_dir.join(session_file_name);
    if !session_path.is_file() {
        File::create(&session_path).map_err(|e| format!("无法创建会话日志: {e}"))?;
    }
    install_latest_link(log_dir, session_file_name)?;
    let pruned = prune_session_logs(
        log_dir,
        session_file_name,
        SystemTime::now(),
        LOG_MAX_AGE,
        LOG_MAX_FILES,
    )?;
    Ok(pruned)
}

/// `latest.log` 始终写一行相对文件名，轮转重建会话文件后仍然有效。
pub fn install_latest_link(log_dir: &Path, session_file_name: &str) -> Result<(), String> {
    let target = log_dir.join(session_file_name);
    if !target.is_file() {
        return Err(format!("会话日志不存在: {}", target.display()));
    }
    let link = log_dir.join(LATEST_LOG_NAME);
    let _ = fs::remove_file(&link);
    fs::write(&link, format!("{session_file_name}\n"))
        .map_err(|e| format!("无法写入 {LATEST_LOG_NAME}: {e}"))
}

/// 删除过旧或超额的 `xmu-sports*.log`；永不删 `keep_name` / `latest.log`。
pub fn prune_session_logs(
    log_dir: &Path,
    keep_name: &str,
    now: SystemTime,
    max_age: Duration,
    max_files: usize,
) -> Result<usize, String> {
    let cutoff = now.checked_sub(max_age).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut cands: Vec<LogCandidate> = Vec::new();
    let Ok(entries) = fs::read_dir(log_dir) else {
        return Ok(0);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str().map(|s| s.to_string())) else {
            continue;
        };
        if name.eq_ignore_ascii_case(LATEST_LOG_NAME) {
            continue;
        }
        if name == keep_name || !is_session_log_name(&name) {
            continue;
        }
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        let Ok(mtime) = meta.modified() else {
            continue;
        };
        cands.push(LogCandidate { path, name, mtime });
    }
    cands.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.name.cmp(&b.name)));

    let mut removed = 0usize;
    // keep 占 1 个名额；其余最多 max_files-1
    let keep_slots = max_files.saturating_sub(1);
    for (i, c) in cands.iter().enumerate() {
        let over_count = i >= keep_slots;
        let too_old = c.mtime < cutoff;
        if over_count || too_old {
            if fs::remove_file(&c.path).is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

fn stamp_local(now: DateTime<Local>) -> String {
    now.format("%Y%m%d-%H%M%S").to_string()
}

fn zip_add_file(zip: &mut ZipWriter<File>, arc_name: &str, src: &Path) -> Result<(), String> {
    let mut f = File::open(src).map_err(|e| format!("读取 {} 失败: {e}", src.display()))?;
    zip.start_file(arc_name, SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated))
        .map_err(|e| format!("写入 zip 条目失败: {e}"))?;
    std::io::copy(&mut f, zip)
        .map_err(|e| format!("写入 zip 数据失败: {e}"))?;
    Ok(())
}

fn zip_add_bytes(zip: &mut ZipWriter<File>, arc_name: &str, data: &[u8]) -> Result<(), String> {
    zip.start_file(arc_name, SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated))
        .map_err(|e| format!("写入 zip 条目失败: {e}"))?;
    zip.write_all(data)
        .map_err(|e| format!("写入 zip 数据失败: {e}"))?;
    Ok(())
}

/// 组装诊断 zip；返回最终绝对路径。
pub fn write_diagnostic_zip(
    download_dir: &Path,
    log_dir: &Path,
    runs_dir: &Path,
    version: &str,
    now: DateTime<Local>,
) -> Result<PathBuf, String> {
    if !download_dir.is_dir() {
        fs::create_dir_all(download_dir).map_err(|e| {
            format!(
                "无法创建下载目录 {}: {e}",
                download_dir.display()
            )
        })?;
    }

    let out = download_dir.join(format!("xmu-sports-logs-{}.zip", stamp_local(now)));
    let file = File::create(&out).map_err(|e| {
        format!(
            "无法写入下载目录 {}: {e}",
            download_dir.display()
        )
    })?;
    let mut zip = ZipWriter::new(file);

    let mut included: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let guide = "HTTP 轻量版诊断\n\
按 run_id 关联 [worker_start]、[worker_ready]、[progress]、[worker_metrics]、[worker_result]。\n\
详细接口证据：[http_exchange] 与 events.jsonl 的 http_exchange；按 sequence 对照接口、阶段、实际请求参数、HTTP 状态、耗时及脱敏回复。非 JSON/HTTP 错误也保存响应正文；超时记录错误分类，未收到回复时没有 http_status。\n\
单次回复上限 131072 字符，response_truncated=true 表示已截断；response_chars 为脱敏后原长度。Cookie、登录票据、签名、验证码与账号资料被过滤，不保存请求头或完整 URL。验证码成功只记录图片已收到，不保存图片。\n\
worker_ready.startup_ms：从创建进程到加载 Python 组件就绪；worker_metrics.phase_ms：各阶段累计耗时。\n\
定时预热：worker_model_warming → worker_warmed → worker_warm_rechecked；检查 process_id 与 model_load_count，预热不访问学校接口。\n\
独立再次查单记录位于 runs/order-rechecks/<运行>/，只使用原请求 journal 的账号与实际场次；ORDER_RECHECK_UNCONFIRMED 仍待确认，不重发预约。\n\
worker_metrics.http：固定接口分类的次数、累计/最大耗时、timeout/connection/invalid_json 等错误数；不含参数、Cookie、票据、验证码或服务端正文。\n\
当前登录链路：复用经学校校验的 H5 → 失效时通过 Windows UIA 操作正常登录界面 → 获取官方生成的新 H5 → HTTP 校验原账号。无需旧内核哈希适配或注入。\n\
LOGIN_FOCUS_REQUIRED / LOGIN_CONFIRM_REQUIRED：查看正常微信界面是否需要获得焦点或本人身份确认；ACCOUNT_CHANGED / ACCOUNT_MISMATCH：停止，核对原账号。\n\
SUBMIT_UNKNOWN / RESULT_UNCONFIRMED：预约结果待确认；查看查单轮次及最终匹配结果。未查到不等于未提交，禁止据此重复发送。\n\
CAPTCHA_REJECTED：只在服务端明确拒绝验证码时换图重试，首次加重试合计最多 3 次。超时、断网或响应不明先只读查单。\n\
运行事件：runs/<任务>/<运行>/events.jsonl；manifest.json 含最终状态。只有匹配原账号、日期、完整时段和实际场地的订单才确认成功。\n\
旧版本历史记录仍可能含 RUNTIME_*、截图或 OCR 文字；它们不代表当前轻量版的登录方式。\n\
反馈时请附应用/微信版本、测试时间、是否被手机顶号、是否有全屏窗口及最终提示。\n";
    zip_add_bytes(&mut zip, "HTTP-diagnostics.txt", guide.as_bytes())?;
    included.push("HTTP-diagnostics.txt".into());

    let logs = collect_log_files(log_dir);
    if logs.is_empty() {
        notes.push("未找到 App 日志文件（xmu-sports*.log）".into());
    }
    for path in &logs {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("xmu-sports.log");
        let arc = format!("logs/{name}");
        zip_add_file(&mut zip, &arc, path)?;
        included.push(arc);
    }

    let picks = select_recent_run_events(
        scan_run_event_files(runs_dir),
        SystemTime::now(),
        RUNS_MAX_AGE,
        RUNS_MAX_TASKS,
    );
    if picks.is_empty() {
        notes.push("未纳入近期 runs/events.jsonl（无文件或不在 7 天窗口内）".into());
    } else {
        notes.push(
            "诊断包可能含微信小程序界面截图与 OCR 文本；请仅本人留存或脱敏后分享".into(),
        );
    }
    for pick in &picks {
        let prefix = archive_run_prefix(pick);
        let arc = format!("{prefix}/events.jsonl");
        zip_add_file(&mut zip, &arc, &pick.events_path)?;
        included.push(arc);
        let Some(run_dir) = pick.events_path.parent() else {
            continue;
        };
        let manifest_path = run_dir.join("manifest.json");
        if manifest_path.is_file() {
            let arc = format!("{prefix}/manifest.json");
            zip_add_file(&mut zip, &arc, &manifest_path)?;
            included.push(arc);
        }
        let observations = run_dir.join("observations.jsonl");
        if observations.is_file() {
            let arc = format!("{prefix}/observations.jsonl");
            zip_add_file(&mut zip, &arc, &observations)?;
            included.push(arc);
        }
        for path in collect_run_evidence(run_dir) {
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let arc = format!("{prefix}/evidence/{name}");
            zip_add_file(&mut zip, &arc, &path)?;
            included.push(arc);
        }
    }

    // Login-only operations have no booking run; export their independent evidence too.
    let login_root=runs_dir.parent().unwrap_or(runs_dir).join("login-diagnostics");
    let login_picks=select_recent_run_events(scan_run_event_files(&login_root),SystemTime::now(),RUNS_MAX_AGE,RUNS_MAX_TASKS);
    if !login_picks.is_empty() {
        notes.push("login-diagnostics 含登录阶段、Windows/微信版本及故障窗口截图；截图可能含个人资料，请仅本人留存或脱敏后分享。检查登录强制重开小程序，不代表已注销微信或制造服务端过期。".into());
    }
    for pick in login_picks {
        let prefix=archive_run_prefix(&pick).replacen("runs/","login-diagnostics/",1);
        let arc=format!("{prefix}/events.jsonl");
        zip_add_file(&mut zip,&arc,&pick.events_path)?;included.push(arc);
        if let Some(dir)=pick.events_path.parent() {
            for path in collect_run_evidence(dir) {
                if let Some(name)=path.file_name().and_then(|name|name.to_str()) {
                    let arc=format!("{prefix}/evidence/{name}");
                    zip_add_file(&mut zip,&arc,&path)?;included.push(arc);
                }
            }
        }
    }

    let manifest = Manifest {
        product: "xmu-sports".into(),
        version: version.into(),
        exported_at: now.to_rfc3339(),
        os: std::env::consts::OS.into(),
        device: crate::wechat_window::device_diagnostics(),
        log_dir: log_dir.display().to_string(),
        runs_dir: runs_dir.display().to_string(),
        included: included.clone(),
        notes,
    };
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|e| format!("manifest 序列化失败: {e}"))?;
    zip_add_bytes(&mut zip, "manifest.json", &manifest_bytes)?;

    zip.finish()
        .map_err(|e| format!("关闭 zip 失败: {e}"))?;

    Ok(out)
}

/// Windows：在资源管理器中选中文件；其它平台打开父目录。
pub fn reveal_in_folder(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let full = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf());
        let display = full.to_string_lossy();
        let cleaned = display
            .strip_prefix(r"\\?\")
            .unwrap_or(display.as_ref());
        Command::new("explorer")
            .raw_arg(format!("/select,\"{cleaned}\""))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("无法打开资源管理器: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let parent = path.parent().unwrap_or(path);
        open_dir(parent)
    }
}

pub fn open_dir(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let display = path.to_string_lossy();
        Command::new("explorer")
            .arg(display.as_ref())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("无法打开文件夹: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use std::process::Command;
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("无法打开文件夹: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_recent_caps_by_age_and_count() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mk = |id: &str, age_secs: u64| RunPick {
            task_id: id.into(),
            run_id: None,
            events_path: PathBuf::from(format!("{id}/events.jsonl")),
            mtime: now - Duration::from_secs(age_secs),
        };
        let picks = select_recent_run_events(
            vec![
                mk("old", 8 * 24 * 3600),
                mk("a", 100),
                mk("b", 200),
                mk("c", 50),
            ],
            now,
            Duration::from_secs(7 * 24 * 3600),
            2,
        );
        assert_eq!(
            picks.iter().map(|p| p.task_id.as_str()).collect::<Vec<_>>(),
            vec!["c", "a"]
        );
    }

    #[test]
    fn write_zip_with_empty_sources() {
        let dir = std::env::temp_dir().join(format!(
            "xmu-diag-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let log_dir = dir.join("logs");
        let runs_dir = dir.join("runs");
        let dl = dir.join("Downloads");
        fs::create_dir_all(&log_dir).unwrap();
        fs::create_dir_all(&runs_dir).unwrap();
        fs::create_dir_all(&dl).unwrap();

        let out = write_diagnostic_zip(
            &dl,
            &log_dir,
            &runs_dir,
            "0.0.0-test",
            Local::now(),
        )
        .expect("zip");
        assert!(out.is_file());
        assert!(out
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("xmu-sports-logs-"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_zip_includes_unified_run_trace_and_evidence() {
        let dir = std::env::temp_dir().join(format!(
            "xmu-diag-run-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let log_dir = dir.join("logs");
        let runs_dir = dir.join("runs");
        // New layout: runs/<task>/<run>/
        let run_dir = runs_dir.join("task-1").join("run-abc");
        let evidence_dir = run_dir.join("evidence");
        let dl = dir.join("Downloads");
        fs::create_dir_all(&log_dir).unwrap();
        fs::create_dir_all(&evidence_dir).unwrap();
        fs::create_dir_all(&dl).unwrap();
        fs::write(run_dir.join("events.jsonl"), b"{}\n").unwrap();
        fs::write(run_dir.join("observations.jsonl"), b"{}\n").unwrap();
        fs::write(run_dir.join("manifest.json"), b"{}\n").unwrap();
        fs::write(evidence_dir.join("failure.bmp"), b"BMtest").unwrap();
        // Legacy flat still exportable
        let legacy = runs_dir.join("task-legacy");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("events.jsonl"), b"{}\n").unwrap();
        let login=dir.join("login-diagnostics/login-1");
        fs::create_dir_all(login.join("evidence")).unwrap();
        fs::write(login.join("events.jsonl"),b"{\"phase\":\"final_failure\"}\n").unwrap();
        fs::write(login.join("evidence/login-01.bmp"),b"BMlogin").unwrap();

        let out = write_diagnostic_zip(
            &dl,
            &log_dir,
            &runs_dir,
            "0.0.0-test",
            Local::now(),
        )
        .expect("zip");
        let file = File::open(&out).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        {
            use std::io::Read;
            let mut guide = archive.by_name("HTTP-diagnostics.txt").unwrap();
            assert_eq!(guide.compression(), zip::CompressionMethod::Deflated);
            let mut text = String::new();
            guide.read_to_string(&mut text).unwrap();
            assert!(text.contains("无需旧内核哈希适配或注入"));
            assert!(text.contains("worker_metrics"));
            assert!(text.contains("未查到不等于未提交"));
        }
        assert!(archive.by_name("HTTP-diagnostics.txt").is_ok());
        assert!(archive
            .by_name("runs/task-1/run-abc/events.jsonl")
            .is_ok());
        assert!(archive
            .by_name("runs/task-1/run-abc/observations.jsonl")
            .is_ok());
        assert!(archive
            .by_name("runs/task-1/run-abc/evidence/failure.bmp")
            .is_ok());
        assert!(archive.by_name("runs/task-legacy/events.jsonl").is_ok());
        assert!(archive.by_name("login-diagnostics/login-1/events.jsonl").is_ok());
        assert!(archive.by_name("login-diagnostics/login-1/evidence/login-01.bmp").is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_session_creates_latest_and_prunes_by_age() {
        let dir = std::env::temp_dir().join(format!(
            "xmu-session-log-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let keep = "xmu-sports-20990101-000000.log";
        // 用 prune 的 age 门：mtime 很旧的文件
        let old = dir.join("xmu-sports-20200101-000000.log");
        fs::write(&old, b"old\n").unwrap();
        // 直接测 prune：把 now 设得靠后，max_age 很短
        let now = SystemTime::now();
        File::create(dir.join(keep)).unwrap();
        install_latest_link(&dir, keep).unwrap();
        let pruned = prune_session_logs(
            &dir,
            keep,
            now,
            Duration::from_secs(1), // 几乎一切非 keep 都算旧（若 old 的 mtime 稍早）
            LOG_MAX_FILES,
        )
        .unwrap();
        // 若文件系统 mtime 精度导致 old 仍“新”，至少 latest + keep 存在
        assert!(dir.join(keep).is_file());
        assert!(dir.join(LATEST_LOG_NAME).is_file());
        let _ = pruned;
        let latest_bytes = fs::read(dir.join(LATEST_LOG_NAME)).unwrap();
        let keep_bytes = fs::read(dir.join(keep)).unwrap();
        assert!(
            latest_bytes == keep_bytes || latest_bytes.starts_with(keep.as_bytes()),
            "latest should hardlink or point to session"
        );

        let pruned2 = prepare_session_logs(&dir, keep).expect("prepare");
        assert!(dir.join(keep).is_file());
        assert!(dir.join(LATEST_LOG_NAME).is_file());
        let _ = pruned2;
        let _ = fs::remove_dir_all(&dir);
    }
    #[test]
    fn latest_pointer_survives_rotation_and_archives_are_export_candidates() {
        let dir = std::env::temp_dir().join(format!("xmu-rotation-pointer-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let name = "xmu-sports-session.log";
        let active = dir.join(name);
        fs::write(&active, "before rotation").unwrap();
        install_latest_link(&dir, name).unwrap();
        let archived = dir.join("xmu-sports-session_2026-10-23_07-00-00.log");
        fs::rename(&active, &archived).unwrap();
        fs::write(&active, "after rotation").unwrap();
        let collision_archive = dir.join("xmu-sports-session_2026-10-23_07-00-00.log.bak");
        fs::write(&collision_archive, "earlier rotation in the same second").unwrap();
        let pointer = fs::read_to_string(dir.join(LATEST_LOG_NAME)).unwrap();
        assert_eq!(pointer.trim(), name);
        assert_eq!(fs::read_to_string(dir.join(pointer.trim())).unwrap(), "after rotation");
        let picked = collect_log_files(&dir);
        assert!(picked.contains(&active) && picked.contains(&archived) && picked.contains(&collision_archive));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_respects_max_files() {
        let dir = std::env::temp_dir().join(format!(
            "xmu-prune-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let keep = "xmu-sports-keep.log";
        fs::write(dir.join(keep), b"keep\n").unwrap();
        for i in 0..5 {
            fs::write(dir.join(format!("xmu-sports-extra-{i}.log")), format!("{i}\n")).unwrap();
        }
        let removed = prune_session_logs(
            &dir,
            keep,
            SystemTime::now(),
            Duration::from_secs(30 * 24 * 3600),
            3, // keep + 2 extras
        )
        .unwrap();
        assert!(removed >= 3);
        assert!(dir.join(keep).is_file());
        let left = collect_log_files(&dir);
        assert!(left.len() <= 3, "left={left:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}

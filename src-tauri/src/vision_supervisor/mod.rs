//! Host-side vision supervisor: pack gate + worker lifecycle over stdin/stdout.
//!
//! Product booking runs in-process (`run_native_loop` / RunSession).
//! `vision-worker` is **probe-only until a future Spec** — missing worker must not
//! block native trial/arm; it only affects probe APIs.

mod worker_session;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;

use serde::Serialize;
use vision_contracts::{CapabilityReport, ReasonCode};
use vision_pack::{
    active_pack_dir, install_from_staging, PackState, PackStatus,
};

pub use worker_session::WorkerSession;

#[derive(Debug, Clone, Serialize)]
pub struct NativeVisionStatus {
    pub pack_status: String,
    pub pack_version: Option<String>,
    pub worker_present: bool,
    pub can_trial: bool,
    pub can_arm_native: bool,
    pub reason_code: String,
    pub executor_impl_default: String,
}

fn default_data_root() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("xmu-sports")
}

pub fn data_root() -> PathBuf {
    std::env::var("XMU_SPORTS_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_data_root())
}

/// Path to the bundled vision-worker next to the app / in target dir during dev.
pub fn worker_binary_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("vision-worker.exe"));
            out.push(dir.join("vision-worker"));
        }
    }
    // Dev: cargo target
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    out.push(
        manifest
            .join("target")
            .join("debug")
            .join("vision-worker.exe"),
    );
    out.push(manifest.join("target").join("debug").join("vision-worker"));
    out.push(
        manifest
            .join("target")
            .join("release")
            .join("vision-worker.exe"),
    );
    out
}

pub fn resolve_worker_binary() -> Option<PathBuf> {
    worker_binary_candidates().into_iter().find(|p| p.is_file())
}

pub fn native_status() -> NativeVisionStatus {
    let root = data_root();
    let active = active_pack_dir(&root);
    let pack_version = active
        .as_ref()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()));
    let pack_ready = active.is_some();
    // Informational only — worker is probe-only; do not gate trial/arm on it.
    let worker_present = resolve_worker_binary().is_some();
    let (status, reason, can) = if !pack_ready {
        (
            PackStatus::NotInstalled,
            ReasonCode::PACK_NOT_INSTALLED,
            false,
        )
    } else {
        (PackStatus::Ready, "OK", true)
    };
    NativeVisionStatus {
        pack_status: format!("{status:?}"),
        pack_version,
        worker_present,
        can_trial: can,
        can_arm_native: can,
        reason_code: reason.into(),
        executor_impl_default: "native".into(),
    }
}

/// Probe worker hello/capabilities without enabling input.
pub fn probe_worker_capabilities() -> Result<CapabilityReport, String> {
    let status = native_status();
    if !status.worker_present {
        return Err("vision-worker binary not found (build workspace first)".into());
    }
    if status.pack_status != format!("{:?}", PackStatus::Ready) {
        return Err(format!(
            "pack not ready: {} ({})",
            status.pack_status, status.reason_code
        ));
    }
    let bin = resolve_worker_binary().ok_or("worker missing")?;
    let mut session = WorkerSession::spawn(&bin, &data_root()).map_err(|e| e.to_string())?;
    let report = session.check_capabilities().map_err(|e| e.to_string())?;
    session.shutdown();
    Ok(report)
}

/// Ensure booking commands do not start native worker when pack missing.
#[allow(dead_code)]
pub fn assert_pack_ready_for_native() -> Result<(), String> {
    let st = native_status();
    if st.can_trial {
        Ok(())
    } else {
        Err(format!(
            "识别组件未就绪: {} — 首次预约前须安装识别资源包",
            st.reason_code
        ))
    }
}

#[allow(dead_code)]
pub fn kill_native_child(child: &mut Option<Child>) {
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

fn host_target_triple() -> &'static str {
    if cfg!(target_os = "windows") {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else {
        "unknown"
    }
}

/// Install a local (signed placeholder) recognition pack from a staging directory.
/// Used for fake-pack acceptance until CDN distribution is wired.
pub fn install_local_pack(staging_dir: &Path) -> Result<PackState, String> {
    install_from_staging(&data_root(), staging_dir, host_target_triple(), 1)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Default, Clone)]
pub struct PackSeedReport {
    pub installed: bool,
    pub skipped: bool,
    pub version: Option<String>,
    pub staging: Option<PathBuf>,
    pub notes: Vec<String>,
}

/// Candidate roots that contain a ready-to-install `manifest.json` staging pack.
pub fn bundle_pack_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(p) = std::env::var("XMU_SPORTS_PACK_BUNDLE") {
        if !p.is_empty() {
            roots.push(PathBuf::from(p));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.join("vision-pack"));
            roots.push(dir.join("resources").join("vision-pack"));
        }
    }
    // Dev: src-tauri/resources/vision-pack
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    roots.push(crate_dir.join("resources").join("vision-pack"));
    roots
}

fn staging_looks_valid(root: &Path) -> bool {
    root.join("manifest.json").is_file() && root.join("health.ok").is_file()
}

pub fn find_bundle_pack_staging() -> Option<PathBuf> {
    bundle_pack_roots()
        .into_iter()
        .find(|r| staging_looks_valid(r))
}

fn read_pack_version(staging: &Path) -> Option<String> {
    let raw = fs::read_to_string(staging.join("manifest.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("pack_version")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

/// Seed recognition pack from installer / dev resources when missing or outdated.
pub fn seed_pack_from_staging(staging: Option<&Path>) -> PackSeedReport {
    let mut report = PackSeedReport::default();
    let Some(root) = staging
        .map(Path::to_path_buf)
        .or_else(find_bundle_pack_staging)
    else {
        report.notes.push("pack:seed=skip_no_bundle".into());
        return report;
    };
    report.staging = Some(root.clone());
    if !staging_looks_valid(&root) {
        report.notes.push("pack:seed=skip_invalid_staging".into());
        return report;
    }

    let bundled_ver = read_pack_version(&root);
    report.version = bundled_ver.clone();
    if let Some(ref ver) = bundled_ver {
        if let Some(active) = active_pack_dir(&data_root()) {
            if active.file_name().and_then(|s| s.to_str()) == Some(ver.as_str()) {
                report.skipped = true;
                report
                    .notes
                    .push(format!("pack:seed=noop_active={ver}"));
                return report;
            }
        }
    }

    match install_from_staging(&data_root(), &root, host_target_triple(), 1) {
        Ok(st) => {
            report.installed = true;
            report.version = st.active_version.clone();
            report.notes.push(format!(
                "pack:seed=installed version={}",
                st.active_version.unwrap_or_default()
            ));
        }
        Err(e) => {
            report
                .notes
                .push(format!("pack:seed_fail={e}"));
        }
    }
    report
}

/// Prefer Tauri `$RESOURCE` then fall back to exe-adjacent / dev tree.
pub fn seed_pack_from_resource_dir(resource_dir: &Path) -> PackSeedReport {
    let candidates = [
        resource_dir.join("vision-pack"),
        resource_dir.join("resources").join("vision-pack"),
        resource_dir.to_path_buf(),
    ];
    for c in candidates {
        if staging_looks_valid(&c) {
            return seed_pack_from_staging(Some(&c));
        }
    }
    seed_pack_from_staging(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `XMU_SPORTS_DATA` is process-global; serialize tests that mutate it.
    static DATA_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_data_root<T>(root: &Path, f: impl FnOnce() -> T) -> T {
        let _guard = DATA_ENV_LOCK.lock().unwrap();
        let prev = std::env::var_os("XMU_SPORTS_DATA");
        std::env::set_var("XMU_SPORTS_DATA", root);
        let out = f();
        match prev {
            Some(v) => std::env::set_var("XMU_SPORTS_DATA", v),
            None => std::env::remove_var("XMU_SPORTS_DATA"),
        }
        out
    }

    fn seed_pack(root: &Path, version: &str) {
        let ver_dir = pack_root_join(root, version);
        fs::create_dir_all(&ver_dir).unwrap();
        let active = root.join("vision-packs").join("active");
        fs::create_dir_all(active.parent().unwrap()).unwrap();
        fs::write(&active, version).unwrap();
    }

    fn pack_root_join(root: &Path, version: &str) -> PathBuf {
        root.join("vision-packs").join("versions").join(version)
    }

    #[test]
    fn native_status_defaults_to_native_executor() {
        let root = std::env::temp_dir().join(format!(
            "xmu-native-status-empty-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let st = with_data_root(&root, native_status);
        assert_eq!(st.executor_impl_default, "native");
        assert!(!st.can_trial);
        assert!(!st.can_arm_native);
        assert_eq!(st.reason_code, ReasonCode::PACK_NOT_INSTALLED);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn native_status_pack_ready_allows_arm_without_worker() {
        let root = std::env::temp_dir().join(format!(
            "xmu-native-status-pack-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        seed_pack(&root, "test-pack-1");
        let st = with_data_root(&root, native_status);
        assert_eq!(st.executor_impl_default, "native");
        assert_eq!(st.pack_status, format!("{:?}", PackStatus::Ready));
        assert!(st.can_trial);
        assert!(st.can_arm_native);
        assert_eq!(st.reason_code, "OK");
        // worker_present may be true in a full cargo workspace build — either is fine;
        // the point is missing worker must not flip can_* to false.
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn seed_pack_from_staging_installs_when_missing() {
        let root = std::env::temp_dir().join(format!(
            "xmu-pack-seed-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        // Use the committed installer staging under src-tauri/resources/vision-pack
        let staging = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("vision-pack");
        assert!(
            staging.join("manifest.json").is_file(),
            "run scripts/ensure_vision_pack_bundle.ps1 first"
        );

        let report = with_data_root(&root, || seed_pack_from_staging(Some(&staging)));
        assert!(
            report.installed,
            "notes={:?}",
            report.notes
        );
        assert!(active_pack_dir(&root).is_some());

        let again = with_data_root(&root, || seed_pack_from_staging(Some(&staging)));
        assert!(again.skipped, "notes={:?}", again.notes);
        assert!(!again.installed);

        let _ = fs::remove_dir_all(&root);
    }
}

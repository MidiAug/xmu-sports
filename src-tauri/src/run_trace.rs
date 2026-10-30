//! Async TraceEnvelope sink + per-run directory layout.
//! Spec: docs/superpowers/specs/2026-09-27-run-session-multiframe-trace-design.md
//! Layout: `runs/<task_id>/<run_id>/{manifest,events,observations,evidence/}`

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const SCHEMA_VERSION: u32 = 1;
type LiveEvents = std::collections::HashMap<PathBuf, std::collections::VecDeque<HistoryEvent>>;
static LIVE_EVENTS: OnceLock<Mutex<LiveEvents>> = OnceLock::new();
fn live_events() -> &'static Mutex<LiveEvents> { LIVE_EVENTS.get_or_init(|| Mutex::new(LiveEvents::new())) }

/// Default soft byte budget for a single run directory (events+observations+evidence).
pub const DEFAULT_SOFT_BYTES_LIMIT: u64 = 64 * 1024 * 1024;
pub const DEFAULT_DEBUG_CAPACITY: usize = 256;
pub const DEFAULT_CRITICAL_CAPACITY: usize = 64;
pub const DEFAULT_FLUSH_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracePriority {
    /// State changes, clicks, terminal — reserved capacity; must not be crowded out by debug.
    Critical,
    /// Observations / verbose payloads — droppable when the debug queue is full.
    Debug,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceEnvelope {
    pub schema_version: u32,
    pub run_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_plan_id: Option<String>,
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryability: Option<String>,
    pub wall_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monotonic_ms: Option<u64>,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_refs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunManifest {
    pub schema_version: u32,
    pub run_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    /// running | succeeded | failed | unconfirmed | cancelled | interrupted
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_result: Option<String>,
    #[serde(default)]
    pub trace_degraded: bool,
    #[serde(default)]
    pub task_summary: Value,
    #[serde(default)]
    pub metrics: TraceMetricsSnapshot,
    #[serde(default)]
    pub device: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TraceMetricsSnapshot {
    pub queue_depth: u64,
    pub dropped_debug_count: u64,
    pub writer_error_count: u64,
    pub enqueue_us_total: u64,
    pub enqueue_count: u64,
    pub flush_ms: u64,
    pub bytes_written: u64,
}

#[derive(Debug, Clone)]
pub struct RunTraceConfig {
    pub debug_capacity: usize,
    pub critical_capacity: usize,
    pub soft_bytes_limit: u64,
    pub detailed_diagnostics: bool,
    pub flush_timeout: Duration,
}

impl Default for RunTraceConfig {
    fn default() -> Self {
        Self {
            debug_capacity: DEFAULT_DEBUG_CAPACITY,
            critical_capacity: DEFAULT_CRITICAL_CAPACITY,
            soft_bytes_limit: DEFAULT_SOFT_BYTES_LIMIT,
            detailed_diagnostics: detailed_diagnostics_from_env(false),
            flush_timeout: DEFAULT_FLUSH_TIMEOUT,
        }
    }
}

/// Env `XMU_SPORTS_DETAILED_DIAG=1|0` overrides the prefs default when set.
pub fn detailed_diagnostics_from_env(prefs_default: bool) -> bool {
    match std::env::var("XMU_SPORTS_DETAILED_DIAG") {
        Ok(v) => matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"),
        Err(_) => prefs_default,
    }
}

pub fn run_dir(runs_root: &Path, task_id: &str, run_id: &str) -> PathBuf {
    runs_root.join(task_id).join(run_id)
}

#[derive(Debug)]
pub enum EnqueueOutcome {
    Queued {
        #[allow(dead_code)]
        enqueue_us: u64,
    },
    DroppedDebug,
}

#[derive(Debug, Clone)]
pub struct FinishReport {
    pub trace_degraded: bool,
    pub metrics: TraceMetricsSnapshot,
    pub flush_ok: bool,
}

struct SharedMetrics {
    queue_depth: AtomicU64,
    dropped_debug_count: AtomicU64,
    writer_error_count: AtomicU64,
    enqueue_us_total: AtomicU64,
    enqueue_count: AtomicU64,
    flush_ms: AtomicU64,
    bytes_written: AtomicU64,
    degraded: AtomicBool,
}

impl SharedMetrics {
    fn new() -> Self {
        Self {
            queue_depth: AtomicU64::new(0),
            dropped_debug_count: AtomicU64::new(0),
            writer_error_count: AtomicU64::new(0),
            enqueue_us_total: AtomicU64::new(0),
            enqueue_count: AtomicU64::new(0),
            flush_ms: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            degraded: AtomicBool::new(false),
        }
    }

    fn snapshot(&self) -> TraceMetricsSnapshot {
        TraceMetricsSnapshot {
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            dropped_debug_count: self.dropped_debug_count.load(Ordering::Relaxed),
            writer_error_count: self.writer_error_count.load(Ordering::Relaxed),
            enqueue_us_total: self.enqueue_us_total.load(Ordering::Relaxed),
            enqueue_count: self.enqueue_count.load(Ordering::Relaxed),
            flush_ms: self.flush_ms.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
        }
    }
}

enum TraceJob {
    Line {
        file: TraceFile,
        line: String,
        bytes: u64,
    },
    UpdateManifest {
        patch: ManifestPatch,
    },
    Flush {
        ack: mpsc::Sender<()>,
    },
    Shutdown,
}

#[derive(Clone, Copy)]
enum TraceFile {
    Events,
    Observations,
}

struct ManifestPatch {
    status: Option<String>,
    final_result: Option<String>,
    ended_at: Option<String>,
    trace_degraded: Option<bool>,
    metrics: Option<TraceMetricsSnapshot>,
}

/// Handle used on the hot path: non-blocking enqueue only.
pub struct RunTraceHandle {
    run_dir: PathBuf,
    run_id: String,
    task_id: String,
    t0: Instant,
    critical_tx: SyncSender<TraceJob>,
    debug_tx: SyncSender<TraceJob>,
    wake_tx: mpsc::Sender<()>,
    metrics: Arc<SharedMetrics>,
    detailed_diagnostics: bool,
    soft_bytes_limit: u64,
    flush_timeout: Duration,
    join: Option<JoinHandle<()>>,
}

impl RunTraceHandle {
    pub fn open(
        runs_root: &Path,
        task_id: &str,
        run_id: &str,
        task_summary: Value,
        until: Option<&str>,
        config: RunTraceConfig,
    ) -> Result<Self, String> {
        if task_id.trim().is_empty() || run_id.trim().is_empty() {
            return Err("task_id/run_id must not be empty".into());
        }
        let dir = run_dir(runs_root, task_id, run_id);
        fs::create_dir_all(dir.join("evidence")).map_err(|e| e.to_string())?;

        let started_at = Local::now().to_rfc3339();
        let manifest = RunManifest {
            schema_version: SCHEMA_VERSION,
            run_id: run_id.to_string(),
            task_id: task_id.to_string(),
            until: until.map(str::to_string),
            started_at,
            ended_at: None,
            status: "running".into(),
            final_result: None,
            trace_degraded: false,
            task_summary,
            metrics: TraceMetricsSnapshot::default(),
            device: crate::wechat_window::device_diagnostics(),
        };
        write_manifest_atomic(&dir, &manifest)?;
        {
            let mut cache = live_events().lock().unwrap_or_else(|error| error.into_inner());
            if cache.len() >= 64 {
                if let Some(oldest) = cache.keys().next().cloned() { cache.remove(&oldest); }
            }
            cache.insert(runs_root.join(task_id), std::collections::VecDeque::new());
        }

        let metrics = Arc::new(SharedMetrics::new());
        let (critical_tx, critical_rx) = mpsc::sync_channel(config.critical_capacity);
        let (debug_tx, debug_rx) = mpsc::sync_channel(config.debug_capacity);
        let (wake_tx, wake_rx) = mpsc::channel();

        let writer_dir = dir.clone();
        let writer_metrics = Arc::clone(&metrics);
        let soft_limit = config.soft_bytes_limit;
        let join = thread::Builder::new()
            .name("run-trace-writer".into())
            .spawn(move || {
                writer_loop(
                    writer_dir,
                    critical_rx,
                    debug_rx,
                    wake_rx,
                    writer_metrics,
                    soft_limit,
                );
            })
            .map_err(|e| format!("spawn run-trace writer: {e}"))?;

        Ok(Self {
            run_dir: dir,
            run_id: run_id.to_string(),
            task_id: task_id.to_string(),
            t0: Instant::now(),
            critical_tx,
            debug_tx,
            wake_tx,
            metrics,
            detailed_diagnostics: config.detailed_diagnostics,
            soft_bytes_limit: config.soft_bytes_limit,
            flush_timeout: config.flush_timeout,
            join: Some(join),
        })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    #[allow(dead_code)]
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    #[allow(dead_code)]
    pub fn detailed_diagnostics(&self) -> bool {
        self.detailed_diagnostics
    }

    #[allow(dead_code)]
    pub fn metrics_snapshot(&self) -> TraceMetricsSnapshot {
        self.metrics.snapshot()
    }

    #[allow(dead_code)]
    pub fn is_degraded(&self) -> bool {
        self.metrics.degraded.load(Ordering::Relaxed)
    }

    fn monotonic_ms(&self) -> u64 {
        self.t0.elapsed().as_millis() as u64
    }

    /// Host event (always critical). `reason_code` must be set (never left blank intentionally).
    pub fn try_event(
        &self,
        stage: &str,
        page_id: Option<&str>,
        reason_code: &str,
        step_id: Option<&str>,
        frame_id: Option<&str>,
        action_plan_id: Option<&str>,
        extra_payload: Value,
    ) -> EnqueueOutcome {
        let mut payload = match extra_payload {
            Value::Object(map) => Value::Object(map),
            other => json!({ "extra": other }),
        };
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("action".into(), json!(stage));
            obj.insert("result".into(), json!(reason_code));
            if let Some(p) = page_id {
                obj.insert("page_id".into(), json!(p));
            }
            obj.insert("evidence".into(), json!(["host"]));
            obj.insert("executor_impl".into(), json!(if obj.contains_key("http") {"http"} else {"native"}));
            obj.insert("platform".into(), json!("windows"));
            // Flat RunEvent-compat fields for readers that ignore envelope nesting.
            obj.insert("ts".into(), json!(Local::now().to_rfc3339()));
            obj.insert("task_id".into(), json!(&self.task_id));
            obj.insert("run_id".into(), json!(&self.run_id));
            obj.insert("schema_version".into(), json!(SCHEMA_VERSION));
            obj.insert("reason_code".into(), json!(reason_code));
        }
        let env = TraceEnvelope {
            schema_version: SCHEMA_VERSION,
            run_id: self.run_id.clone(),
            task_id: self.task_id.clone(),
            step_id: step_id.map(str::to_string),
            parent_step_id: None,
            frame_id: frame_id.map(str::to_string),
            action_plan_id: action_plan_id.map(str::to_string),
            stage: stage.to_string(),
            reason_code: Some(reason_code.to_string()),
            retryability: retryability_for(reason_code),
            wall_time: Local::now().to_rfc3339(),
            monotonic_ms: Some(self.monotonic_ms()),
            payload,
            evidence_refs: None,
        };
        // UI state is available immediately, independent of the async disk writer.
        if let Some(mut event) = (stage != "http_exchange").then(|| serde_json::from_value::<HistoryEvent>(env.payload.clone()).ok()).flatten() {
            event.ts = env.wall_time.clone();
            let mut cache = live_events().lock().unwrap_or_else(|error| error.into_inner());
            if let Some(events) = self.run_dir.parent().and_then(|path| cache.get_mut(path)) {
                    events.push_back(event);
                    while events.len() > 120 { events.pop_front(); }
            }
        }
        self.enqueue_envelope(TracePriority::Critical, TraceFile::Events, env)
    }

    pub fn try_observation(&self, step:u32, phase:&str, observation:&Value)->EnqueueOutcome {
        let env=TraceEnvelope {
            schema_version:SCHEMA_VERSION,run_id:self.run_id.clone(),task_id:self.task_id.clone(),
            step_id:Some(step.to_string()),parent_step_id:None,frame_id:None,action_plan_id:None,
            stage:format!("observation:{phase}"),reason_code:Some("observed".into()),retryability:None,
            wall_time:Local::now().to_rfc3339(),monotonic_ms:Some(self.monotonic_ms()),
            payload:observation.clone(),evidence_refs:None,
        };
        self.enqueue_envelope(TracePriority::Debug,TraceFile::Observations,env)
    }

    fn enqueue_envelope(
        &self,
        priority: TracePriority,
        file: TraceFile,
        env: TraceEnvelope,
    ) -> EnqueueOutcome {
        let t0 = Instant::now();
        let line = match serde_json::to_string(&env) {
            Ok(s) => s,
            Err(_) => {
                self.metrics
                    .writer_error_count
                    .fetch_add(1, Ordering::Relaxed);
                self.metrics.degraded.store(true, Ordering::Relaxed);
                return EnqueueOutcome::DroppedDebug;
            }
        };
        let bytes = line.len() as u64 + 1;
        // Soft limit: drop debug when already over budget.
        if matches!(priority, TracePriority::Debug)
            && self.metrics.bytes_written.load(Ordering::Relaxed) >= self.soft_bytes_limit
        {
            self.metrics
                .dropped_debug_count
                .fetch_add(1, Ordering::Relaxed);
            return EnqueueOutcome::DroppedDebug;
        }
        let job = TraceJob::Line { file, line, bytes };
        let tx = match priority {
            TracePriority::Critical => &self.critical_tx,
            TracePriority::Debug => &self.debug_tx,
        };
        self.metrics.queue_depth.fetch_add(1, Ordering::Relaxed);
        match tx.try_send(job) {
            Ok(()) => {
                let _ = self.wake_tx.send(());
                let us = t0.elapsed().as_micros() as u64;
                self.metrics.enqueue_us_total.fetch_add(us, Ordering::Relaxed);
                self.metrics.enqueue_count.fetch_add(1, Ordering::Relaxed);
                EnqueueOutcome::Queued { enqueue_us: us }
            }
            Err(_) => {
                self.metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
                if matches!(priority, TracePriority::Debug) {
                    self.metrics
                        .dropped_debug_count
                        .fetch_add(1, Ordering::Relaxed);
                    EnqueueOutcome::DroppedDebug
                } else {
                    self.metrics.degraded.store(true, Ordering::Relaxed);
                    self.metrics
                        .writer_error_count
                        .fetch_add(1, Ordering::Relaxed);
                    EnqueueOutcome::DroppedDebug
                }
            }
        }
    }

    /// Flush with timeout, write final manifest, join writer.
    pub fn finish(mut self, status: &str, final_result: Option<&str>) -> FinishReport {
        let ended_at = Local::now().to_rfc3339();
        let degraded_before = self.metrics.degraded.load(Ordering::Relaxed);
        let (ack_tx, ack_rx) = mpsc::channel();
        let _ = self.critical_tx.try_send(TraceJob::Flush { ack: ack_tx });
        let _ = self.wake_tx.send(());
        let flush_ok = ack_rx.recv_timeout(self.flush_timeout).is_ok();
        if !flush_ok {
            self.metrics.degraded.store(true, Ordering::Relaxed);
        }
        let metrics = self.metrics.snapshot();
        let degraded = self.metrics.degraded.load(Ordering::Relaxed) || degraded_before || !flush_ok;
        let _ = self.critical_tx.try_send(TraceJob::UpdateManifest {
            patch: ManifestPatch {
                status: Some(status.into()),
                final_result: final_result.map(str::to_string),
                ended_at: Some(ended_at),
                trace_degraded: Some(degraded),
                metrics: Some(metrics.clone()),
            },
        });
        let _ = self.wake_tx.send(());
        // Never block forever on a full sync_channel / stuck writer.
        if self.critical_tx.try_send(TraceJob::Shutdown).is_err() {
            self.metrics.degraded.store(true, Ordering::Relaxed);
        }
        let _ = self.wake_tx.send(());
        let join_ok = self.join_writer_timeout(self.flush_timeout);
        let degraded = self.metrics.degraded.load(Ordering::Relaxed)
            || degraded_before
            || !flush_ok
            || !join_ok;
        // Final atomic manifest write from this thread (writer may have exited).
        if let Ok(mut m) = read_manifest(&self.run_dir) {
            m.status = status.into();
            m.final_result = final_result.map(str::to_string);
            m.ended_at = Some(Local::now().to_rfc3339());
            m.trace_degraded = degraded;
            m.metrics = self.metrics.snapshot();
            let _ = write_manifest_atomic(&self.run_dir, &m);
        }
        FinishReport {
            trace_degraded: degraded,
            metrics: self.metrics.snapshot(),
            flush_ok: flush_ok && join_ok,
        }
    }

    fn join_writer_timeout(&mut self, timeout: Duration) -> bool {
        let Some(j) = self.join.take() else {
            return true;
        };
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = j.join();
            let _ = done_tx.send(());
        });
        done_rx.recv_timeout(timeout).is_ok()
    }
}

impl Drop for RunTraceHandle {
    fn drop(&mut self) {
        if self.join.is_some() {
            let _ = self.critical_tx.try_send(TraceJob::Shutdown);
            let _ = self.wake_tx.send(());
            let _ = self.join_writer_timeout(DEFAULT_FLUSH_TIMEOUT);
        }
    }
}

fn writer_loop(
    run_dir: PathBuf,
    critical_rx: mpsc::Receiver<TraceJob>,
    debug_rx: mpsc::Receiver<TraceJob>,
    wake_rx: mpsc::Receiver<()>,
    metrics: Arc<SharedMetrics>,
    soft_limit: u64,
) {
    let events_path = run_dir.join("events.jsonl");
    let observations_path = run_dir.join("observations.jsonl");
    let evidence_dir = run_dir.join("evidence");
    let _ = fs::create_dir_all(&evidence_dir);

    let mut shutdown = false;
    while !shutdown {
        let mut did = false;
        while let Ok(job) = critical_rx.try_recv() {
            did = true;
            if matches!(job, TraceJob::Line { .. }) {
                metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
            }
            if handle_job(
                job,
                &run_dir,
                &events_path,
                &observations_path,
                &evidence_dir,
                &metrics,
                soft_limit,
                &mut shutdown,
            ) {
                // flush ack already sent inside handle
            }
        }
        while let Ok(job) = debug_rx.try_recv() {
            did = true;
            if matches!(job, TraceJob::Line { .. }) {
                metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
            }
            let _ = handle_job(
                job,
                &run_dir,
                &events_path,
                &observations_path,
                &evidence_dir,
                &metrics,
                soft_limit,
                &mut shutdown,
            );
        }
        if shutdown {
            break;
        }
        if !did {
            match wake_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    // Drain remaining then exit.
                    while let Ok(job) = critical_rx.try_recv() {
                        if matches!(job, TraceJob::Line { .. }) {
                            metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
                        }
                        let _ = handle_job(
                            job,
                            &run_dir,
                            &events_path,
                            &observations_path,
                            &evidence_dir,
                            &metrics,
                            soft_limit,
                            &mut shutdown,
                        );
                    }
                    while let Ok(job) = debug_rx.try_recv() {
                        if matches!(job, TraceJob::Line { .. }) {
                            metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
                        }
                        let _ = handle_job(
                            job,
                            &run_dir,
                            &events_path,
                            &observations_path,
                            &evidence_dir,
                            &metrics,
                            soft_limit,
                            &mut shutdown,
                        );
                    }
                    break;
                }
            }
        }
    }
}

fn handle_job(
    job: TraceJob,
    run_dir: &Path,
    events_path: &Path,
    observations_path: &Path,
    _evidence_dir: &Path,
    metrics: &SharedMetrics,
    soft_limit: u64,
    shutdown: &mut bool,
) -> bool {
    match job {
        TraceJob::Line { file, line, bytes } => {
            if metrics.bytes_written.load(Ordering::Relaxed) >= soft_limit
                && matches!(file, TraceFile::Observations)
            {
                metrics.dropped_debug_count.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            let path = match file {
                TraceFile::Events => events_path,
                TraceFile::Observations => observations_path,
            };
            if let Err(e) = append_line(path, &line) {
                log::warn!(target: "trace", "[writer_error] err={e}");
                metrics.writer_error_count.fetch_add(1, Ordering::Relaxed);
                metrics.degraded.store(true, Ordering::Relaxed);
            } else {
                metrics.bytes_written.fetch_add(bytes, Ordering::Relaxed);
            }
            false
        }
        TraceJob::UpdateManifest { patch } => {
            if let Ok(mut m) = read_manifest(run_dir) {
                if let Some(s) = patch.status {
                    m.status = s;
                }
                if let Some(r) = patch.final_result {
                    m.final_result = Some(r);
                }
                if let Some(t) = patch.ended_at {
                    m.ended_at = Some(t);
                }
                if let Some(d) = patch.trace_degraded {
                    m.trace_degraded = d;
                }
                if let Some(metrics_snap) = patch.metrics {
                    m.metrics = metrics_snap;
                }
                if let Err(e) = write_manifest_atomic(run_dir, &m) {
                    log::warn!(target: "trace", "[manifest_write_failed] err={e}");
                    metrics.writer_error_count.fetch_add(1, Ordering::Relaxed);
                    metrics.degraded.store(true, Ordering::Relaxed);
                }
            }
            false
        }
        TraceJob::Flush { ack } => {
            let t0 = Instant::now();
            // Best-effort: open+sync events/observations.
            for p in [events_path, observations_path] {
                if let Ok(f) = OpenOptions::new().create(true).append(true).open(p) {
                    let _ = f.sync_all();
                }
            }
            metrics
                .flush_ms
                .store(t0.elapsed().as_millis() as u64, Ordering::Relaxed);
            let _ = ack.send(());
            true
        }
        TraceJob::Shutdown => {
            *shutdown = true;
            false
        }
    }
}

fn append_line(path: &Path, line: &str) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(f, "{line}").map_err(|e| e.to_string())
}

pub fn write_manifest_atomic(run_dir: &Path, manifest: &RunManifest) -> Result<(), String> {
    let dst = run_dir.join("manifest.json");
    let raw = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    crate::atomic_fs::write_atomic(&dst, raw)
}

pub fn read_manifest(run_dir: &Path) -> Result<RunManifest, String> {
    let raw = fs::read_to_string(run_dir.join("manifest.json")).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

/// On host boot: any leftover `status=running` manifests become `interrupted`.
pub fn mark_interrupted_stale_runs(runs_root: &Path) -> usize {
    let mut n = 0usize;
    let Ok(tasks) = fs::read_dir(runs_root) else {
        return 0;
    };
    for task in tasks.flatten() {
        let task_path = task.path();
        if !task_path.is_dir() {
            continue;
        }
        let Ok(runs) = fs::read_dir(&task_path) else {
            continue;
        };
        for run in runs.flatten() {
            let run_path = run.path();
            if !run_path.is_dir() {
                continue;
            }
            let manifest_path = run_path.join("manifest.json");
            if !manifest_path.is_file() {
                continue;
            }
            let Ok(mut m) = read_manifest(&run_path) else {
                continue;
            };
            if m.status == "running" {
                m.status = "interrupted".into();
                m.ended_at = Some(Local::now().to_rfc3339());
                m.trace_degraded = true;
                if write_manifest_atomic(&run_path, &m).is_ok() {
                    n += 1;
                }
            }
        }
    }
    n
}

fn retryability_for(reason: &str) -> Option<String> {
    let r = reason.to_ascii_uppercase();
    if r.starts_with("PRE_INPUT_REBIND_REQUIRED")
        || r.contains("EVIDENCE_WEAK")
        || r == "OK"
        || r.ends_with("MS")
        || r == "STARTED"
        || r == "CHECKING"
        || r == "ALREADY_OPEN"
    {
        Some("retryable".into())
    } else if r == "REBIND_RETRY_EXHAUSTED"
        || r.contains("MISMATCH")
        || r.contains("UNAVAILABLE")
        || r == "MAX_STEPS"
        || r == "CANCELLED"
        || r == "FAILED"
        || r.contains("TIMEOUT")
        || r == "READY_TIMEOUT"
    {
        Some("terminal".into())
    } else {
        Some("unknown".into())
    }
}

/// Flatten TraceEnvelope / legacy RunEvent lines into history-friendly records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<Value>,
    pub ts: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_impl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

/// A terminal failure may expose a small exchange summary without polling raw replies.
pub fn failure_exchange(runs_root: &Path, task_id: &str, run_id: &str) -> Option<Value> {
    let valid = |key: &str| !key.is_empty() && key != "." && key != ".."
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    if !valid(task_id) || !valid(run_id) { return None; }
    let file = File::open(runs_root.join(task_id).join(run_id).join("events.jsonl")).ok()?;
    let mut latest = None;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<Value>(&line) else { continue; };
        if event["stage"] != "http_exchange" || event["task_id"] != task_id || event["run_id"] != run_id { continue; }
        let http = &event["payload"]["http"];
        let mut summary = serde_json::Map::new();
        for key in ["api", "method", "phase", "elapsed_ms", "http_status", "error", "error_type", "content_type", "timeout_seconds"] {
            if let Some(value) = http.get(key) { summary.insert(key.into(), value.clone()); }
        }
        if let Some(request) = http["request"].as_object() {
            let request: serde_json::Map<String, Value> = request.iter().take(16).map(|(key, value)| {
                let compact = if value.to_string().len() <= 256 { value.clone() } else { json!("较长参数，请查看导出日志") };
                (key.clone(), compact)
            }).collect();
            summary.insert("request".into(), Value::Object(request));
        }
        if let Some(info) = http["response"]["info"].as_str() {
            summary.insert("response".into(), json!({"info":info.chars().take(300).collect::<String>()}));
        }
        latest = Some(Value::Object(summary));
    }
    latest
}

pub fn history_event_from_line(line: &str) -> Option<HistoryEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    // Large diagnostic replies belong on disk/export, not in UI polling payloads.
    if v["stage"] == "http_exchange" || v["action"] == "http_exchange" { return None; }
    if v.get("schema_version").and_then(|x| x.as_u64()) == Some(SCHEMA_VERSION as u64)
        && v.get("stage").is_some()
        && v.get("payload").is_some()
    {
        let env: TraceEnvelope = serde_json::from_value(v).ok()?;
        let payload = &env.payload;
        return Some(HistoryEvent {
            http: payload.get("http").cloned(),
            ts: env.wall_time,
            task_id: env.task_id,
            run_id: Some(env.run_id),
            schema_version: Some(env.schema_version),
            page_id: payload
                .get("page_id")
                .and_then(|x| x.as_str())
                .map(str::to_string),
            action: payload
                .get("action")
                .and_then(|x| x.as_str())
                .map(str::to_string)
                .or(Some(env.stage)),
            result: payload
                .get("result")
                .and_then(|x| x.as_str())
                .map(str::to_string)
                .or(env.reason_code.clone()),
            evidence: payload
                .get("evidence")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            reason_code: env.reason_code,
            executor_impl: payload
                .get("executor_impl")
                .and_then(|x| x.as_str())
                .map(str::to_string),
            platform: payload
                .get("platform")
                .and_then(|x| x.as_str())
                .map(str::to_string),
        });
    }
    // Legacy flat RunEvent
    let mut ev: HistoryEvent = serde_json::from_value(v).ok()?;
    if ev.reason_code.is_none() {
        ev.reason_code = ev.result.clone();
    }
    Some(ev)
}

/// Collect events from new `task/run/events.jsonl` and legacy `task/events.jsonl`.
pub fn collect_history_events(runs_root: &Path) -> Result<Vec<HistoryEvent>, String> {
    let mut all = Vec::new();
    if !runs_root.is_dir() {
        return Ok(all);
    }
    for task_entry in fs::read_dir(runs_root).map_err(|e| e.to_string())? {
        let task_entry = task_entry.map_err(|e| e.to_string())?;
        let task_path = task_entry.path();
        if !task_path.is_dir() {
            continue;
        }
        // Legacy flat
        let legacy = task_path.join("events.jsonl");
        if legacy.is_file() {
            append_events_file(&legacy, &mut all)?;
        }
        // New per-run
        for run_entry in fs::read_dir(&task_path).map_err(|e| e.to_string())? {
            let run_entry = run_entry.map_err(|e| e.to_string())?;
            let run_path = run_entry.path();
            if !run_path.is_dir() {
                continue;
            }
            let evp = run_path.join("events.jsonl");
            if evp.is_file() {
                append_events_file(&evp, &mut all)?;
            }
        }
    }
    all.sort_by(|a, b| b.ts.cmp(&a.ts));
    Ok(all)
}

#[derive(Clone)]
struct CachedHistoryFile {
    len: u64,
    modified: Option<std::time::SystemTime>,
    events: Vec<HistoryEvent>,
}
type HistoryFiles = std::collections::HashMap<PathBuf, CachedHistoryFile>;
static HISTORY_FILES: OnceLock<Mutex<HistoryFiles>> = OnceLock::new();
fn history_files() -> &'static Mutex<HistoryFiles> {
    HISTORY_FILES.get_or_init(|| Mutex::new(HistoryFiles::new()))
}

fn append_events_file(path: &Path, out: &mut Vec<HistoryEvent>) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
    let modified = metadata.modified().ok();
    {
        let cache = history_files().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(path).filter(|entry| entry.len == metadata.len() && entry.modified == modified) {
            out.extend(entry.events.iter().cloned());
            return Ok(());
        }
    }
    let f = File::open(path).map_err(|e| e.to_string())?;
    let mut events = Vec::new();
    for line in BufReader::new(f).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(event) = history_event_from_line(&line) { events.push(event); }
    }
    // Cache only complete snapshots; an in-progress append is reread next time.
    let after = fs::metadata(path).map_err(|e| e.to_string())?;
    if after.len() == metadata.len() && after.modified().ok() == modified && events.len() <= 20_000 {
        let mut cache = history_files().lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= 128 || cache.values().map(|entry| entry.events.len()).sum::<usize>() + events.len() > 20_000 {
            cache.clear();
        }
        cache.insert(path.to_owned(), CachedHistoryFile { len: metadata.len(), modified, events: events.clone() });
    }
    out.extend(events);
    Ok(())
}

/// Read only the newest run, preserving short steps between frontend refreshes.
pub fn read_recent_run_events(runs_root: &Path, task_id: &str, limit: usize) -> Result<Vec<HistoryEvent>, String> {
    let task_path = runs_root.join(task_id);
    if !task_path.is_dir() {
        return Ok(Vec::new());
    }
    if let Some(events) = live_events().lock().unwrap_or_else(|error| error.into_inner()).get(&task_path) {
        if !events.is_empty() {
            let skip = events.len().saturating_sub(limit.max(1));
            return Ok(events.iter().skip(skip).cloned().collect());
        }
    }
    // Prefer newest per-run events.jsonl by mtime; fallback legacy.
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let legacy = task_path.join("events.jsonl");
    if legacy.is_file() {
        if let Ok(m) = fs::metadata(&legacy).and_then(|m| m.modified()) {
            candidates.push((m, legacy));
        }
    }
    if let Ok(runs) = fs::read_dir(&task_path) {
        for run in runs.flatten() {
            let evp = run.path().join("events.jsonl");
            if evp.is_file() {
                if let Ok(m) = fs::metadata(&evp).and_then(|m| m.modified()) {
                    candidates.push((m, evp));
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in candidates {
        let f = File::open(&path).map_err(|e| e.to_string())?;
        let mut recent = std::collections::VecDeque::new();
        for line in BufReader::new(f).lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(ev) = history_event_from_line(&line) {
                recent.push_back(ev);
                if recent.len() > limit.max(1) { recent.pop_front(); }
            }
        }
        if !recent.is_empty() {
            return Ok(recent.into_iter().collect());
        }
    }
    Ok(Vec::new())
}

/// Delete events.jsonl under both layouts (history clear).
pub fn clear_all_events(runs_root: &Path) -> Result<usize, String> {
    history_files().lock().unwrap_or_else(|error| error.into_inner()).retain(|path, _| !path.starts_with(runs_root));
    live_events().lock().unwrap_or_else(|error| error.into_inner()).retain(|path, _| !path.starts_with(runs_root));
    let mut n = 0usize;
    if !runs_root.is_dir() {
        return Ok(0);
    }
    for task_entry in fs::read_dir(runs_root).map_err(|e| e.to_string())? {
        let task_path = task_entry.map_err(|e| e.to_string())?.path();
        if !task_path.is_dir() {
            continue;
        }
        let legacy = task_path.join("events.jsonl");
        if legacy.is_file() {
            fs::remove_file(&legacy).map_err(|e| e.to_string())?;
            n += 1;
        }
        for run_entry in fs::read_dir(&task_path).map_err(|e| e.to_string())? {
            let run_path = run_entry.map_err(|e| e.to_string())?.path();
            if !run_path.is_dir() {
                continue;
            }
            let evp = run_path.join("events.jsonl");
            if evp.is_file() {
                fs::remove_file(&evp).map_err(|e| e.to_string())?;
                n += 1;
            }
        }
    }
    Ok(n)
}

pub fn safe_evidence_name(run_id: &str, step: u32, reason: &str) -> String {
    let safe_reason: String = reason
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    format!("{run_id}-step{step:03}-{safe_reason}.bmp")
}

#[cfg(test)]
mod tests {
    #[test]
    fn failure_summary_is_scoped_and_does_not_expose_large_responses() {
        let root = tmp_root("failure-summary");
        let trace = RunTraceHandle::open(&root, "task", "run", json!({}), None, RunTraceConfig::default()).unwrap();
        trace.try_event("http_exchange", None, "upstream_reply", None, None, None,
            json!({"http":{"api":"order_list","method":"POST","phase":"checking_orders","error":"timeout","error_type":"read_timeout","elapsed_ms":1875,"request":{"p":"2","long":"x".repeat(1000)},"response":{"info":"upstream reason","data":{"private":"not-for-ui"}}}}));
        trace.finish("failed", Some("HTTP_FAILED"));
        let summary = failure_exchange(&root, "task", "run").unwrap();
        assert_eq!(summary["request"]["p"], "2");
        assert_eq!(summary["error_type"], "read_timeout");
        assert_eq!(summary["elapsed_ms"], 1875);
        assert!(summary["request"]["long"].as_str().unwrap().len() < 256);
        assert!(summary["response"]["data"].is_null());
        assert!(!summary.to_string().contains("not-for-ui"));
        assert!(failure_exchange(&root, "task", "other").is_none());
        assert!(failure_exchange(&root, "../task", "run").is_none());
        assert!(failure_exchange(&root, "task", "..").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn upstream_evidence_is_saved_without_entering_live_or_reloaded_ui_history() {
        let root = tmp_root("upstream-evidence");
        let trace = RunTraceHandle::open(&root, "task", "run", json!({}), None, RunTraceConfig::default()).unwrap();
        trace.try_event("http_phase", None, "querying_slots", None, None, None, json!({}));
        trace.try_event("http_exchange", None, "upstream_reply", None, None, None,
            json!({"http":{"response":{"status":0,"info":"参数错误"},"request":{"venue_id":"14"}}}));
        let live = read_recent_run_events(&root, "task", 120).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].result.as_deref(), Some("querying_slots"));
        trace.finish("failed", None);
        let text = fs::read_to_string(root.join("task/run/events.jsonl")).unwrap();
        let evidence: Value = text.lines().map(|line|serde_json::from_str::<Value>(line).unwrap())
            .find(|value|value["stage"]=="http_exchange").unwrap();
        assert_eq!(evidence["payload"]["http"]["response"]["info"], "参数错误");
        assert!(text.lines().filter_map(history_event_from_line).all(|event|event.action.as_deref()!=Some("http_exchange")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn live_events_are_immediate_bounded_and_invalidated_when_history_is_cleared() {
        let root = tmp_root("live-cache");
        let trace = RunTraceHandle::open(&root, "task", "run", json!({}), None, RunTraceConfig::default()).unwrap();
        for index in 0..125 {
            trace.try_event("http_phase", None, &format!("phase{index}"), None, None, None, json!({"http":{"phase":index}}));
        }
        let live = read_recent_run_events(&root, "task", 120).unwrap();
        assert_eq!(live.len(), 120);
        assert_eq!(live.last().unwrap().result.as_deref(), Some("phase124"));
        trace.finish("succeeded", None);
        clear_all_events(&root).unwrap();
        assert!(read_recent_run_events(&root, "task", 120).unwrap().is_empty());
    }
    #[test]
    fn live_history_keeps_fast_steps_from_only_latest_run_and_bounds_payload() {
        let root = tmp_root("live-progress");
        let older = root.join("task").join("old");
        let latest = root.join("task").join("latest");
        fs::create_dir_all(&older).unwrap();
        fs::create_dir_all(&latest).unwrap();
        let older_path = older.join("events.jsonl");
        fs::write(&older_path, "{\"ts\":\"2026-10-13T18:00:00+08:00\",\"task_id\":\"task\",\"run_id\":\"old\",\"action\":\"fail\"}\n").unwrap();
        File::options().write(true).open(&older_path).unwrap().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(100)).unwrap();
        let rows: Vec<String> = ["captcha", "submitting", "verifying_order", "done"].iter().enumerate().map(|(index, phase)| {
            json!({"ts":format!("2026-10-13T18:55:42.00{index}+08:00"),"task_id":"task","run_id":"latest","action":phase,"http":{"phase":phase}}).to_string()
        }).collect();
        fs::write(latest.join("events.jsonl"), rows.join("\n") + "\n{").unwrap();
        let full = read_recent_run_events(&root, "task", 120).unwrap();
        assert_eq!(full.len(), 4);
        assert!(full.iter().all(|event| event.run_id.as_deref() == Some("latest")));
        assert_eq!(full[0].action.as_deref(), Some("captcha"));
        let tail = read_recent_run_events(&root, "task", 2).unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[1].action.as_deref(), Some("done"));
        let _ = fs::remove_dir_all(root);
    }
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    fn tmp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "xmu-run-trace-{}-{}-{}",
            tag,
            std::process::id(),
            Local::now().timestamp_millis()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn history_file_cache_observes_append_delete_and_clear() {
        let root = tmp_root("history-cache");
        let dir = root.join("task/run");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        let first = json!({"ts":"2026-10-23T07:00:00+08:00","task_id":"task","action":"spawn"}).to_string();
        let last = json!({"ts":"2026-10-23T07:00:01+08:00","task_id":"task","action":"done"}).to_string();
        fs::write(&path, format!("{first}\n")).unwrap();
        assert_eq!(collect_history_events(&root).unwrap().len(), 1);
        assert_eq!(collect_history_events(&root).unwrap().len(), 1);
        OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("{last}\n").as_bytes()).unwrap();
        let updated = collect_history_events(&root).unwrap();
        assert_eq!(updated.len(), 2);
        assert_eq!(updated[0].action.as_deref(), Some("done"));
        fs::remove_file(&path).unwrap();
        assert!(collect_history_events(&root).unwrap().is_empty());
        fs::write(&path, format!("{last}\n")).unwrap();
        assert_eq!(collect_history_events(&root).unwrap()[0].action.as_deref(), Some("done"));
        clear_all_events(&root).unwrap();
        assert!(collect_history_events(&root).unwrap().is_empty());
        assert!(!history_files().lock().unwrap().contains_key(&path));
        let _ = fs::remove_dir_all(root);
    }

    /// Opt-in read-only profiling; prints counts/timings, never user content.
    #[test]
    #[ignore]
    fn benchmark_history_read() {
        let root = PathBuf::from(std::env::var("XMU_HISTORY_BENCH_ROOT").expect("Set XMU_HISTORY_BENCH_ROOT to a runs directory"));
        let cold_start = Instant::now();
        let cold = collect_history_events(&root).unwrap();
        let cold_ms = cold_start.elapsed().as_secs_f64() * 1000.;
        let warm_start = Instant::now();
        let warm = collect_history_events(&root).unwrap();
        let warm_ms = warm_start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(cold.len(), warm.len());
        println!("history events={} cold_ms={cold_ms:.2} cached_ms={warm_ms:.2}", cold.len());
    }

    #[test]
    fn envelope_roundtrip_and_history_mapping() {
        let env = TraceEnvelope {
            schema_version: SCHEMA_VERSION,
            run_id: "r1".into(),
            task_id: "t1".into(),
            step_id: Some("3".into()),
            parent_step_id: None,
            frame_id: Some("f1".into()),
            action_plan_id: None,
            stage: "glance".into(),
            reason_code: Some("ok".into()),
            retryability: Some("retryable".into()),
            wall_time: "2026-09-28T00:00:00+08:00".into(),
            monotonic_ms: Some(12),
            payload: json!({
                "action": "glance",
                "result": "ok",
                "page_id": "venue_detail",
                "evidence": ["host"],
                "executor_impl": "native",
                "platform": "windows",
            }),
            evidence_refs: None,
        };
        let line = serde_json::to_string(&env).unwrap();
        let hist = history_event_from_line(&line).expect("map");
        assert_eq!(hist.task_id, "t1");
        assert_eq!(hist.run_id.as_deref(), Some("r1"));
        assert_eq!(hist.action.as_deref(), Some("glance"));
        assert_eq!(hist.reason_code.as_deref(), Some("ok"));
        assert_eq!(hist.page_id.as_deref(), Some("venue_detail"));
    }

    #[test]
    fn legacy_flat_event_still_parses() {
        let line = r#"{"ts":"2026-09-27T12:00:00+08:00","task_id":"t","action":"spawn","result":"started","evidence":["host"]}"#;
        let hist = history_event_from_line(line).expect("legacy");
        assert_eq!(hist.task_id, "t");
        assert_eq!(hist.action.as_deref(), Some("spawn"));
        assert_eq!(hist.reason_code.as_deref(), Some("started"));
    }

    #[test]
    fn open_writes_layout_and_critical_events_flush() {
        let root = tmp_root("layout");
        let cfg = RunTraceConfig {
            debug_capacity: 8,
            critical_capacity: 8,
            soft_bytes_limit: 1_000_000,
            detailed_diagnostics: false,
            flush_timeout: Duration::from_secs(2),
        };
        let trace = RunTraceHandle::open(
            &root,
            "task-a",
            "run-1",
            json!({"sport": "羽毛球"}),
            Some("trial_slot"),
            cfg,
        )
        .expect("open");
        let dir = run_dir(&root, "task-a", "run-1");
        assert!(dir.join("manifest.json").is_file());
        assert!(dir.join("evidence").is_dir());

        let _ = trace.try_event("spawn", None, "started", None, None, None, json!({}));
        let _ = trace.try_event("done", Some("confirm"), "TRIAL_OK", Some("1"), None, None, json!({}));
        let finish = trace.finish("succeeded", Some("TRIAL_OK"));
        assert!(finish.flush_ok, "flush should succeed");
        assert!(!finish.trace_degraded);
        assert_eq!(finish.metrics.queue_depth, 0, "finished writer queue must be empty");

        let events = fs::read_to_string(dir.join("events.jsonl")).unwrap();
        assert!(events.contains("\"run_id\":\"run-1\""));
        assert!(events.contains("\"reason_code\":\"started\""));
        assert!(events.contains("\"schema_version\":1"));

        let m = read_manifest(&dir).unwrap();
        assert_eq!(m.status, "succeeded");
        assert_eq!(m.final_result.as_deref(), Some("TRIAL_OK"));
        assert_eq!(m.schema_version, 1);

        let hist = collect_history_events(&root).unwrap();
        assert!(hist.iter().any(|e| e.action.as_deref() == Some("spawn")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn debug_queue_drop_does_not_block_and_counts() {
        let root = tmp_root("drop");
        let cfg = RunTraceConfig {
            debug_capacity: 1,
            critical_capacity: 8,
            soft_bytes_limit: 1_000_000,
            detailed_diagnostics: true,
            flush_timeout: Duration::from_secs(2),
        };
        let mut trace = RunTraceHandle::open(
            &root,
            "task-b",
            "run-2",
            json!({}),
            None,
            cfg,
        )
        .unwrap();
        // Hold the debug receiver explicitly; flooding a live writer races its drain.
        // Restore the real writer afterwards so finish/critical flush stay covered.
        let (held_tx, held_rx) = mpsc::sync_channel(1);
        let writer_tx = std::mem::replace(&mut trace.debug_tx, held_tx);
        let observation=json!({"page":"test","padding":"x".repeat(4096)});
        let mut dropped = 0u32;
        for i in 0..20 {
            match trace.try_observation(i, "test", &observation) {
                EnqueueOutcome::DroppedDebug => dropped += 1,
                EnqueueOutcome::Queued { .. } => {}
            }
        }
        assert_eq!(dropped, 19, "one queued item must fill the held capacity=1 queue");
        let snap = trace.metrics_snapshot();
        assert_eq!(snap.dropped_debug_count, 19);
        let queued = held_rx.try_recv().expect("one observation queued");
        trace.debug_tx = writer_tx;
        assert!(trace.debug_tx.try_send(queued).is_ok());
        let _ = trace.wake_tx.send(());
        let _ = trace.try_event("fail", None, "DATE_MISMATCH", None, None, None, json!({}));
        let finish = trace.finish("failed", Some("DATE_MISMATCH"));
        assert!(finish.flush_ok);
        assert_eq!(finish.metrics.queue_depth, 0);
        let events = fs::read_to_string(run_dir(&root, "task-b", "run-2").join("events.jsonl"))
            .unwrap();
        assert!(events.contains("DATE_MISMATCH"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn mark_interrupted_stale_running_manifest() {
        let root = tmp_root("interrupt");
        let dir = run_dir(&root, "t", "r");
        fs::create_dir_all(dir.join("evidence")).unwrap();
        let m = RunManifest {
            schema_version: SCHEMA_VERSION,
            run_id: "r".into(),
            task_id: "t".into(),
            until: None,
            started_at: Local::now().to_rfc3339(),
            ended_at: None,
            status: "running".into(),
            final_result: None,
            trace_degraded: false,
            task_summary: json!({}),
            metrics: TraceMetricsSnapshot::default(),
            device: json!({}),
        };
        write_manifest_atomic(&dir, &m).unwrap();
        assert_eq!(mark_interrupted_stale_runs(&root), 1);
        let m2 = read_manifest(&dir).unwrap();
        assert_eq!(m2.status, "interrupted");
        assert!(m2.trace_degraded);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_prefers_new_layout_and_falls_back_legacy() {
        let root = tmp_root("compat");
        let legacy_dir = root.join("old-task");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("events.jsonl"),
            r#"{"ts":"2026-09-01T00:00:00+08:00","task_id":"old-task","action":"spawn","result":"started","evidence":[]}"#,
        )
        .unwrap();
        let new_dir = run_dir(&root, "new-task", "run-x");
        fs::create_dir_all(new_dir.join("evidence")).unwrap();
        let env = TraceEnvelope {
            schema_version: SCHEMA_VERSION,
            run_id: "run-x".into(),
            task_id: "new-task".into(),
            step_id: None,
            parent_step_id: None,
            frame_id: None,
            action_plan_id: None,
            stage: "spawn".into(),
            reason_code: Some("started".into()),
            retryability: None,
            wall_time: "2026-09-28T01:00:00+08:00".into(),
            monotonic_ms: None,
            payload: json!({"action":"spawn","result":"started","evidence":["host"]}),
            evidence_refs: None,
        };
        fs::write(
            new_dir.join("events.jsonl"),
            serde_json::to_string(&env).unwrap() + "\n",
        )
        .unwrap();
        let all = collect_history_events(&root).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|e| e.task_id == "old-task"));
        assert!(all.iter().any(|e| e.run_id.as_deref() == Some("run-x")));
        let _ = fs::remove_dir_all(&root);
    }
}

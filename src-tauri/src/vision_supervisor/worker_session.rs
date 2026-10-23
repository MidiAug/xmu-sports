//! WorkerSession — **probe-only** IPC to `vision-worker`.
//!
//! Booking / arm / trial execution lives in the host RunSession (`run_native_loop`).
//! This session is for capability probes and future Spec-gated elevation only.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use thiserror::Error;
use vision_contracts::{CapabilityReport, IpcEnvelope, IpcKind, ReasonCode, PROTOCOL_VERSION};

#[derive(Debug, Error)]
pub enum WorkerSessionError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub struct WorkerSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl WorkerSession {
    pub fn spawn(bin: &Path, data_root: &Path) -> Result<Self, WorkerSessionError> {
        let mut cmd = Command::new(bin);
        cmd.env("XMU_SPORTS_DATA", data_root)
            .env("VISION_WORKER_PLATFORM", "fake")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| WorkerSessionError::Message("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerSessionError::Message("no stdout".into()))?;
        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        session.hello()?;
        Ok(session)
    }

    fn next_request_id(&mut self) -> String {
        let id = self.next_id;
        self.next_id += 1;
        format!("host-{id}")
    }

    fn send_recv(&mut self, kind: IpcKind, payload: serde_json::Value) -> Result<IpcEnvelope, WorkerSessionError> {
        let req = IpcEnvelope::new(self.next_request_id(), kind, payload);
        let line = serde_json::to_string(&req)?;
        writeln!(self.stdin, "{line}")?;
        self.stdin.flush()?;

        // Bounded wait via try_wait + read with timeout approximation
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(WorkerSessionError::Message(format!(
                    "{}: worker exited {status}",
                    ReasonCode::WORKER_EXITED
                )));
            }
            // Non-blocking-ish: BufReader will block; rely on worker being responsive.
            let mut resp_line = String::new();
            match self.stdout.read_line(&mut resp_line) {
                Ok(0) => {
                    return Err(WorkerSessionError::Message(
                        ReasonCode::WORKER_EXITED.into(),
                    ));
                }
                Ok(_) => {
                    let env: IpcEnvelope = serde_json::from_str(resp_line.trim())?;
                    env.check_protocol()
                        .map_err(WorkerSessionError::Message)?;
                    return Ok(env);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    if std::time::Instant::now() > deadline {
                        return Err(WorkerSessionError::Message("timeout".into()));
                    }
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    pub fn hello(&mut self) -> Result<(), WorkerSessionError> {
        let env = self.send_recv(
            IpcKind::Hello,
            serde_json::json!({
                "app_version": env!("CARGO_PKG_VERSION"),
                "worker_role": "vision-worker",
                "input_mode": "fake"
            }),
        )?;
        if env.kind != IpcKind::HelloAck {
            return Err(WorkerSessionError::Message(format!(
                "expected hello_ack got {:?}",
                env.kind
            )));
        }
        let pv = env
            .payload
            .get("protocol_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if pv != PROTOCOL_VERSION as u64 {
            return Err(WorkerSessionError::Message(
                ReasonCode::PROTOCOL_MISMATCH.into(),
            ));
        }
        Ok(())
    }

    pub fn check_capabilities(&mut self) -> Result<CapabilityReport, WorkerSessionError> {
        let env = self.send_recv(IpcKind::CheckCapabilities, serde_json::json!({}))?;
        if env.kind != IpcKind::Capabilities {
            return Err(WorkerSessionError::Message(format!(
                "expected capabilities got {:?}",
                env.kind
            )));
        }
        Ok(serde_json::from_value(env.payload)?)
    }

    pub fn shutdown(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

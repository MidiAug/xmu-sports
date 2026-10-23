//! vision-worker: stdin/stdout JSON envelopes for **diagnostic probes**.
//!
//! Product booking / arm / trial run in the host process (`RunSession` /
//! `run_native_loop`). `StartRun` here is a stub until a future Spec elevates
//! the worker. Default input mode is fake — never clicks the live desktop
//! unless payload says so AND host has already gated on pack + explicit authority.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde_json::json;
use vision_contracts::{
    CapabilityReport, CapabilityStatus, IpcEnvelope, IpcKind, PlatformKind, ReasonCode,
    PROTOCOL_VERSION,
};
use vision_platform_api::{FakePlatform, PlatformPort};
use vision_pack::{active_pack_dir, PackStatus};

fn read_envelope(stdin: &mut impl BufRead) -> io::Result<Option<IpcEnvelope>> {
    let mut line = String::new();
    let n = stdin.read_line(&mut line)?;
    if n == 0 {
        return Ok(None);
    }
    let env: IpcEnvelope = serde_json::from_str(line.trim())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(env))
}

fn write_envelope(stdout: &mut impl Write, env: &IpcEnvelope) -> io::Result<()> {
    let line = serde_json::to_string(env)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    writeln!(stdout, "{line}")?;
    stdout.flush()
}

fn pack_ready(data_root: &PathBuf) -> bool {
    active_pack_dir(data_root).is_some()
}

fn main() {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let data_root = std::env::var("XMU_SPORTS_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    // Default: fake platform so CI never touches the desktop.
    let use_fake = std::env::var("VISION_WORKER_PLATFORM")
        .map(|v| v == "fake" || v.is_empty())
        .unwrap_or(true);

    let mut fake = FakePlatform::new();
    fake.pack_ready = pack_ready(&data_root);
    fake.input_enabled = false;

    #[cfg(windows)]
    let mut win = {
        let mut p = vision_platform_windows::WindowsPlatform::new();
        p.pack_ready = fake.pack_ready;
        p.input_enabled = false;
        p
    };

    let mut active_run: Option<String> = None;

    loop {
        let env = match read_envelope(&mut stdin) {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                eprintln!("vision-worker read error: {e}");
                break;
            }
        };
        if let Err(msg) = env.check_protocol() {
            let _ = write_envelope(
                &mut stdout,
                &IpcEnvelope::new(
                    env.request_id.clone(),
                    IpcKind::Error,
                    json!({ "reason_code": ReasonCode::PROTOCOL_MISMATCH, "message": msg }),
                ),
            );
            break;
        }

        match env.kind {
            IpcKind::Hello => {
                let ack = IpcEnvelope::new(
                    env.request_id,
                    IpcKind::HelloAck,
                    json!({
                        "protocol_version": PROTOCOL_VERSION,
                        "worker_version": env!("CARGO_PKG_VERSION"),
                        "platform_mode": if use_fake { "fake" } else { "native" },
                        "pack_ready": fake.pack_ready,
                    }),
                );
                let _ = write_envelope(&mut stdout, &ack);
            }
            IpcKind::Ping => {
                let _ = write_envelope(
                    &mut stdout,
                    &IpcEnvelope::new(env.request_id, IpcKind::Pong, json!({})),
                );
            }
            IpcKind::CheckCapabilities => {
                let report: CapabilityReport = if use_fake {
                    fake.check_capabilities().unwrap_or_else(|e| CapabilityReport {
                        status: CapabilityStatus::RuntimeFault,
                        reason_code: e.to_string(),
                        platform: PlatformKind::Fake,
                        pack_ready: fake.pack_ready,
                        input_enabled: false,
                        details: None,
                    })
                } else {
                    #[cfg(windows)]
                    {
                        win.pack_ready = pack_ready(&data_root);
                        win.check_capabilities().unwrap_or_else(|e| CapabilityReport {
                            status: CapabilityStatus::RuntimeFault,
                            reason_code: e.to_string(),
                            platform: PlatformKind::Windows,
                            pack_ready: win.pack_ready,
                            input_enabled: false,
                            details: None,
                        })
                    }
                    #[cfg(not(windows))]
                    {
                        CapabilityReport {
                            status: CapabilityStatus::Unsupported,
                            reason_code: ReasonCode::MACOS_PATH_UNVERIFIED.into(),
                            platform: PlatformKind::Macos,
                            pack_ready: pack_ready(&data_root),
                            input_enabled: false,
                            details: None,
                        }
                    }
                };
                let _ = write_envelope(
                    &mut stdout,
                    &IpcEnvelope::new(
                        env.request_id,
                        IpcKind::Capabilities,
                        serde_json::to_value(report).unwrap_or(json!({})),
                    ),
                );
            }
            // Probe / stub only — booking execution is host RunSession, not this worker.
            IpcKind::StartRun => {
                if !pack_ready(&data_root) {
                    let _ = write_envelope(
                        &mut stdout,
                        &IpcEnvelope::new(
                            env.request_id,
                            IpcKind::Error,
                            json!({
                                "reason_code": ReasonCode::PACK_NOT_INSTALLED,
                                "message": "recognition pack required before booking run",
                                "pack_status": format!("{:?}", PackStatus::NotInstalled),
                            }),
                        ),
                    );
                    continue;
                }
                if active_run.is_some() {
                    let _ = write_envelope(
                        &mut stdout,
                        &IpcEnvelope::new(
                            env.request_id,
                            IpcKind::Error,
                            json!({ "reason_code": "RUN_ACTIVE", "message": "only one active run" }),
                        ),
                    );
                    continue;
                }
                let run_id = env
                    .payload
                    .get("run_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("run")
                    .to_string();
                let input_mode = env
                    .payload
                    .get("input_mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("fake");
                // Worker refuses live input unless explicitly requested; host must also authorize.
                let live = input_mode == "live";
                fake.input_enabled = live && use_fake;
                #[cfg(windows)]
                {
                    win.input_enabled = live && !use_fake;
                }
                if live {
                    // Hard stop: this binary never auto-enables live without VISION_ALLOW_LIVE=1
                    if std::env::var("VISION_ALLOW_LIVE").ok().as_deref() != Some("1") {
                        fake.input_enabled = false;
                        #[cfg(windows)]
                        {
                            win.input_enabled = false;
                        }
                        let _ = write_envelope(
                            &mut stdout,
                            &IpcEnvelope::new(
                                env.request_id,
                                IpcKind::Error,
                                json!({
                                    "reason_code": ReasonCode::INPUT_DISABLED,
                                    "message": "live input requires VISION_ALLOW_LIVE=1 and explicit host trial/arm"
                                }),
                            ),
                        );
                        continue;
                    }
                }
                active_run = Some(run_id.clone());
                let finished = IpcEnvelope::new(
                    env.request_id,
                    IpcKind::RunFinished,
                    json!({
                        "run_id": run_id,
                        "result": "shadow_or_probe_only",
                        "message": "worker accepts run; full fire path not yet enabled as default"
                    }),
                )
                .with_run_id(active_run.take().unwrap_or_default());
                let _ = write_envelope(&mut stdout, &finished);
            }
            IpcKind::CancelRun => {
                active_run = None;
                fake.input_enabled = false;
                #[cfg(windows)]
                {
                    win.input_enabled = false;
                }
                let _ = write_envelope(
                    &mut stdout,
                    &IpcEnvelope::new(env.request_id, IpcKind::RunFinished, json!({ "cancelled": true })),
                );
            }
            other => {
                let _ = write_envelope(
                    &mut stdout,
                    &IpcEnvelope::new(
                        env.request_id,
                        IpcKind::Error,
                        json!({ "message": format!("unhandled kind {other:?}") }),
                    ),
                );
            }
        }
    }
}

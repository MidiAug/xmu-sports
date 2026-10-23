use serde::{Deserialize, Serialize};

/// IPC protocol major version — independent of BookingTask schema.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcKind {
    Hello,
    HelloAck,
    CheckCapabilities,
    Capabilities,
    StartRun,
    RunEvent,
    CancelRun,
    RunFinished,
    Error,
    Ping,
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcEnvelope {
    pub protocol_version: u32,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub kind: IpcKind,
    pub payload: serde_json::Value,
}

impl IpcEnvelope {
    pub fn new(request_id: impl Into<String>, kind: IpcKind, payload: serde_json::Value) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            run_id: None,
            kind,
            payload,
        }
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn check_protocol(&self) -> Result<(), String> {
        if self.protocol_version != PROTOCOL_VERSION {
            Err(format!(
                "protocol_version mismatch: got {} want {}",
                self.protocol_version, PROTOCOL_VERSION
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_roundtrip_matches_example() {
        let raw = include_str!("../../../../../docs/contracts/examples/vision_ipc_hello.json");
        let env: IpcEnvelope = serde_json::from_str(raw).expect("parse hello");
        assert_eq!(env.protocol_version, PROTOCOL_VERSION);
        assert_eq!(env.kind, IpcKind::Hello);
        let back = serde_json::to_value(&env).unwrap();
        let again: IpcEnvelope = serde_json::from_value(back).unwrap();
        assert_eq!(again.request_id, "req-0001");
    }
}

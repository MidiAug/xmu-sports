//! Injectable PlatformPort — no Win32/Cocoa types.

mod fake;
mod port;

pub use fake::{FakeInputSink, FakePlatform, FakeScript};
pub use port::{
    ActivationStatus, CaptureError, InputError, InputReceipt, PlatformDiagnostics, PlatformError,
    PlatformPort,
};

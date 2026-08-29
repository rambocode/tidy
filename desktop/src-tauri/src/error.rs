// IPC error envelope: stable machine-readable codes so the frontend can
// branch on cause, mirroring the project rule that a refusing gate must name
// which cause it hit and what to do next.

use serde::Serialize;

/// Error payload every command returns on failure.
#[derive(Debug, Serialize)]
pub struct IpcError {
    /// Stable code: protected_path / plan_expired / plan_not_found /
    /// selection_mismatch / cancelled / requires_admin / io.
    pub code: &'static str,
    pub message: String,
}

impl IpcError {
    /// Build an error with a stable code and human message.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for IpcError {
    fn from(e: std::io::Error) -> Self {
        IpcError::new("io", e.to_string())
    }
}

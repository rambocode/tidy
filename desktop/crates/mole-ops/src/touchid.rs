// Touch ID for sudo: read-only status probe. Configuration edits /etc/pam.d
// and stays with the CLI (or a future helper); the GUI only reports state.

use serde::Serialize;

/// Touch ID / pam_tid status for sudo.
#[derive(Debug, Serialize)]
pub struct TouchIdStatus {
    /// pam_tid.so present in /etc/pam.d/sudo or sudo_local.
    pub enabled: bool,
    /// Which file carries the entry, when enabled.
    pub source: Option<String>,
}

/// Probe the PAM sudo files for pam_tid.so (the `mole` router's one-liner).
pub fn status() -> TouchIdStatus {
    for file in ["/etc/pam.d/sudo_local", "/etc/pam.d/sudo"] {
        if let Ok(content) = std::fs::read_to_string(file) {
            // Comment lines must not read as configuration.
            let live = content
                .lines()
                .any(|l| !l.trim_start().starts_with('#') && l.contains("pam_tid.so"));
            if live {
                return TouchIdStatus {
                    enabled: true,
                    source: Some(file.to_string()),
                };
            }
        }
    }
    TouchIdStatus {
        enabled: false,
        source: None,
    }
}

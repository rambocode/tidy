// Whitelist feature: read/write the SAME ~/.config/mole/whitelist file the
// CLI uses. Validation lives in mole-core::state; invalid entries come back
// as named warnings instead of being silently dropped.

use mole_core::state::{load_whitelist, save_whitelist, Whitelist};

/// Load the user's whitelist with validation warnings.
pub fn get(home: &str) -> Whitelist {
    load_whitelist(home)
}

/// Save patterns after a validation round trip; returns the warnings for any
/// entries the grammar rejected (they are not written).
pub fn set(home: &str, patterns: &[String]) -> std::io::Result<Vec<String>> {
    save_whitelist(home, patterns)?;
    // Re-load through the validating parser so the caller sees exactly what
    // the CLI will accept from the file we just wrote.
    let wl = load_whitelist(home);
    Ok(wl.warnings)
}

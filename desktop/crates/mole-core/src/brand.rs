// Shared on-disk identity. Tidy is the desktop product name, but state and
// operation logs intentionally retain Mole's paths so the CLI and desktop app
// read the same policy and history.

/// Config directory slug: `~/.config/<CONFIG_DIR>/`.
pub const CONFIG_DIR: &str = "mole";

/// Log directory name: `~/Library/Logs/<LOG_DIR>/`.
pub const LOG_DIR: &str = "mole";

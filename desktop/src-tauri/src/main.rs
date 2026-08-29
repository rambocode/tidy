// Bootstrap only (mirrors the `mole` router-only rule): everything lives in lib.rs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tidy_lib::run()
}

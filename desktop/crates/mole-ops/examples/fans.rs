// Manual probe: print live SMC fan telemetry as JSON (empty on fanless Macs).
fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&mole_ops::smc::fans()).unwrap()
    );
}

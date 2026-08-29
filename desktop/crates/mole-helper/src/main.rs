// Helper binary entry point. The XPC service loop (SMAppService registration,
// audit-token identity, per-request vetting through lib.rs) lands with the
// signing phase of the release tooling; running the binary directly does
// nothing privileged and says so.

fn main() {
    eprintln!(
        "mole-helper: this binary only serves XPC requests when registered via \
         SMAppService inside the signed Mole Desktop bundle; direct invocation does nothing."
    );
    std::process::exit(1);
}

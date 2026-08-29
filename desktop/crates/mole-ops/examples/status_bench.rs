// CPU-cost harness for status::snapshot(): runs N snapshots on the dashboard's
// 2s cadence and reports per-call CPU-ms (self threads + spawned children),
// burst parallelism (cpu/wall while the call runs, informational), and the
// sustained duty cycle. Activity Monitor averages over ~1-2s windows, so the
// red condition is per-tick CPU share, not instantaneous parallelism: red when
// any single tick consumes ≥ 50% of its 2s window or the average duty ≥ 20%.

/// Total user+sys CPU seconds for `who` (RUSAGE_SELF or RUSAGE_CHILDREN).
fn cpu_secs(who: i32) -> f64 {
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        libc::getrusage(who, &mut ru);
        let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        tv(ru.ru_utime) + tv(ru.ru_stime)
    }
}

fn main() {
    const TICK_SECS: f64 = 2.0; // dashboard refresh cadence
    const ITERS: usize = 10;

    // Warm one-time caches (system_profiler GPU core count) so steady-state
    // cost is what gets measured, matching a dashboard left open.
    let _ = mole_ops::status::snapshot();

    let mut sum_self = 0.0;
    let mut sum_child = 0.0;
    let mut sum_wall = 0.0;
    let mut max_burst: f64 = 0.0;
    let mut max_tick_duty: f64 = 0.0;
    for i in 0..ITERS {
        let (s0, c0) = (cpu_secs(libc::RUSAGE_SELF), cpu_secs(libc::RUSAGE_CHILDREN));
        let t0 = std::time::Instant::now();
        let snap = mole_ops::status::snapshot();
        let wall = t0.elapsed().as_secs_f64();
        let self_cpu = cpu_secs(libc::RUSAGE_SELF) - s0;
        let child_cpu = cpu_secs(libc::RUSAGE_CHILDREN) - c0;
        // Burst = how many cores the call keeps busy while it runs.
        let burst = (self_cpu + child_cpu) / wall.max(0.001);
        max_burst = max_burst.max(burst);
        max_tick_duty = max_tick_duty.max((self_cpu + child_cpu) / TICK_SECS * 100.0);
        sum_self += self_cpu;
        sum_child += child_cpu;
        sum_wall += wall;
        println!(
            "tick {:2}: wall {:6.0}ms  self {:6.1}ms  children {:6.1}ms  burst {:4.2} cores  (procs={})",
            i,
            wall * 1000.0,
            self_cpu * 1000.0,
            child_cpu * 1000.0,
            burst,
            snap.top_processes.len()
        );
        let remain = TICK_SECS - wall;
        if remain > 0.0 {
            std::thread::sleep(std::time::Duration::from_secs_f64(remain));
        }
    }

    let per_call_cpu = (sum_self + sum_child) / ITERS as f64;
    let duty = per_call_cpu / TICK_SECS * 100.0;
    println!("---");
    println!(
        "avg per call: self {:.1}ms  children {:.1}ms  wall {:.0}ms",
        sum_self / ITERS as f64 * 1000.0,
        sum_child / ITERS as f64 * 1000.0,
        sum_wall / ITERS as f64 * 1000.0
    );
    println!(
        "sustained duty cycle @2s: {:.1}%   worst tick: {:.1}%   peak call burst: {:.2} cores",
        duty, max_tick_duty, max_burst
    );
    // Pass/fail verdict so the harness is red-capable, not just informative.
    let pass = duty < 20.0 && max_tick_duty < 50.0;
    println!("verdict: {}", if pass { "PASS" } else { "FAIL" });
    std::process::exit(if pass { 0 } else { 1 });
}

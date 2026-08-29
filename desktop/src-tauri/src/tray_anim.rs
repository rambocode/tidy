// Menu-bar tray animation, two modes driven by one persistent thread:
//   idle — the brand icon slowly "breathes" (sine alpha fade, ~3s cycle);
//   busy — a rotating spinner (template image, adapts to light/dark menus)
//          while any long-running command holds a BusyGuard.
// Spinner frames are drawn procedurally; breathing frames are the decoded
// brand PNG with its alpha channel scaled per tick. No extra image assets.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::image::Image;
use tauri::AppHandle;

/// Spinner geometry: 44px canvas (22pt menu bar @2x), 12-frame rotation at
/// ~11 fps — smooth enough for a spinner, negligible CPU.
const SPIN_SIZE: u32 = 44;
const SPIN_FRAMES: usize = 12;
const TICK_MS: u64 = 90;
/// One breathing cycle (idle mode) spans this many ticks (~3.2s).
const BREATH_TICKS: usize = 36;
/// Idle alpha swings between these two factors of the original icon alpha —
/// a visible but calm pulse.
const BREATH_MIN: f32 = 0.55;
const BREATH_MAX: f32 = 1.0;

/// Number of in-flight long-running commands; > 0 selects spinner mode.
static BUSY: AtomicUsize = AtomicUsize::new(0);

/// Decode the brand icon and start the animation thread. Before this is
/// called (or if the tray is missing), busy guards are harmless no-ops.
pub fn init(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || run_animation(app));
}

/// RAII busy marker held by long-running commands for their whole lifetime,
/// so the counter stays balanced on every exit path — including `?` errors.
pub struct BusyGuard(());

/// Mark a long-running task as active; the spinner shows while any guard lives.
pub fn busy() -> BusyGuard {
    BUSY.fetch_add(1, Ordering::AcqRel);
    BusyGuard(())
}

impl Drop for BusyGuard {
    // The animation thread notices the 0 count on its next tick and falls
    // back to the idle breathing loop; nothing else to do here.
    fn drop(&mut self) {
        BUSY.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The two animation states; template mode is toggled only on transitions.
#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Idle,
    Busy,
}

/// Persistent frame loop. TrayIcon methods dispatch to the main thread
/// internally, so a plain background thread with sleep is safe here.
fn run_animation(app: AppHandle) {
    let Some(tray) = app.tray_by_id("mole-tray") else {
        return;
    };
    // Base icon decoded once; breathing frames are derived from this buffer.
    let Ok(base) = Image::from_bytes(include_bytes!("../icons/icon.png")) else {
        return;
    };
    let (base_rgba, w, h) = (base.rgba().to_vec(), base.width(), base.height());

    let mut mode = Mode::Idle;
    let mut spin = 0usize; // spinner frame index
    let mut breath = 0usize; // breathing phase tick
    loop {
        let busy = BUSY.load(Ordering::Acquire) > 0;
        match (busy, mode) {
            (true, Mode::Idle) => {
                // Grace period: a command that finishes this fast never
                // flips the icon, so cache hits cause no spinner flash.
                std::thread::sleep(Duration::from_millis(150));
                if BUSY.load(Ordering::Acquire) > 0 {
                    mode = Mode::Busy;
                    spin = 0;
                    let _ = tray.set_icon_as_template(true);
                }
            }
            (true, Mode::Busy) => {
                let frames = spinner_frames();
                let _ = tray.set_icon(Some(Image::new(
                    &frames[spin % SPIN_FRAMES],
                    SPIN_SIZE,
                    SPIN_SIZE,
                )));
                spin += 1;
                std::thread::sleep(Duration::from_millis(TICK_MS));
            }
            (false, Mode::Busy) => {
                mode = Mode::Idle;
                let _ = tray.set_icon_as_template(false);
            }
            (false, Mode::Idle) => {
                let phase = breath as f32 / BREATH_TICKS as f32 * std::f32::consts::TAU;
                let factor = BREATH_MIN + (BREATH_MAX - BREATH_MIN) * (0.5 + 0.5 * phase.sin());
                let _ = tray.set_icon(Some(Image::new_owned(
                    scaled_alpha(&base_rgba, factor),
                    w,
                    h,
                )));
                breath = (breath + 1) % BREATH_TICKS;
                std::thread::sleep(Duration::from_millis(TICK_MS));
            }
        }
    }
}

/// The base icon's RGBA with every alpha value scaled by `factor` — the
/// breathing effect, computed fresh per tick (1MB memcpy-scale, negligible).
fn scaled_alpha(base: &[u8], factor: f32) -> Vec<u8> {
    let mut out = base.to_vec();
    for px in out.chunks_exact_mut(4) {
        px[3] = (px[3] as f32 * factor) as u8;
    }
    out
}

/// Lazily rendered spinner frame buffers, one per rotation step.
fn spinner_frames() -> &'static [Vec<u8>] {
    static BUF: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    BUF.get_or_init(|| (0..SPIN_FRAMES).map(render_spin_frame).collect())
}

/// Draw one spinner frame: an anti-aliased ring in template black, with alpha
/// fading from the rotating head (opaque) along ~300° of tail to transparent —
/// the classic macOS activity-spinner look.
fn render_spin_frame(index: usize) -> Vec<u8> {
    let center = SPIN_SIZE as f32 / 2.0;
    let ring_radius = 15.0; // leaves a ~7px margin on the 44px canvas
    let half_stroke = 3.0;
    let tail = std::f32::consts::TAU * 0.83; // arc length; the rest is the gap
    let head = index as f32 / SPIN_FRAMES as f32 * std::f32::consts::TAU;

    let mut rgba = vec![0u8; (SPIN_SIZE * SPIN_SIZE * 4) as usize];
    for y in 0..SPIN_SIZE {
        for x in 0..SPIN_SIZE {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            // Radial coverage: 1 inside the stroke, 1px linear AA edge.
            let radial =
                (half_stroke - ((dx * dx + dy * dy).sqrt() - ring_radius).abs()).clamp(0.0, 1.0);
            if radial <= 0.0 {
                continue;
            }
            // Angular distance behind the head, in rotation direction.
            let behind = (dy.atan2(dx) - head).rem_euclid(std::f32::consts::TAU);
            let angular = if behind < tail {
                1.0 - behind / tail
            } else {
                0.0
            };
            let a = (radial * angular * 255.0) as u8;
            // Template image: black pixels, shape carried by alpha alone.
            rgba[((y * SPIN_SIZE + x) * 4 + 3) as usize] = a;
        }
    }
    rgba
}

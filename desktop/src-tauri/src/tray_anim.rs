// Menu-bar tray state, driven by one persistent thread:
//   idle — a static menu-bar-sized brand icon with no main-thread refreshes;
//   busy — a pre-rendered axe-strike animation on the same white tile as the
//          idle icon while any command holds a BusyGuard.
// Keeping idle static is intentional: macOS converts every tray frame to PNG
// on the app's main thread, so a decorative idle loop would compete with the
// WebView display link.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::image::Image;
use tauri::AppHandle;

/// Runtime frame contract: 44px canvas (22pt menu bar @2x), 14 authored poses
/// at ~11 fps. Tauri decodes these embedded PNGs once and the loop only swaps
/// frames; it never rotates, scales, or paints the axe at runtime.
const TRAY_FRAME_SIZE: u32 = 44;
const AXE_FRAMES: usize = 14;
const TICK_MS: u64 = 90;

/// Number of in-flight long-running commands; > 0 selects axe animation mode.
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

/// Mark a long-running task as active; the axe animation shows while any guard lives.
pub fn busy() -> BusyGuard {
    BUSY.fetch_add(1, Ordering::AcqRel);
    BusyGuard(())
}

impl Drop for BusyGuard {
    // The animation thread notices the 0 count on its next tick and restores
    // the static brand icon; nothing else to do here.
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
    // The idle icon is already authored at menu-bar resolution. Runtime code
    // never scales, rotates, recolors, or repaints any tray image.
    let Ok(idle_icon) = Image::from_bytes(include_bytes!("../icons/tray-axe/idle.png")) else {
        return;
    };
    if idle_icon.width() != TRAY_FRAME_SIZE || idle_icon.height() != TRAY_FRAME_SIZE {
        return;
    }
    let idle_icon = idle_icon.to_owned();

    let mut mode = Mode::Idle;
    let mut frame = 0usize; // authored axe frame index
    loop {
        let busy = BUSY.load(Ordering::Acquire) > 0;
        match (busy, mode) {
            (true, Mode::Idle) => {
                // Grace period: a command that finishes this fast never
                // flips the icon, so cache hits cause no axe-animation flash.
                std::thread::sleep(Duration::from_millis(150));
                if BUSY.load(Ordering::Acquire) > 0 {
                    mode = Mode::Busy;
                    frame = 0;
                    // Authored frames carry their own white background and
                    // brand colors, so macOS must not template-tint them.
                    let _ = tray.set_icon_as_template(false);
                }
            }
            (true, Mode::Busy) => {
                let frames = axe_frames();
                let _ = tray.set_icon(Some(frames[frame % AXE_FRAMES].clone()));
                frame += 1;
                std::thread::sleep(Duration::from_millis(TICK_MS));
            }
            (false, Mode::Busy) => {
                mode = Mode::Idle;
                let _ = tray.set_icon_as_template(false);
                // Restore the colored brand once on the state transition.
                // The idle branch performs no further main-thread icon work.
                let _ = tray.set_icon(Some(idle_icon.clone()));
            }
            (false, Mode::Idle) => {
                // Poll only the atomic busy counter. In particular, do not
                // call `set_icon` here: on macOS that schedules PNG encoding
                // on the main thread and causes periodic WebView frame drops.
                std::thread::sleep(Duration::from_millis(TICK_MS));
            }
        }
    }
}

/// Decode the authored PNG sequence once. A single animated GIF cannot be
/// used here: Tauri's tray image API exposes RGBA pixels and macOS status-item
/// buttons display only one decoded image. Separate frames preserve the exact
/// asset-authored motion while keeping runtime work to frame selection.
fn axe_frames() -> &'static [Image<'static>] {
    static FRAMES: OnceLock<Vec<Image<'static>>> = OnceLock::new();
    static PNGS: [&[u8]; AXE_FRAMES] = [
        include_bytes!("../icons/tray-axe/axe-00.png"),
        include_bytes!("../icons/tray-axe/axe-01.png"),
        include_bytes!("../icons/tray-axe/axe-02.png"),
        include_bytes!("../icons/tray-axe/axe-03.png"),
        include_bytes!("../icons/tray-axe/axe-04.png"),
        include_bytes!("../icons/tray-axe/axe-05.png"),
        include_bytes!("../icons/tray-axe/axe-06.png"),
        include_bytes!("../icons/tray-axe/axe-07.png"),
        include_bytes!("../icons/tray-axe/axe-08.png"),
        include_bytes!("../icons/tray-axe/axe-09.png"),
        include_bytes!("../icons/tray-axe/axe-10.png"),
        include_bytes!("../icons/tray-axe/axe-11.png"),
        include_bytes!("../icons/tray-axe/axe-12.png"),
        include_bytes!("../icons/tray-axe/axe-13.png"),
    ];
    FRAMES.get_or_init(|| {
        PNGS.iter()
            .map(|png| {
                let frame = Image::from_bytes(png)
                    .expect("embedded tray axe frame must be a valid PNG")
                    .to_owned();
                assert_eq!(
                    (frame.width(), frame.height()),
                    (TRAY_FRAME_SIZE, TRAY_FRAME_SIZE),
                    "embedded tray axe frame must be 44x44"
                );
                frame
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_axe_frames_match_the_menu_bar_contract() {
        let frames = axe_frames();
        let idle = Image::from_bytes(include_bytes!("../icons/tray-axe/idle.png")).unwrap();

        assert_eq!(frames.len(), AXE_FRAMES);
        assert!(frames
            .iter()
            .all(|frame| frame.width() == TRAY_FRAME_SIZE && frame.height() == TRAY_FRAME_SIZE));
        assert_eq!(
            (idle.width(), idle.height()),
            (TRAY_FRAME_SIZE, TRAY_FRAME_SIZE)
        );
    }
}

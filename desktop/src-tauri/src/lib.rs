// Tauri app shell: command registration, shared state, the menu-bar tray, and
// the close-to-tray behavior. Commands stay thin — they translate IPC to
// mole-ops calls and map errors to stable codes.

mod commands;
mod dto;
mod error;
mod state;
mod tray_anim;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};

/// Resolve the invoking user's home once for every command.
fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string())
}

/// Close-to-tray flag: when set, closing the window hides it instead of
/// quitting (the tray keeps the app reachable). Toggled from settings.
pub static KEEP_IN_TRAY: AtomicBool = AtomicBool::new(false);

/// Show + focus the main window (tray click / menu navigation).
fn show_main(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Build the tray icon with its menu; menu ids map to route hashes.
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let items: &[(&str, &str)] = &[
        ("open", "打开 / Open"),
        ("#/clean", "清理 Clean"),
        ("#/apps", "软件 Apps"),
        ("#/optimize", "优化 Optimize"),
        ("#/analyze", "分析 Analyze"),
        ("#/status", "状态 Status"),
        ("#/settings", "设置 Settings"),
        ("quit", "退出 Quit"),
    ];
    let mut builder = MenuBuilder::new(app);
    for (id, label) in items {
        builder = builder.item(&MenuItemBuilder::with_id(*id, *label).build(app)?);
        if *id == "open" || *id == "#/status" {
            builder = builder.separator();
        }
    }
    let menu = builder.build()?;

    // Use the authored 44px tray asset directly. The 512px application icon
    // belongs to app packaging; sending it through the status-item API would
    // make macOS resize and PNG-encode far more pixels than it can display.
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-axe/idle.png"))?;
    TrayIconBuilder::with_id("mole-tray")
        .icon(icon)
        .icon_as_template(false)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            match id {
                "quit" => app.exit(0),
                "open" => show_main(app),
                hash => {
                    show_main(app);
                    let _ = app.emit("mole-nav", hash.to_string());
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            // Left click opens the window (menu stays on right click).
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Warm the installed-app inventory at startup: seed the cache from the
/// persisted file of the previous launch (instant Apps view, slightly stale
/// sizes), then run a fresh full scan so the seed is replaced within seconds.
/// The scan itself runs on bounded utility-QoS workers, so "full speed" stays
/// off the performance cores and never reads as UI jank.
fn warm_apps_inventory(app: &tauri::App) {
    let inventory = app.state::<state::AppState>().apps.clone();
    let home = home();
    tauri::async_runtime::spawn_blocking(move || {
        inventory.seed_from_disk(&home);
        let probes = mole_core::probes::SystemProbes::new();
        // Warmup is never cancelled, so its result is always cacheable.
        let cancel: mole_ops::scanutil::CancelFlag = std::sync::Arc::new(AtomicBool::new(false));
        inventory.refresh_if_idle(|| {
            (
                mole_ops::uninstall::inventory(&home, &probes, &cancel),
                true,
            )
        });
    });
}

/// Warm the status sampler at startup on a one-shot utility-QoS thread: the
/// first snapshot pays a process baseline, a ~1s cached system_profiler call,
/// and a 200ms delta window. Paying that here makes the dashboard's first
/// fetch fast and its CPU deltas real. A dedicated thread (not spawn_blocking)
/// so the QoS downgrade cannot stick to a reused pool thread.
fn warm_status_sampler() {
    std::thread::spawn(|| {
        mole_ops::scanutil::set_scan_thread_qos();
        let _ = mole_ops::status::snapshot();
    });
}

/// Build and run the Tauri application.
pub fn run() {
    tauri::Builder::default()
        .manage(state::AppState::default())
        .setup(|app| {
            setup_tray(app)?;
            tray_anim::init(app.handle());
            warm_apps_inventory(app);
            warm_status_sampler();
            Ok(())
        })
        .on_page_load(|webview, payload| {
            // PageLoadEvent::Started is the reliable backend-side signal for
            // Vite reloads and normal navigations. The old JavaScript module
            // (and its in-memory task id) is already gone at this point.
            if webview.label() == "main"
                && payload.event() == tauri::webview::PageLoadEvent::Started
            {
                webview.state::<state::AppState>().cancel_analyze();
            }
        })
        .on_web_content_process_terminate(|webview| {
            if webview.label() == "main" {
                webview.state::<state::AppState>().cancel_analyze();
            }
        })
        .on_window_event(|window, event| {
            // Whatever closed the helper (✕, drop, grant, main closing), the
            // settings page re-reads the permission once.
            if window.label() == "fda-helper" && matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(main) = window.app_handle().get_webview_window("main") {
                    let _ = main.emit("fda-helper-closed", ());
                }
            }
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.state::<state::AppState>().cancel_analyze();
                // The drag helper has no life of its own.
                if let Some(helper) = window.app_handle().get_webview_window("fda-helper") {
                    let _ = helper.close();
                }
            }
            // Close-to-tray: hide instead of quitting when enabled.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if KEEP_IN_TRAY.load(Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .plugin(tauri_plugin_drag::init())
        // Analyze's "pick a folder" scope. Only the open dialog is granted; the
        // picker returns a path string and grants the webview no fs access.
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::app_meta,
            commands::status_snapshot,
            commands::process_detail,
            commands::signal_process,
            commands::whitelist_get,
            commands::whitelist_set,
            commands::purge_paths_get,
            commands::touchid_status,
            commands::analyze_scan,
            commands::plan_delete_paths,
            commands::plan_clean,
            commands::list_apps,
            commands::app_icon,
            commands::refresh_app_cache,
            commands::preview_uninstall,
            commands::cached_app_updates,
            commands::list_app_updates,
            commands::run_app_updates,
            commands::set_app_update_ignored,
            commands::list_login_items,
            commands::list_embedded_login_items,
            commands::set_login_item_enabled,
            commands::plan_uninstall,
            commands::plan_purge,
            commands::plan_docker,
            commands::execute_docker,
            commands::plan_installer,
            commands::execute_plan,
            commands::cancel_task,
            commands::reveal_in_finder,
            commands::list_optimize_tasks,
            commands::run_optimize,
            commands::fda_status,
            commands::open_fda_settings,
            commands::fda_helper_show,
            commands::fda_helper_hide,
            commands::fda_drag_source,
            commands::autostart_get,
            commands::autostart_set,
            commands::tray_set_visible,
            commands::set_keep_in_tray,
        ])
        .run(tauri::generate_context!())
        .expect("error while running mole desktop");
}

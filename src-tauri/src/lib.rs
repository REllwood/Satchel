//! Satchel Tauri shell. Thin command layer over `satchel-core`; all real logic
//! lives in the core. See `commands.rs` for the command surface.

mod commands;
#[cfg(test)]
mod ipc_tests;
mod state;

use std::ffi::OsString;
use std::path::PathBuf;

use satchel_core::{embed, vault};
use state::AppState;
use tauri::Manager;

/// Returns the core crate version (used by the status bar / health checks).
#[tauri::command]
fn core_version() -> String {
    satchel_core::version().to_string()
}

/// Locate the bundled embedding model: the app's resource dir in installed
/// builds, falling back to the core's search (dev checkout, env override).
fn resolve_model_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("models").join(embed::DEFAULT_MODEL))
        .filter(|dir| dir.join("model.onnx").is_file())
        .unwrap_or_else(embed::default_model_dir)
}

/// `Satchel mcp [--vault <folder>]`: serve MCP on stdio for Claude Code / Codex
/// instead of opening a window. Returns the process exit code.
pub fn run_mcp(args: &[OsString]) -> i32 {
    let mut vault_arg = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--vault" {
            vault_arg = it.next().map(PathBuf::from);
        } else if let Some(path) = arg.to_str().and_then(|a| a.strip_prefix("--vault=")) {
            vault_arg = Some(PathBuf::from(path));
        } else {
            eprintln!("satchel mcp: ignoring unknown argument {}", arg.to_string_lossy());
        }
    }
    let Some(root) = vault_arg.or_else(|| std::env::var_os("SATCHEL_VAULT").map(PathBuf::from))
    else {
        eprintln!("usage: Satchel mcp --vault <notes folder>");
        return 2;
    };
    if !root.is_dir() {
        eprintln!("satchel mcp: notes folder not found: {}", root.display());
        return 2;
    }
    vault::allow_cloud_downloads();
    match satchel_core::mcp::run(&root) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("satchel mcp: {e:#}");
            1
        }
    }
}

/// The macOS menu bar: the standard items, except that Quit (⌘Q) goes
/// through `request_exit`, so unsaved edits are saved first. The standard Quit
/// item ends the app at once, with no chance to save.
#[cfg(target_os = "macos")]
fn app_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
    let sep = || PredefinedMenuItem::separator(app);
    let quit = MenuItem::with_id(app, QUIT_MENU_ID, "Quit Satchel", true, Some("CmdOrCtrl+Q"))?;
    let app_menu = Submenu::with_items(
        app,
        "Satchel",
        true,
        &[
            &PredefinedMenuItem::about(app, None, Some(AboutMetadata::default()))?,
            &sep()?,
            &PredefinedMenuItem::services(app, None)?,
            &sep()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &sep()?,
            &quit,
        ],
    )?;
    let file = Submenu::with_items(app, "File", true, &[&PredefinedMenuItem::close_window(app, None)?])?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &sep()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(app, "View", true, &[&PredefinedMenuItem::fullscreen(app, None)?])?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &sep()?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &file, &edit, &view, &window])
}

#[cfg(target_os = "macos")]
const QUIT_MENU_ID: &str = "quit";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    vault::allow_cloud_downloads();
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.menu(app_menu).on_menu_event(|app, event| {
        if event.id() == QUIT_MENU_ID {
            commands::request_exit(app);
        }
    });
    builder
        .plugin(tauri_plugin_dialog::init())
        // Closing the window quits the app. Give it a moment to save unsaved
        // edits first.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let exiting = window.state::<AppState>().exiting.load(std::sync::atomic::Ordering::SeqCst);
                if !exiting {
                    api.prevent_close();
                    commands::request_exit(window.app_handle());
                }
            }
        })
        .setup(|app| {
            let model_dir = resolve_model_dir(app.handle());
            app.manage(AppState::new(model_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_version,
            commands::detect_sync_locations,
            commands::create_vault,
            commands::open_vault,
            commands::current_vault,
            commands::close_vault,
            commands::get_last_vault,
            commands::reindex,
            commands::embed_vault,
            commands::add_agent_docs,
            commands::agent_setup,
            commands::list_notes,
            commands::list_folders,
            commands::create_folder,
            commands::rename_folder,
            commands::delete_folder,
            commands::read_note,
            commands::write_note,
            commands::create_note,
            commands::rename_note,
            commands::delete_note,
            commands::save_attachment,
            commands::read_attachment,
            commands::list_canvases,
            commands::read_canvas,
            commands::write_canvas,
            commands::create_canvas,
            commands::search_fulltext,
            commands::search_semantic,
            commands::search_hybrid,
            commands::run_query,
            commands::get_backlinks,
            commands::get_graph,
            commands::get_tags,
            commands::notes_for_tag,
            commands::confirm_exit,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

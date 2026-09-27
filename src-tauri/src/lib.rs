//! Satchel Tauri shell. Thin command layer over `satchel-core`; all real logic
//! lives in the core. See `commands.rs` for the command surface.

mod commands;
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
        }
    }
    let Some(root) = vault_arg.or_else(|| std::env::var_os("SATCHEL_VAULT").map(PathBuf::from))
    else {
        eprintln!("usage: Satchel mcp --vault <notes folder>");
        return 2;
    };
    vault::allow_cloud_downloads();
    match satchel_core::mcp::run(&root) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("satchel mcp: {e:#}");
            1
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    vault::allow_cloud_downloads();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

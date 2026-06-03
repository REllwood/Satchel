//! MyNote Tauri shell. Thin command layer over `mynote-core`; all real logic
//! lives in the core. See `commands.rs` for the command surface.

mod commands;
mod state;

use state::AppState;

/// Returns the core crate version (used by the status bar / health checks).
#[tauri::command]
fn core_version() -> String {
    mynote_core::version().to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            core_version,
            commands::open_vault,
            commands::current_vault,
            commands::close_vault,
            commands::get_last_vault,
            commands::reindex,
            commands::embed_vault,
            commands::list_notes,
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

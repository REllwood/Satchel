//! Calls commands through Tauri's real IPC layer with the argument names the
//! frontend sends (`src/lib/api.ts`). Tauri matches arguments by their
//! camelCase names, so a mismatch here breaks the command in the app even
//! though every Rust-level test passes.

use std::path::PathBuf;

use satchel_core::{db, index, query};
use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

use crate::commands;
use crate::state::{AppState, Vault};

fn send(
    webview: &WebviewWindow<MockRuntime>,
    cmd: &str,
    body: InvokeBody,
    headers: tauri::http::HeaderMap,
) -> Result<InvokeResponseBody, Value> {
    let request = InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: "tauri://localhost".parse().unwrap(),
        body,
        headers,
        invoke_key: INVOKE_KEY.to_string(),
    };
    get_ipc_response(webview, request)
}

fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    send(webview, cmd, InvokeBody::Json(args), Default::default()).map(|body| body.deserialize().unwrap())
}

#[test]
fn commands_accept_the_frontends_argument_names() {
    let vault_dir = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let root: PathBuf = vault_dir.path().canonicalize().unwrap();
    std::fs::write(root.join("a.md"), "# A\n\nsee [[b]]").unwrap();
    std::fs::write(root.join("b.md"), "# B\n").unwrap();

    let db_path = index_dir.path().join("index.db");
    let conn = db::open(&db_path).unwrap();
    index::reindex_all(&conn, &root).unwrap();
    let state = AppState::new(PathBuf::from("no-model"));
    *state.vault.lock().unwrap() = Some(Vault {
        root: root.clone(),
        conn,
        query_conn: query::open_query_connection(&db_path).unwrap(),
        watch: None,
    });

    let app = mock_builder()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::list_notes,
            commands::read_note,
            commands::write_note,
            commands::create_note,
            commands::rename_note,
            commands::create_folder,
            commands::save_attachment,
            commands::read_attachment,
            commands::create_canvas,
            commands::read_canvas,
            commands::write_canvas,
            commands::run_query,
            commands::get_backlinks,
        ])
        .build(mock_context(noop_assets()))
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();

    let note = invoke(&webview, "read_note", json!({ "relPath": "a.md" })).unwrap();
    assert_eq!(note["content"], "# A\n\nsee [[b]]");

    invoke(&webview, "write_note", json!({ "relPath": "a.md", "content": "# A\n\nedited" })).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("a.md")).unwrap(), "# A\n\nedited");

    let created = invoke(&webview, "create_note", json!({ "relPath": "c.md", "content": null }));
    assert_eq!(created.unwrap(), "c.md");
    let rewritten = invoke(&webview, "rename_note", json!({ "from": "c.md", "to": "d.md" })).unwrap();
    assert_eq!(rewritten, 0);
    assert!(root.join("d.md").is_file());

    invoke(&webview, "create_folder", json!({ "relPath": "Projects" })).unwrap();
    assert!(root.join("Projects").is_dir());

    // Attachments travel as raw bytes, not JSON arrays of numbers.
    let mut headers = tauri::http::HeaderMap::new();
    headers.insert("x-satchel-ext", "png".parse().unwrap());
    let saved = send(&webview, "save_attachment", InvokeBody::Raw(vec![1, 2, 3]), headers).unwrap();
    let saved: String = saved.deserialize().unwrap();
    assert!(saved.starts_with("attachments/") && saved.ends_with(".png"), "{saved}");
    let name = saved.trim_start_matches("attachments/");
    let read = send(
        &webview,
        "read_attachment",
        InvokeBody::Json(json!({ "target": name, "fromNote": "a.md" })),
        Default::default(),
    )
    .unwrap();
    match read {
        InvokeResponseBody::Raw(bytes) => assert_eq!(bytes, vec![1, 2, 3]),
        other => panic!("expected raw bytes, got {other:?}"),
    }

    let board = invoke(&webview, "create_canvas", json!({ "relPath": "Board" })).unwrap();
    assert_eq!(board, "Board.canvas");
    let canvas = json!({ "nodes": [], "edges": [] });
    invoke(&webview, "write_canvas", json!({ "relPath": "Board.canvas", "canvas": canvas })).unwrap();
    invoke(&webview, "read_canvas", json!({ "relPath": "Board.canvas" })).unwrap();

    let rows = invoke(
        &webview,
        "run_query",
        json!({ "sql": "SELECT rel_path FROM notes ORDER BY rel_path", "maxRows": 10 }),
    )
    .unwrap();
    assert_eq!(rows["rows"], json!([["a.md"], ["b.md"], ["d.md"]]));

    let backlinks = invoke(&webview, "get_backlinks", json!({ "relPath": "b.md" })).unwrap();
    assert!(backlinks.is_array());

    // The snake_case spelling v0.1.0 sent is rejected outright.
    let err = invoke(&webview, "read_note", json!({ "rel_path": "a.md" })).unwrap_err();
    assert!(err.to_string().contains("relPath"), "{err}");
}

//! End-to-end CLI integration tests — run the real `satchel` binary against a
//! temp vault. Exercises the same core path the desktop app uses.

use std::fs;
use std::process::Command;

fn satchel(vault: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_satchel"));
    cmd.arg("--vault").arg(vault);
    // Keep the per-machine index inside the test's temp dir (a hidden folder
    // the scanner ignores), never in the user's real app-data folder.
    cmd.env("SATCHEL_DATA_DIR", vault.join(".satchel-test-data"));
    cmd
}

#[test]
fn init_search_query_new_flow() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("alpha.md"), "# Alpha\n\nthe zebra crossed the road #animals").unwrap();

    // init: builds index + writes AGENTS.md/CLAUDE.md
    let out = satchel(root).arg("init").output().unwrap();
    assert!(out.status.success(), "init failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(root.join("AGENTS.md").exists());
    assert!(root.join("CLAUDE.md").exists());

    // search finds the note by a distinctive word
    let out = satchel(root).args(["search", "zebra"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("alpha.md"));

    // read-only query returns rows
    let out = satchel(root)
        .args(["query", "SELECT count(*) AS n FROM notes", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"n\""));

    // a write query is rejected (sandbox) — non-zero exit
    let out = satchel(root).args(["query", "DELETE FROM notes"]).output().unwrap();
    assert!(!out.status.success(), "write query should be rejected");

    // create a new note
    let out = satchel(root).args(["new", "My Idea"]).output().unwrap();
    assert!(out.status.success());
    assert!(root.join("My Idea.md").exists());
}

#[test]
fn mcp_lists_tools_and_searches() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("note.md"), "# Note\n\npineapple express").unwrap();

    use std::io::Write;
    let mut child = satchel(root)
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        // initialize, tools/list, tools/call(search_notes)
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05"}}}}"#).unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"search_notes","arguments":{{"query":"pineapple"}}}}}}"#).unwrap();
    } // drop stdin → EOF → server exits
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"serverInfo\""), "initialize response missing: {stdout}");
    assert!(stdout.contains("search_notes"), "tools/list missing tools");
    assert!(stdout.contains("note.md"), "search_notes did not return the note");
}

/// Send `lines` to `satchel mcp` and return one parsed JSON message per line
/// of output.
fn mcp_exchange(root: &std::path::Path, lines: &[&[u8]]) -> Vec<serde_json::Value> {
    use std::io::Write;
    let mut child = satchel(root)
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for line in lines {
            stdin.write_all(line).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn tool_text(response: &serde_json::Value) -> String {
    response["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

#[test]
fn mcp_answers_bad_input_and_negotiates_its_version() {
    let dir = tempfile::tempdir().unwrap();
    let replies = mcp_exchange(
        dir.path(),
        &[
            b"{not json",
            b"[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]",
            b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\",\"params\":{\"x\":\"\xff\"}}",
            br#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#,
            br#"{"jsonrpc":"2.0","id":4,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        ],
    );
    assert_eq!(replies.len(), 5, "{replies:?}");
    assert_eq!(replies[0]["error"]["code"], -32700);
    assert_eq!(replies[1]["error"]["code"], -32600);
    assert_eq!(replies[2]["id"], 2, "invalid UTF-8 does not end the server");
    assert_eq!(replies[3]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[4]["result"]["protocolVersion"], "2025-03-26");
}

#[test]
fn mcp_reads_only_notes_inside_the_vault() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("note.md"), "# Note").unwrap();
    fs::write(root.join(".env"), "SECRET=1").unwrap();
    fs::write(outside.path().join("secret.md"), "SECRET=2").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), root.join("linked")).unwrap();

    let call = |id: u32, path: &str| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"read_note","arguments":{{"path":"{path}"}}}}}}"#
        )
    };
    let lines = [call(1, "note.md"), call(2, ".env"), call(3, "linked/secret.md")];
    let lines: Vec<&[u8]> = lines.iter().map(|l| l.as_bytes()).collect();
    let replies = mcp_exchange(root, &lines);
    assert_eq!(tool_text(&replies[0]), "# Note");
    for reply in &replies[1..] {
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        assert!(!tool_text(reply).contains("SECRET"));
    }
}

#[test]
fn mcp_creates_visible_notes_in_folders_and_pages_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let replies = mcp_exchange(
        root,
        &[
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_note","arguments":{"title":".NET tips: async?"}}}"#,
            br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_note","arguments":{"title":"Plan","folder":"Projects/2026"}}}"#,
            br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_note","arguments":{"title":"Plan","folder":"Projects/2026"}}}"#,
            br#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_notes","arguments":{"limit":1}}}"#,
        ],
    );
    assert_eq!(tool_text(&replies[0]), "NET tips- async-.md");
    assert_eq!(tool_text(&replies[1]), "Projects/2026/Plan.md");
    assert_eq!(replies[2]["result"]["isError"], true, "never overwrites");
    let page: serde_json::Value = serde_json::from_str(&tool_text(&replies[3])).unwrap();
    assert_eq!(page["total"], 2);
    assert_eq!(page["next_offset"], 1);
}

#[test]
fn mcp_edits_and_appends_to_existing_notes_only() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("Projects")).unwrap();
    fs::write(root.join("Projects/Plan.md"), "# Plan\r\n\r\n- milk\r\n- eggs\r\n").unwrap();
    fs::write(root.join("board.canvas"), "{}").unwrap();
    fs::create_dir(root.join(".obsidian")).unwrap();
    fs::write(root.join(".obsidian/notes.md"), "- milk").unwrap();
    fs::write(outside.path().join("secret.md"), "- milk").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), root.join("linked")).unwrap();

    let call = |id: u32, name: &str, args: serde_json::Value| {
        serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": args }
        })
        .to_string()
    };
    let edit = |path: &str| serde_json::json!({ "path": path, "old_text": "- milk", "new_text": "- oat milk" });
    let lines = [
        call(1, "edit_note", serde_json::json!({ "path": "Projects/Plan.md", "old_text": "- milk\n- eggs", "new_text": "- milk\n- bread" })),
        call(2, "append_to_note", serde_json::json!({ "path": "./Projects/Plan.md", "content": "- zucchini" })),
        call(3, "search_notes", serde_json::json!({ "query": "zucchini" })),
        call(4, "edit_note", serde_json::json!({ "path": "Projects/Plan.md", "old_text": "- tea", "new_text": "x" })),
        call(5, "edit_note", edit("projects/plan.md")),
        call(6, "edit_note", edit("Projects/Missing.md")),
        call(7, "edit_note", edit("board.canvas")),
        call(8, "edit_note", edit(".obsidian/notes.md")),
        call(9, "edit_note", edit("linked/secret.md")),
        call(10, "append_to_note", serde_json::json!({ "path": "../secret.md", "content": "x" })),
    ];
    let lines: Vec<&[u8]> = lines.iter().map(|l| l.as_bytes()).collect();
    let replies = mcp_exchange(root, &lines);

    assert_eq!(tool_text(&replies[0]), "Edited Projects/Plan.md: 1 replacement.");
    assert_eq!(tool_text(&replies[1]), "Added to the end of Projects/Plan.md.");
    assert_eq!(
        fs::read_to_string(root.join("Projects/Plan.md")).unwrap(),
        "# Plan\r\n\r\n- milk\r\n- bread\r\n- zucchini\r\n",
        "edits keep the note's Windows line endings"
    );
    assert!(tool_text(&replies[2]).contains("Projects/Plan.md"), "the index sees the edit");
    for reply in &replies[3..] {
        assert_eq!(reply["result"]["isError"], true, "{reply}");
    }
    assert!(tool_text(&replies[3]).contains("not found in the note"));
    assert_eq!(fs::read_to_string(root.join("board.canvas")).unwrap(), "{}");
    assert_eq!(fs::read_to_string(root.join(".obsidian/notes.md")).unwrap(), "- milk");
    assert_eq!(fs::read_to_string(outside.path().join("secret.md")).unwrap(), "- milk");
    assert!(!root.join("Projects/Missing.md").exists());
}

#[test]
fn a_missing_vault_folder_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("not mounted");
    for args in [&["index"][..], &["search", "x"], &["mcp"]] {
        let out = satchel(&missing).args(args).output().unwrap();
        assert!(!out.status.success(), "{args:?} should fail");
    }
    assert!(!missing.exists(), "nothing was created");
}

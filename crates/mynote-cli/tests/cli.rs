//! End-to-end CLI integration tests — run the real `mynote` binary against a
//! temp vault. Exercises the same core path the desktop app uses.

use std::fs;
use std::process::Command;

fn mynote(vault: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mynote"));
    cmd.arg("--vault").arg(vault);
    cmd
}

#[test]
fn init_search_query_new_flow() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("alpha.md"), "# Alpha\n\nthe zebra crossed the road #animals").unwrap();

    // init: builds index + writes AGENTS.md/CLAUDE.md
    let out = mynote(root).arg("init").output().unwrap();
    assert!(out.status.success(), "init failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(root.join("AGENTS.md").exists());
    assert!(root.join("CLAUDE.md").exists());

    // search finds the note by a distinctive word
    let out = mynote(root).args(["search", "zebra"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("alpha.md"));

    // read-only query returns rows
    let out = mynote(root)
        .args(["query", "SELECT count(*) AS n FROM notes", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"n\""));

    // a write query is rejected (sandbox) — non-zero exit
    let out = mynote(root).args(["query", "DELETE FROM notes"]).output().unwrap();
    assert!(!out.status.success(), "write query should be rejected");

    // create a new note
    let out = mynote(root).args(["new", "My Idea"]).output().unwrap();
    assert!(out.status.success());
    assert!(root.join("My Idea.md").exists());
}

#[test]
fn mcp_lists_tools_and_searches() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("note.md"), "# Note\n\npineapple express").unwrap();

    use std::io::Write;
    let mut child = mynote(root)
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

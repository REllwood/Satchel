// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The app binary doubles as the MCP server for AI tools, so an installed
    // Satchel is all Claude Code / Codex need: `Satchel mcp --vault <folder>`.
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "mcp") {
        std::process::exit(satchel_app_lib::run_mcp(&args[1..]));
    }
    satchel_app_lib::run()
}

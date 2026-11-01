// Typed bindings for the Rust command surface. The UI only talks to these.
import { invoke } from "@tauri-apps/api/core";

export type HitSource = "full_text" | "semantic" | "hybrid";

export type Provider =
  | "icloud"
  | "google_drive"
  | "dropbox"
  | "one_drive"
  | "box"
  | "local";

export interface VaultInfo {
  root: string;
  note_count: number;
  provider: Provider;
  /** e.g. "iCloud Drive", "This computer" */
  provider_name: string;
  /** The welcome note, when `create_vault` just wrote one into a new vault. */
  welcome_note: string | null;
}

/** A place a vault can live (detected cloud folder or local). */
export interface SyncLocation {
  provider: Provider;
  label: string;
  vault_path: string;
  vault_exists: boolean;
}

/** Background semantic-indexer progress (event `embed-status`). */
export interface EmbedStatus {
  running: boolean;
  done: number;
  total: number;
  error: string | null;
}
export interface NoteMeta {
  id: number;
  rel_path: string;
  title: string;
  mtime: number;
}
export interface NoteContent {
  rel_path: string;
  content: string;
}
export interface SearchHit {
  note_id: number;
  rel_path: string;
  title: string;
  snippet: string;
  score: number;
  source: HitSource;
}
export type Cell = string | number | boolean | null;
export interface QueryResult {
  columns: string[];
  rows: Cell[][];
  truncated: boolean;
}
export interface IndexStats {
  added: number;
  updated: number;
  unchanged: number;
  removed: number;
  /** Notes found at a new path (moved or renamed outside the app). */
  moved: number;
  /** Notes that could not be read this time, left as they were. */
  skipped: number;
  total: number;
}
export interface Backlink {
  rel_path: string;
  title: string;
  unlinked: boolean;
}
export interface GraphNode {
  id: string;
  title: string;
}
export interface GraphEdge {
  source: string;
  target: string;
}
export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
}
export interface TagCount {
  tag: string;
  count: number;
}

// Tauri matches command arguments by their camelCase names (`rel_path` in Rust
// is `relPath` here). api.test.ts checks every call follows this.

// ---- vault lifecycle -------------------------------------------------------
export const detectSyncLocations = () =>
  invoke<SyncLocation[]>("detect_sync_locations");
export const createVault = (path: string) =>
  invoke<VaultInfo>("create_vault", { path });
export const openVault = (path: string) =>
  invoke<VaultInfo>("open_vault", { path });
export const currentVault = () => invoke<VaultInfo | null>("current_vault");
export const closeVault = () => invoke<void>("close_vault");
export const getLastVault = () => invoke<string | null>("get_last_vault");
export const reindex = () => invoke<IndexStats>("reindex");
/** Rebuild all embeddings in the background (progress via `embed-status`). */
export const embedVault = () => invoke<void>("embed_vault");
export const addAgentDocs = () => invoke<void>("add_agent_docs");

/**
 * Emitted when the window is closing or the app is quitting. Save pending
 * edits, then call `confirmExit`. The app quits on its own after a few
 * seconds if it is not confirmed.
 */
export const APP_EXIT_REQUESTED = "app-exit-requested";
export const confirmExit = () => invoke<void>("confirm_exit");

/** Ready-to-paste MCP setup for AI tools, pointing at this app + vault. */
export interface AgentSetup {
  claude_code: string;
  codex: string;
  /** Set when the app is running from a location that may not last (a disk image). */
  warning: string | null;
}
export const agentSetup = () => invoke<AgentSetup>("agent_setup");

// ---- folders (notebooks) ---------------------------------------------------
export const listFolders = () => invoke<string[]>("list_folders");
export const createFolder = (relPath: string) =>
  invoke<void>("create_folder", { relPath });
/** Returns how many other notes had links to the moved notes rewritten. */
export const renameFolder = (from: string, to: string) =>
  invoke<number>("rename_folder", { from, to });
export const deleteFolder = (relPath: string) =>
  invoke<void>("delete_folder", { relPath });

// ---- notes -----------------------------------------------------------------
export const listNotes = () => invoke<NoteMeta[]>("list_notes");
export const readNote = (relPath: string) =>
  invoke<NoteContent>("read_note", { relPath });
export const writeNote = (relPath: string, content: string) =>
  invoke<void>("write_note", { relPath, content });
export const createNote = (relPath: string, content?: string) =>
  invoke<string>("create_note", { relPath, content: content ?? null });
/** Returns how many other notes had links to it rewritten. */
export const renameNote = (from: string, to: string) =>
  invoke<number>("rename_note", { from, to });
export const deleteNote = (relPath: string) =>
  invoke<void>("delete_note", { relPath });

// ---- attachments -----------------------------------------------------------
/** Store a file in the vault's attachments/ folder. Returns its vault path. */
export const saveAttachment = (bytes: Uint8Array, ext: string) =>
  invoke<string>("save_attachment", bytes, { headers: { "x-satchel-ext": ext } });
/**
 * Read an embedded file. `target` is the path as written in the note. It is
 * resolved next to `fromNote` first, then from the vault root, then (for a
 * bare file name) anywhere in the vault.
 */
export const readAttachment = (target: string, fromNote: string | null) =>
  invoke<ArrayBuffer>("read_attachment", { target, fromNote });

// ---- canvas (JSONCanvas) ---------------------------------------------------
// Canvas objects keep any fields Satchel does not use, so saving a canvas
// made in another app does not drop them.
export interface CanvasNode {
  [key: string]: unknown;
  id: string;
  type: string; // text | file | link | group
  x: number;
  y: number;
  width: number;
  height: number;
  color?: string;
  text?: string;
  file?: string;
  url?: string;
  label?: string;
}
export interface CanvasEdge {
  [key: string]: unknown;
  id: string;
  fromNode: string;
  toNode: string;
  fromSide?: string;
  toSide?: string;
  color?: string;
  label?: string;
}
export interface Canvas {
  [key: string]: unknown;
  nodes: CanvasNode[];
  edges: CanvasEdge[];
}
export const listCanvases = () => invoke<string[]>("list_canvases");
export const readCanvas = (relPath: string) =>
  invoke<Canvas>("read_canvas", { relPath });
export const writeCanvas = (relPath: string, canvas: Canvas) =>
  invoke<void>("write_canvas", { relPath, canvas });
export const createCanvas = (relPath: string) =>
  invoke<string>("create_canvas", { relPath });

// ---- search & query --------------------------------------------------------
export const searchFulltext = (query: string, limit = 30) =>
  invoke<SearchHit[]>("search_fulltext", { query, limit });
export const searchSemantic = (query: string, limit = 30) =>
  invoke<SearchHit[]>("search_semantic", { query, limit });
export const searchHybrid = (query: string, limit = 30) =>
  invoke<SearchHit[]>("search_hybrid", { query, limit });
export const runQuery = (sql: string, maxRows = 500) =>
  invoke<QueryResult>("run_query", { sql, maxRows });

// ---- links, graph, tags ----------------------------------------------------
export const getBacklinks = (relPath: string) =>
  invoke<Backlink[]>("get_backlinks", { relPath });
export const getGraph = () => invoke<GraphData>("get_graph");
export const getTags = () => invoke<TagCount[]>("get_tags");
export const notesForTag = (tag: string) =>
  invoke<string[]>("notes_for_tag", { tag });

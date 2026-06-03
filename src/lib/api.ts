// Typed bindings for the Rust command surface. The UI only talks to these.
// Arg keys are snake_case to match the Rust command parameter names exactly.
import { invoke } from "@tauri-apps/api/core";

export type HitSource = "full_text" | "semantic" | "hybrid";

export interface VaultInfo {
  root: string;
  note_count: number;
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

// ---- vault lifecycle -------------------------------------------------------
export const openVault = (path: string) =>
  invoke<VaultInfo>("open_vault", { path });
export const currentVault = () => invoke<VaultInfo | null>("current_vault");
export const closeVault = () => invoke<void>("close_vault");
export const getLastVault = () => invoke<string | null>("get_last_vault");
export const reindex = () => invoke<IndexStats>("reindex");
export const embedVault = () => invoke<number>("embed_vault");

// ---- notes -----------------------------------------------------------------
export const listNotes = () => invoke<NoteMeta[]>("list_notes");
export const readNote = (rel_path: string) =>
  invoke<NoteContent>("read_note", { rel_path });
export const writeNote = (rel_path: string, content: string) =>
  invoke<void>("write_note", { rel_path, content });
export const createNote = (rel_path: string, content?: string) =>
  invoke<string>("create_note", { rel_path, content: content ?? null });
export const renameNote = (from: string, to: string) =>
  invoke<void>("rename_note", { from, to });
export const deleteNote = (rel_path: string) =>
  invoke<void>("delete_note", { rel_path });

// ---- attachments -----------------------------------------------------------
export const saveAttachment = (bytes: number[], ext: string) =>
  invoke<string>("save_attachment", { bytes, ext });
export const readAttachment = (rel_path: string) =>
  invoke<number[]>("read_attachment", { rel_path });

// ---- canvas (JSONCanvas) ---------------------------------------------------
export interface CanvasNode {
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
  id: string;
  fromNode: string;
  toNode: string;
  fromSide?: string;
  toSide?: string;
  color?: string;
  label?: string;
}
export interface Canvas {
  nodes: CanvasNode[];
  edges: CanvasEdge[];
}
export const listCanvases = () => invoke<string[]>("list_canvases");
export const readCanvas = (rel_path: string) =>
  invoke<Canvas>("read_canvas", { rel_path });
export const writeCanvas = (rel_path: string, canvas: Canvas) =>
  invoke<void>("write_canvas", { rel_path, canvas });
export const createCanvas = (rel_path: string) =>
  invoke<string>("create_canvas", { rel_path });

// ---- search & query --------------------------------------------------------
export const searchFulltext = (query: string, limit = 30) =>
  invoke<SearchHit[]>("search_fulltext", { query, limit });
export const searchSemantic = (query: string, limit = 30) =>
  invoke<SearchHit[]>("search_semantic", { query, limit });
export const searchHybrid = (query: string, limit = 30) =>
  invoke<SearchHit[]>("search_hybrid", { query, limit });
export const runQuery = (sql: string, max_rows = 500) =>
  invoke<QueryResult>("run_query", { sql, max_rows });

// ---- links, graph, tags ----------------------------------------------------
export const getBacklinks = (rel_path: string) =>
  invoke<Backlink[]>("get_backlinks", { rel_path });
export const getGraph = () => invoke<GraphData>("get_graph");
export const getTags = () => invoke<TagCount[]>("get_tags");
export const notesForTag = (tag: string) =>
  invoke<string[]>("notes_for_tag", { tag });

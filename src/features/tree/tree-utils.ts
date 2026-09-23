import type { NoteMeta } from "@/lib/api";

export interface TreeNode {
  name: string;
  /** Folder path (for dirs) or note rel_path (for files). */
  path: string;
  isDir: boolean;
  children: TreeNode[];
  note?: NoteMeta;
}

/** Walk (creating as needed) the folder chain for `dirPath` under `root`. */
function ensureDir(root: TreeNode, dirPath: string): TreeNode {
  let cursor = root;
  let acc = "";
  for (const part of dirPath.split("/").filter(Boolean)) {
    acc = acc ? `${acc}/${part}` : part;
    let dir = cursor.children.find((c) => c.isDir && c.name === part);
    if (!dir) {
      dir = { name: part, path: acc, isDir: true, children: [] };
      cursor.children.push(dir);
    }
    cursor = dir;
  }
  return cursor;
}

/**
 * Build a nested folder/file tree from a flat list of notes plus the vault's
 * folders (so empty folders still show up).
 */
export function buildTree(notes: NoteMeta[], folders: string[] = []): TreeNode {
  const root: TreeNode = { name: "", path: "", isDir: true, children: [] };

  for (const folder of folders) ensureDir(root, folder);

  for (const note of notes) {
    const slash = note.rel_path.lastIndexOf("/");
    const parent = slash === -1 ? root : ensureDir(root, note.rel_path.slice(0, slash));
    parent.children.push({
      name: note.rel_path.slice(slash + 1),
      path: note.rel_path,
      isDir: false,
      children: [],
      note,
    });
  }

  sortNode(root);
  return root;
}

/** Folders first, then files; each alphabetical (case-insensitive). */
function sortNode(node: TreeNode) {
  node.children.sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  });
  for (const child of node.children) if (child.isDir) sortNode(child);
}

/** Display title for a note row (frontmatter/H1 title, else filename stem). */
export function noteLabel(node: TreeNode): string {
  if (node.note && node.note.title.trim()) return node.note.title;
  return node.name.replace(/\.md$/i, "");
}

/** Parent folder of a vault-relative path ("" for the vault root). */
export function parentFolder(relPath: string): string {
  const slash = relPath.lastIndexOf("/");
  return slash === -1 ? "" : relPath.slice(0, slash);
}

/**
 * Normalise a user-typed folder name: trims, collapses slashes, and rejects
 * anything that could escape the vault or hide the folder.
 */
export function cleanFolderName(input: string): string | null {
  const parts = input
    .split(/[\\/]+/)
    .map((p) => p.trim())
    .filter(Boolean);
  if (parts.length === 0) return null;
  if (parts.some((p) => p === "." || p === ".." || p.startsWith("."))) return null;
  return parts.join("/");
}

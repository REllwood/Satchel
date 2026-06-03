import type { NoteMeta } from "@/lib/api";

export interface TreeNode {
  name: string;
  /** Folder path (for dirs) or note rel_path (for files). */
  path: string;
  isDir: boolean;
  children: TreeNode[];
  note?: NoteMeta;
}

/** Build a nested folder/file tree from a flat list of note paths. */
export function buildTree(notes: NoteMeta[]): TreeNode {
  const root: TreeNode = { name: "", path: "", isDir: true, children: [] };

  for (const note of notes) {
    const parts = note.rel_path.split("/");
    let cursor = root;
    let acc = "";
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      const isLeaf = i === parts.length - 1;
      acc = acc ? `${acc}/${part}` : part;
      if (isLeaf) {
        cursor.children.push({
          name: part,
          path: note.rel_path,
          isDir: false,
          children: [],
          note,
        });
      } else {
        let dir = cursor.children.find((c) => c.isDir && c.name === part);
        if (!dir) {
          dir = { name: part, path: acc, isDir: true, children: [] };
          cursor.children.push(dir);
        }
        cursor = dir;
      }
    }
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

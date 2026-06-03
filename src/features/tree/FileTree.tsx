import { useMemo, useRef, useState, type KeyboardEvent } from "react";
import { ChevronDown, ChevronRight, FileText, MoreHorizontal } from "lucide-react";
import { toast } from "sonner";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useVault } from "@/features/vault/vault-store";
import { buildTree, noteLabel, type TreeNode } from "@/features/tree/tree-utils";

export function FileTree() {
  const { notes, selected, select, renameNote, removeNote, tagFilter, tagPaths } =
    useVault();
  const shown = useMemo(
    () => (tagFilter ? notes.filter((n) => tagPaths.has(n.rel_path)) : notes),
    [notes, tagFilter, tagPaths],
  );
  const tree = useMemo(() => buildTree(shown), [shown]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [renaming, setRenaming] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const toggle = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      next.has(path) ? next.delete(path) : next.add(path);
      return next;
    });

  // Roving focus across visible rows.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const rows = Array.from(
      containerRef.current?.querySelectorAll<HTMLElement>("[data-tree-row]") ?? [],
    );
    const idx = rows.indexOf(document.activeElement as HTMLElement);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      rows[Math.min(idx + 1, rows.length - 1)]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      rows[Math.max(idx - 1, 0)]?.focus();
    } else if (e.key === "Home") {
      e.preventDefault();
      rows[0]?.focus();
    } else if (e.key === "End") {
      e.preventDefault();
      rows[rows.length - 1]?.focus();
    }
  };

  if (notes.length === 0) {
    return (
      <p className="px-1 py-2 text-sm text-muted-foreground">
        No notes yet. Create one with the + button above.
      </p>
    );
  }
  if (shown.length === 0) {
    return (
      <p className="px-1 py-2 text-sm text-muted-foreground">
        No notes tagged #{tagFilter}.
      </p>
    );
  }

  const renderNode = (node: TreeNode, depth: number) => {
    const pad = { paddingLeft: `${depth * 12 + 4}px` };
    if (node.isDir) {
      const isCollapsed = collapsed.has(node.path);
      return (
        <div key={`d:${node.path}`}>
          <button
            type="button"
            data-tree-row
            style={pad}
            className="flex w-full items-center gap-1 rounded-sm py-1 pr-2 text-left text-sm hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-expanded={!isCollapsed}
            onClick={() => toggle(node.path)}
          >
            {isCollapsed ? (
              <ChevronRight className="size-3.5 shrink-0" />
            ) : (
              <ChevronDown className="size-3.5 shrink-0" />
            )}
            <span className="truncate font-medium">{node.name}</span>
          </button>
          {!isCollapsed && node.children.map((c) => renderNode(c, depth + 1))}
        </div>
      );
    }

    const isSelected = selected === node.path;
    const isRenaming = renaming === node.path;
    return (
      <div key={`f:${node.path}`} className="group/row relative flex items-center">
        {isRenaming ? (
          <RenameRow
            node={node}
            pad={pad}
            onCommit={(to) => {
              setRenaming(null);
              renameNote(node.path, to);
            }}
            onCancel={() => setRenaming(null)}
          />
        ) : (
          <>
            <button
              type="button"
              data-tree-row
              style={pad}
              className={cn(
                "flex min-w-0 flex-1 items-center gap-1.5 rounded-sm py-1 pr-7 text-left text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                isSelected
                  ? "bg-accent text-accent-foreground"
                  : "hover:bg-accent/60",
              )}
              aria-current={isSelected ? "true" : undefined}
              onClick={() => select(node.path)}
            >
              <FileText className="size-3.5 shrink-0 text-muted-foreground" />
              <span className="truncate">{noteLabel(node)}</span>
            </button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Actions for ${node.name}`}
                  className="absolute right-1 size-6 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100"
                >
                  <MoreHorizontal className="size-3.5" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => setRenaming(node.path)}>
                  Rename
                </DropdownMenuItem>
                <DropdownMenuItem
                  variant="destructive"
                  onClick={() =>
                    toast("Delete this note?", {
                      action: {
                        label: "Delete",
                        onClick: () => removeNote(node.path),
                      },
                    })
                  }
                >
                  Delete
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </>
        )}
      </div>
    );
  };

  return (
    <div ref={containerRef} role="tree" aria-label="Notes" onKeyDown={onKeyDown}>
      {tree.children.map((c) => renderNode(c, 0))}
    </div>
  );
}

function RenameRow({
  node,
  pad,
  onCommit,
  onCancel,
}: {
  node: TreeNode;
  pad: React.CSSProperties;
  onCommit: (to: string) => void;
  onCancel: () => void;
}) {
  const dir = node.path.includes("/")
    ? node.path.slice(0, node.path.lastIndexOf("/") + 1)
    : "";
  const [value, setValue] = useState(node.name);
  const commit = () => {
    let name = value.trim();
    if (!name) return onCancel();
    if (!/\.md$/i.test(name)) name += ".md";
    onCommit(`${dir}${name}`);
  };
  return (
    <div style={pad} className="flex-1 py-0.5 pr-2">
      <Input
        autoFocus
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          else if (e.key === "Escape") onCancel();
        }}
        className="h-7 text-sm"
      />
    </div>
  );
}

import { useMemo, useRef, useState, type KeyboardEvent } from "react";
import {
  ChevronDown,
  ChevronRight,
  FileText,
  Folder,
  FolderInput,
  MoreHorizontal,
} from "lucide-react";
import { toast } from "sonner";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useVault } from "@/features/vault/vault-store";
import {
  buildTree,
  cleanFolderName,
  noteLabel,
  parentFolder,
  type TreeNode,
} from "@/features/tree/tree-utils";

interface FileTreeProps {
  /** Folder currently getting an inline "new folder" row ("" = vault root). */
  newFolderIn: string | null;
  onNewFolderIn: (folder: string | null) => void;
}

export function FileTree({ newFolderIn, onNewFolderIn }: FileTreeProps) {
  const {
    notes,
    folders,
    selected,
    select,
    createNote,
    renameNote,
    moveNote,
    removeNote,
    createFolder,
    renameFolder,
    removeFolder,
    tagFilter,
    tagPaths,
  } = useVault();
  const shown = useMemo(
    () => (tagFilter ? notes.filter((n) => tagPaths.has(n.rel_path)) : notes),
    [notes, tagFilter, tagPaths],
  );
  // With a tag filter active, only show folders that contain matching notes.
  const tree = useMemo(
    () => buildTree(shown, tagFilter ? [] : folders),
    [shown, folders, tagFilter],
  );
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [renaming, setRenaming] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const toggle = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const startNewFolder = (parent: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      next.delete(parent);
      return next;
    });
    onNewFolderIn(parent);
  };

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

  const newFolderRow = (parent: string, depth: number) =>
    newFolderIn === parent ? (
      <NameInputRow
        key={`new:${parent}`}
        pad={{ paddingLeft: `${depth * 12 + 4}px` }}
        initial=""
        placeholder="Folder name"
        onCommit={async (value) => {
          onNewFolderIn(null);
          const name = cleanFolderName(value);
          if (!name) return;
          await createFolder(parent ? `${parent}/${name}` : name);
        }}
        onCancel={() => onNewFolderIn(null)}
      />
    ) : null;

  if (notes.length === 0 && folders.length === 0 && newFolderIn === null) {
    return (
      <p className="px-1 py-2 text-sm text-muted-foreground">
        No notes yet. Create one with the + button above.
      </p>
    );
  }
  if (tagFilter && shown.length === 0) {
    return (
      <p className="px-1 py-2 text-sm text-muted-foreground">
        No notes tagged #{tagFilter}.
      </p>
    );
  }

  const moveTargets = ["", ...folders];

  const renderNode = (node: TreeNode, depth: number) => {
    const pad = { paddingLeft: `${depth * 12 + 4}px` };

    if (node.isDir) {
      const isCollapsed = collapsed.has(node.path);
      const isRenaming = renaming === node.path;
      return (
        <div key={`d:${node.path}`}>
          <div className="group/row relative flex items-center">
            {isRenaming ? (
              <NameInputRow
                pad={pad}
                initial={node.name}
                onCommit={(value) => {
                  setRenaming(null);
                  const name = cleanFolderName(value);
                  if (!name || name.includes("/")) return;
                  const parent = parentFolder(node.path);
                  void renameFolder(node.path, parent ? `${parent}/${name}` : name);
                }}
                onCancel={() => setRenaming(null)}
              />
            ) : (
              <>
                <button
                  type="button"
                  data-tree-row
                  style={pad}
                  className="flex min-w-0 flex-1 items-center gap-1 rounded-sm py-1 pr-7 text-left text-sm hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
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
                <RowMenu label={node.name}>
                  <DropdownMenuItem onClick={() => void createNote(node.path)}>
                    New note here
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => startNewFolder(node.path)}>
                    New folder inside
                  </DropdownMenuItem>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem onClick={() => setRenaming(node.path)}>
                    Rename
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    variant="destructive"
                    onClick={() =>
                      toast(`Move “${node.name}” and everything in it to the Trash?`, {
                        action: {
                          label: "Move to Trash",
                          onClick: () => void removeFolder(node.path),
                        },
                      })
                    }
                  >
                    Delete folder
                  </DropdownMenuItem>
                </RowMenu>
              </>
            )}
          </div>
          {!isCollapsed && (
            <>
              {newFolderRow(node.path, depth + 1)}
              {node.children.map((c) => renderNode(c, depth + 1))}
            </>
          )}
        </div>
      );
    }

    const isSelected = selected === node.path;
    const isRenaming = renaming === node.path;
    const here = parentFolder(node.path);
    return (
      <div key={`f:${node.path}`} className="group/row relative flex items-center">
        {isRenaming ? (
          <NameInputRow
            pad={pad}
            initial={node.name}
            onCommit={(value) => {
              setRenaming(null);
              let name = value.trim().replace(/[\\/]/g, "-");
              if (!name) return;
              if (!/\.md$/i.test(name)) name += ".md";
              void renameNote(node.path, here ? `${here}/${name}` : name);
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
                isSelected ? "bg-accent text-accent-foreground" : "hover:bg-accent/60",
              )}
              aria-current={isSelected ? "true" : undefined}
              onClick={() => select(node.path)}
            >
              <FileText className="size-3.5 shrink-0 text-muted-foreground" />
              <span className="truncate">{noteLabel(node)}</span>
            </button>
            <RowMenu label={node.name}>
              <DropdownMenuItem onClick={() => setRenaming(node.path)}>
                Rename
              </DropdownMenuItem>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <FolderInput />
                  Move to
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="max-h-72 overflow-y-auto">
                  {moveTargets.map((folder) => (
                    <DropdownMenuItem
                      key={folder || "/"}
                      disabled={folder === here}
                      onClick={() => void moveNote(node.path, folder)}
                    >
                      <Folder />
                      <span className="truncate">{folder || "Top level"}</span>
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                variant="destructive"
                onClick={() =>
                  toast("Move this note to the Trash?", {
                    action: {
                      label: "Move to Trash",
                      onClick: () => void removeNote(node.path),
                    },
                  })
                }
              >
                Delete
              </DropdownMenuItem>
            </RowMenu>
          </>
        )}
      </div>
    );
  };

  return (
    <div ref={containerRef} role="tree" aria-label="Notes" onKeyDown={onKeyDown}>
      {newFolderRow("", 0)}
      {tree.children.map((c) => renderNode(c, 0))}
    </div>
  );
}

function RowMenu({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label={`Actions for ${label}`}
          className="absolute right-1 size-6 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100"
        >
          <MoreHorizontal className="size-3.5" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">{children}</DropdownMenuContent>
    </DropdownMenu>
  );
}

function NameInputRow({
  pad,
  initial,
  placeholder,
  onCommit,
  onCancel,
}: {
  pad: React.CSSProperties;
  initial: string;
  placeholder?: string;
  onCommit: (value: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initial);
  const done = useRef(false);
  const finish = (commit: boolean) => {
    if (done.current) return;
    done.current = true;
    if (commit && value.trim() && value.trim() !== initial) onCommit(value);
    else onCancel();
  };
  return (
    <div style={pad} className="flex-1 py-0.5 pr-2">
      <Input
        autoFocus
        value={value}
        placeholder={placeholder}
        aria-label={placeholder ?? "New name"}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => finish(true)}
        onKeyDown={(e) => {
          if (e.key === "Enter") finish(true);
          else if (e.key === "Escape") finish(false);
        }}
        className="h-7 text-sm"
      />
    </div>
  );
}

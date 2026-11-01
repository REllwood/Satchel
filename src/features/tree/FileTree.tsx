import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import {
  ChevronDown,
  ChevronRight,
  FileText,
  Folder,
  FolderInput,
  MoreHorizontal,
} from "lucide-react";

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
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { useVault } from "@/features/vault/vault-store";
import {
  buildTree,
  cleanFolderName,
  noteLabel,
  parentFolder,
  rowKey,
  visibleRows,
  type TreeRow,
} from "@/features/tree/tree-utils";

interface FileTreeProps {
  /** Folder currently getting an inline "new folder" row ("" = vault root). */
  newFolderIn: string | null;
  onNewFolderIn: (folder: string | null) => void;
}

/** A note or folder waiting for the user to confirm moving it to the Trash. */
interface PendingDelete {
  key: string;
  path: string;
  label: string;
  isDir: boolean;
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
    setTagFilter,
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
  const rows = useMemo(() => visibleRows(tree, collapsed), [tree, collapsed]);
  const [renaming, setRenaming] = useState<string | null>(null);
  /** The row whose menu is open. */
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<PendingDelete | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const rowEls = useRef(new Map<string, HTMLElement>());
  // A menu item that puts focus somewhere else (a name input or a dialog)
  // runs once the menu has closed. While it is open, the menu keeps focus.
  const afterMenu = useRef<(() => void) | null>(null);
  /** The row to focus when the delete dialog closes. */
  const deleteFrom = useRef<string | null>(null);
  // A row to focus once it is on screen, after a name input closes.
  const refocus = useRef<{ key: string; until: number } | null>(null);

  // Roving tabindex: the tree is one Tab stop, on the row last focused, or
  // else the open note, or else the first row.
  const [focusKey, setFocusKey] = useState<string | null>(null);
  useEffect(() => {
    if (selected) setFocusKey(`f:${selected}`);
  }, [selected]);
  const tabStop =
    rows.find((r) => rowKey(r.node) === focusKey) ??
    rows.find((r) => !r.node.isDir && r.node.path === selected) ??
    rows[0];
  const tabStopKey = tabStop ? rowKey(tabStop.node) : null;

  const focusRow = (key: string) => rowEls.current.get(key)?.focus();

  useEffect(() => {
    const want = refocus.current;
    if (!want) return;
    if (Date.now() > want.until) {
      refocus.current = null;
      return;
    }
    const el = rowEls.current.get(want.key);
    const active = document.activeElement;
    if (!el) return;
    refocus.current = null;
    // Only if focus was dropped, not if the user moved on to something else.
    if (!active || active === document.body || containerRef.current?.contains(active)) el.focus();
  });
  const focusLater = (key: string) => {
    refocus.current = { key, until: Date.now() + 3000 };
  };

  const setExpanded = (path: string, expanded: boolean) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (expanded) next.delete(path);
      else next.add(path);
      return next;
    });
  const toggle = (path: string) => setExpanded(path, collapsed.has(path));

  const startNewFolder = (parent: string) => {
    setExpanded(parent, true);
    onNewFolderIn(parent);
  };

  const activate = (row: TreeRow) => {
    if (row.node.isDir) toggle(row.node.path);
    else select(row.node.path);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    // Only keys pressed on a row: not a name being typed, nor a menu or
    // dialog (their portals bubble React events through the tree).
    const target = e.target as HTMLElement;
    if (!target.hasAttribute("data-tree-row") || !containerRef.current?.contains(target)) return;
    const index = rows.findIndex((r) => rowKey(r.node) === target.dataset.key);
    const row = rows[index];
    if (!row) return;
    const { node } = row;
    const go = (to: TreeRow | undefined) => to && focusRow(rowKey(to.node));
    switch (e.key) {
      case "ArrowDown":
        go(rows[index + 1]);
        break;
      case "ArrowUp":
        go(rows[index - 1]);
        break;
      case "Home":
        go(rows[0]);
        break;
      case "End":
        go(rows[rows.length - 1]);
        break;
      case "ArrowRight":
        // Expand a folder, or step into an open one.
        if (!node.isDir) return;
        if (collapsed.has(node.path)) setExpanded(node.path, true);
        else if (rows[index + 1]?.parent === node.path) go(rows[index + 1]);
        break;
      case "ArrowLeft":
        // Collapse an open folder, or step out to the folder this row is in.
        if (node.isDir && !collapsed.has(node.path)) setExpanded(node.path, false);
        else if (row.parent !== null) focusRow(`d:${row.parent}`);
        break;
      case "Enter":
      case " ":
        activate(row);
        break;
      case "F2":
        setRenaming(node.path);
        break;
      case "F10":
        if (!e.shiftKey) return;
        setMenuFor(rowKey(node));
        break;
      case "ContextMenu":
        setMenuFor(rowKey(node));
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  /** A menu closed: run the chosen item's follow-up, or focus its row. */
  const onMenuClosed = (key: string) => (e: Event) => {
    e.preventDefault();
    const next = afterMenu.current;
    afterMenu.current = null;
    if (next) return next();
    // Unless the click that closed it put focus somewhere else.
    const active = document.activeElement;
    if (!active || active === document.body || containerRef.current?.contains(active)) focusRow(key);
  };
  const thenFocus = (next: () => void) => () => {
    afterMenu.current = next;
  };

  const askDelete = (row: TreeRow) => {
    const { node } = row;
    deleteFrom.current = rowKey(node);
    setConfirmDelete({
      key: rowKey(node),
      path: node.path,
      label: node.isDir ? node.name : noteLabel(node),
      isDir: node.isDir,
    });
  };

  const newFolderRow = (parent: string, level: number) =>
    newFolderIn === parent ? (
      <NameInputRow
        key={`new:${parent}`}
        pad={{ paddingLeft: `${(level - 1) * 12 + 4}px` }}
        initial=""
        placeholder="Folder name"
        onCommit={async (value) => {
          onNewFolderIn(null);
          const name = cleanFolderName(value);
          if (!name) return;
          const path = parent ? `${parent}/${name}` : name;
          focusLater(`d:${path}`);
          await createFolder(path);
        }}
        onCancel={() => {
          onNewFolderIn(null);
          if (parent) focusLater(`d:${parent}`);
        }}
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
      <div className="px-1 py-2 text-sm text-muted-foreground">
        <p>No notes tagged #{tagFilter}.</p>
        <Button
          variant="link"
          size="sm"
          className="h-auto px-0"
          onClick={() => void setTagFilter(null)}
        >
          Clear the filter
        </Button>
      </div>
    );
  }

  const moveTargets = ["", ...folders];

  const renderRow = (row: TreeRow) => {
    const { node, level } = row;
    const key = rowKey(node);
    const pad = { paddingLeft: `${(level - 1) * 12 + 4}px` };

    if (renaming === node.path) {
      const here = parentFolder(node.path);
      return (
        <NameInputRow
          key={`rename:${key}`}
          pad={pad}
          initial={node.name}
          onCommit={(value) => {
            setRenaming(null);
            if (node.isDir) {
              const name = cleanFolderName(value);
              if (!name || name.includes("/")) return focusLater(key);
              const to = here ? `${here}/${name}` : name;
              focusLater(`d:${to}`);
              void renameFolder(node.path, to);
            } else {
              let name = value.trim().replace(/[\\/]/g, "-");
              if (!name) return focusLater(key);
              if (!/\.md$/i.test(name)) name += ".md";
              const to = here ? `${here}/${name}` : name;
              focusLater(`f:${to}`);
              void renameNote(node.path, to);
            }
          }}
          onCancel={() => {
            setRenaming(null);
            focusLater(key);
          }}
        />
      );
    }

    const isCollapsed = node.isDir && collapsed.has(node.path);
    const isSelected = !node.isDir && selected === node.path;
    const label = node.isDir ? node.name : noteLabel(node);
    const here = parentFolder(node.path);

    const menu = node.isDir ? (
      <>
        <DropdownMenuItem onSelect={() => void createNote(node.path)}>
          New note here
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={thenFocus(() => startNewFolder(node.path))}>
          New folder inside
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={thenFocus(() => setRenaming(node.path))}>
          Rename
        </DropdownMenuItem>
        <DropdownMenuItem variant="destructive" onSelect={thenFocus(() => askDelete(row))}>
          Delete folder
        </DropdownMenuItem>
      </>
    ) : (
      <>
        <DropdownMenuItem onSelect={thenFocus(() => setRenaming(node.path))}>
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
                onSelect={() => void moveNote(node.path, folder)}
              >
                <Folder />
                <span className="truncate">{folder || "Top level"}</span>
              </DropdownMenuItem>
            ))}
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onSelect={thenFocus(() => askDelete(row))}>
          Delete
        </DropdownMenuItem>
      </>
    );

    return (
      <div
        key={key}
        ref={(el) => {
          if (el) rowEls.current.set(key, el);
          else rowEls.current.delete(key);
        }}
        role="treeitem"
        aria-label={label}
        aria-level={level}
        aria-setsize={row.setSize}
        aria-posinset={row.posInSet}
        aria-expanded={node.isDir ? !isCollapsed : undefined}
        aria-selected={isSelected}
        aria-haspopup="menu"
        tabIndex={key === tabStopKey ? 0 : -1}
        data-tree-row
        data-key={key}
        style={pad}
        className={cn(
          "group/row relative flex min-w-0 cursor-default select-none items-center rounded-sm py-1 pr-7 text-left text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          node.isDir ? "gap-1" : "gap-1.5",
          isSelected
            ? "bg-accent text-accent-foreground"
            : node.isDir
              ? "hover:bg-accent hover:text-accent-foreground"
              : "hover:bg-accent/60",
        )}
        onFocus={(e) => {
          if (e.target === e.currentTarget) setFocusKey(key);
        }}
        // Clicks in the row's menu bubble here through its portal: skip those.
        onClick={(e) => {
          if (e.currentTarget.contains(e.target as Node)) activate(row);
        }}
        onContextMenu={(e) => {
          if (!e.currentTarget.contains(e.target as Node)) return;
          e.preventDefault();
          setMenuFor(key);
        }}
      >
        {node.isDir ? (
          isCollapsed ? (
            <ChevronRight className="size-3.5 shrink-0" />
          ) : (
            <ChevronDown className="size-3.5 shrink-0" />
          )
        ) : (
          <FileText className="size-3.5 shrink-0 text-muted-foreground" />
        )}
        <span className={cn("truncate", node.isDir && "font-medium")}>{label}</span>
        <RowMenu
          label={node.name}
          open={menuFor === key}
          onOpenChange={(open) => setMenuFor(open ? key : null)}
          onCloseAutoFocus={onMenuClosed(key)}
        >
          {menu}
        </RowMenu>
      </div>
    );
  };

  const items: ReactNode[] = [newFolderRow("", 1)];
  for (const row of rows) {
    items.push(renderRow(row));
    if (row.node.isDir && !collapsed.has(row.node.path)) {
      items.push(newFolderRow(row.node.path, row.level + 1));
    }
  }

  return (
    <>
      <div ref={containerRef} role="tree" aria-label="Notes" onKeyDown={onKeyDown}>
        {items}
      </div>
      <AlertDialog
        open={confirmDelete !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmDelete(null);
        }}
      >
        <AlertDialogContent
          onCloseAutoFocus={(e) => {
            e.preventDefault();
            if (deleteFrom.current) focusRow(deleteFrom.current);
            deleteFrom.current = null;
          }}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirmDelete?.isDir
                ? `Move “${confirmDelete.label}” and everything in it to the Trash?`
                : `Move “${confirmDelete?.label}” to the Trash?`}
            </AlertDialogTitle>
            <AlertDialogDescription>
              It goes to the Trash on this computer, so you can restore it from there.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                if (!confirmDelete) return;
                if (confirmDelete.isDir) void removeFolder(confirmDelete.path);
                else void removeNote(confirmDelete.path);
              }}
            >
              Move to Trash
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

function RowMenu({
  label,
  open,
  onOpenChange,
  onCloseAutoFocus,
  children,
}: {
  label: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCloseAutoFocus: (e: Event) => void;
  children: ReactNode;
}) {
  // Not a Tab stop of its own: the row opens it with Shift+F10 or the
  // context-menu key, as well as by right-click.
  return (
    <DropdownMenu open={open} onOpenChange={onOpenChange}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          tabIndex={-1}
          aria-label={`Actions for ${label}`}
          className="absolute right-1 size-6 opacity-0 group-hover/row:opacity-100 group-focus-visible/row:opacity-100 data-[state=open]:opacity-100"
          onClick={(e) => e.stopPropagation()}
        >
          <MoreHorizontal className="size-3.5" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" onCloseAutoFocus={onCloseAutoFocus}>
        {children}
      </DropdownMenuContent>
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

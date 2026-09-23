import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  Database,
  FolderPlus,
  LayoutDashboard,
  Network,
  PanelLeft,
  PanelRight,
  Plus,
  Search,
  Settings,
} from "lucide-react";
import type { ImperativePanelHandle } from "react-resizable-panels";

import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ModeToggle } from "@/components/mode-toggle";
import { StatusBar } from "@/components/layout/StatusBar";
import { FileTree } from "@/features/tree/FileTree";
import { Editor } from "@/features/editor/Editor";
import { ContextPanel } from "@/features/panel/ContextPanel";
import { GraphView } from "@/features/graph/GraphView";
import { CanvasView } from "@/features/canvas/CanvasView";
import { SearchDialog } from "@/features/search/SearchDialog";
import { QueryDialog } from "@/features/query/QueryDialog";
import { TagsBar } from "@/features/tags/TagsBar";
import { SettingsDialog } from "@/features/settings/SettingsDialog";
import {
  CommandPalette,
  type PaletteCommand,
} from "@/features/command/CommandPalette";
import { useVault } from "@/features/vault/vault-store";
import { Onboarding } from "@/features/vault/Onboarding";
import { useTheme } from "@/components/theme-provider";

function Pane({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex h-full flex-col bg-sidebar">
      <div className="flex h-8 shrink-0 items-center px-3 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {title}
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-3 pb-3 text-sm">
        {children}
      </div>
    </div>
  );
}

function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

function NotesPanel() {
  const { vault, createNote } = useVault();
  // Folder that currently shows an inline "new folder" input ("" = top level).
  const [newFolderIn, setNewFolderIn] = useState<string | null>(null);
  return (
    <div className="flex h-full flex-col bg-sidebar">
      <div className="flex h-8 shrink-0 items-center gap-0.5 px-2">
        <span
          className="flex-1 truncate text-xs font-medium uppercase tracking-wide text-muted-foreground"
          title={vault?.root}
        >
          {vault ? baseName(vault.root) : "Notes"}
        </span>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              size="icon"
              variant="ghost"
              className="size-6"
              aria-label="New folder"
              onClick={() => setNewFolderIn("")}
            >
              <FolderPlus />
            </Button>
          </TooltipTrigger>
          <TooltipContent>New folder</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              size="icon"
              variant="ghost"
              className="size-6"
              aria-label="New note"
              onClick={() => void createNote()}
            >
              <Plus />
            </Button>
          </TooltipTrigger>
          <TooltipContent>New note (⌘N)</TooltipContent>
        </Tooltip>
      </div>
      <TagsBar />
      <ScrollArea className="min-h-0 flex-1 px-1 pb-2">
        <FileTree newFolderIn={newFolderIn} onNewFolderIn={setNewFolderIn} />
      </ScrollArea>
    </div>
  );
}

export function AppShell() {
  const leftRef = useRef<ImperativePanelHandle>(null);
  const rightRef = useRef<ImperativePanelHandle>(null);
  const [graphOpen, setGraphOpen] = useState(false);
  const [canvasOpen, setCanvasOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [queryOpen, setQueryOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);

  const { vault, booting, createNote, closeVault, buildEmbeddings, addAgentDocs } =
    useVault();
  const { setTheme } = useTheme();

  const commands: PaletteCommand[] = [
    { id: "new", label: "New note", hint: "⌘N", run: () => void createNote() },
    { id: "search", label: "Search notes", hint: "⌘K", run: () => setSearchOpen(true) },
    { id: "graph", label: "Open graph view", run: () => setGraphOpen(true) },
    { id: "canvas", label: "Open canvas", run: () => setCanvasOpen(true) },
    { id: "query", label: "Open query view", run: () => setQueryOpen(true) },
    { id: "vault", label: "Change notes location…", run: closeVault },
    { id: "embed", label: "Rebuild semantic index", run: buildEmbeddings },
    { id: "agents", label: "Add AGENTS.md / CLAUDE.md for AI tools", run: addAgentDocs },
    { id: "settings", label: "Open settings", run: () => setSettingsOpen(true) },
    { id: "theme-system", label: "Theme: System", run: () => setTheme("system") },
    { id: "theme-light", label: "Theme: Light", run: () => setTheme("light") },
    { id: "theme-dark", label: "Theme: Dark", run: () => setTheme("dark") },
  ];

  const toggle = (ref: typeof leftRef) => {
    const panel = ref.current;
    if (!panel) return;
    if (panel.isCollapsed()) panel.expand();
    else panel.collapse();
  };

  // Global shortcuts: ⌘/Ctrl+K search, ⌘/Ctrl+\ toggle sidebar.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "p") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setSearchOpen((v) => !v);
      } else if ((e.metaKey || e.ctrlKey) && e.key === "\\") {
        e.preventDefault();
        toggle(leftRef);
      } else if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        void createNote();
      }
    };
    if (!vault) return;
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [vault, createNote]);

  if (booting) return <div className="h-full bg-background" aria-busy="true" />;
  if (!vault) {
    return (
      <div className="flex h-full flex-col">
        <div className="min-h-0 flex-1">
          <Onboarding />
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-11 shrink-0 items-center gap-1 border-b px-2">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Toggle notes sidebar"
              onClick={() => toggle(leftRef)}
            >
              <PanelLeft />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Toggle notes</TooltipContent>
        </Tooltip>
        <span className="ml-1 select-none font-semibold tracking-tight">
          MyNote
        </span>
        <div className="flex-1" />
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Search notes"
              onClick={() => setSearchOpen(true)}
            >
              <Search />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Search (⌘K)</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Query view"
              onClick={() => setQueryOpen(true)}
            >
              <Database />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Query</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Open graph view"
              onClick={() => setGraphOpen(true)}
            >
              <Network />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Graph view</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Open canvas"
              onClick={() => setCanvasOpen(true)}
            >
              <LayoutDashboard />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Canvas</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Toggle context panel"
              onClick={() => toggle(rightRef)}
            >
              <PanelRight />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Toggle context panel</TooltipContent>
        </Tooltip>
        <ModeToggle />
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Settings"
              onClick={() => setSettingsOpen(true)}
            >
              <Settings />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Settings</TooltipContent>
        </Tooltip>
      </header>

      <div className="min-h-0 flex-1">
        <ResizablePanelGroup direction="horizontal" autoSaveId="mynote-layout">
          <ResizablePanel
            ref={leftRef}
            order={1}
            collapsible
            collapsedSize={0}
            minSize={12}
            defaultSize={20}
          >
            <NotesPanel />
          </ResizablePanel>

          <ResizableHandle withHandle />

          <ResizablePanel order={2} minSize={30} defaultSize={56}>
            <div className="h-full bg-background">
              <Editor />
            </div>
          </ResizablePanel>

          <ResizableHandle withHandle />

          <ResizablePanel
            ref={rightRef}
            order={3}
            collapsible
            collapsedSize={0}
            minSize={14}
            defaultSize={24}
          >
            <Pane title="Context">
              <ContextPanel />
            </Pane>
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>

      <StatusBar />
      <GraphView open={graphOpen} onOpenChange={setGraphOpen} />
      <CanvasView open={canvasOpen} onOpenChange={setCanvasOpen} />
      <SearchDialog open={searchOpen} onOpenChange={setSearchOpen} />
      <QueryDialog open={queryOpen} onOpenChange={setQueryOpen} />
      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        commands={commands}
      />
    </div>
  );
}

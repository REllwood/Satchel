import { useCallback, useEffect, useRef, useState } from "react";
import {
  addEdge,
  applyEdgeChanges,
  applyNodeChanges,
  Background,
  Controls,
  ReactFlow,
  type Connection,
  type Edge,
  type EdgeChange,
  type Node,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { Plus } from "lucide-react";
import { toast } from "sonner";

import * as api from "@/lib/api";
import { useVault } from "@/features/vault/vault-store";
import { registerFlush } from "@/features/editor/pending-save";
import { useTheme } from "@/components/theme-provider";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { fromFlow, toFlow, toFlowNode, type CardData } from "./canvas-model";

export function CanvasView({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { select } = useVault();
  const { resolved } = useTheme();
  const [canvases, setCanvases] = useState<string[]>([]);
  const [path, setPath] = useState<string | null>(null);
  const [nodes, setNodes] = useState<Node[]>([]);
  const [edges, setEdges] = useState<Edge[]>([]);
  // Live mirrors, so a save always writes what is on screen.
  const live = useRef({ path, nodes, edges });
  live.current = { path, nodes, edges };
  /** The canvas as loaded: saving merges into it, keeping what Satchel does not edit. */
  const base = useRef<api.Canvas>({ nodes: [], edges: [] });
  // Unsaved edits, their debounce timer, and the save queue (one at a time).
  const dirty = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const saves = useRef<Promise<void>>(Promise.resolve());
  // Only the newest load applies when canvases are switched quickly.
  const loadRequest = useRef(0);

  /** Write unsaved edits now. */
  const flush = useCallback((): Promise<void> => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    const { path: target, nodes: n, edges: e } = live.current;
    if (!dirty.current || !target) return saves.current;
    dirty.current = false;
    const canvas = fromFlow(base.current, n, e);
    saves.current = saves.current.then(() =>
      api.writeCanvas(target, canvas).catch((err) => {
        toast.error(`Could not save ${target}: ${err}`);
        // Try again with the next change, or when the canvas closes.
        if (live.current.path === target) dirty.current = true;
      }),
    );
    return saves.current;
  }, []);

  /** Note an edit, saving it shortly after the last one. */
  const markDirty = useCallback(() => {
    dirty.current = true;
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => void flush(), 500);
  }, [flush]);

  // Quitting, or a file operation elsewhere, saves the canvas too.
  useEffect(() => {
    const unregister = registerFlush(flush);
    return () => {
      unregister();
      void flush();
    };
  }, [flush]);

  const load = useCallback(
    async (rel: string) => {
      const request = ++loadRequest.current;
      await flush();
      let canvas: api.Canvas;
      try {
        canvas = await api.readCanvas(rel);
      } catch (e) {
        if (request === loadRequest.current) toast.error(`Could not open ${rel}: ${e}`);
        return;
      }
      if (request !== loadRequest.current) return;
      base.current = canvas;
      dirty.current = false;
      const flow = toFlow(canvas);
      setNodes(flow.nodes);
      setEdges(flow.edges);
      setPath(rel);
    },
    [flush],
  );

  useEffect(() => {
    if (!open) return;
    api.listCanvases().then(setCanvases).catch(() => setCanvases([]));
    // Reopening shows the canvas as it is on disk now.
    if (live.current.path) void load(live.current.path);
  }, [open, load]);

  // Selecting a card or measuring it is not an edit, so it does not save.
  const onNodesChange = useCallback(
    (changes: NodeChange[]) => {
      setNodes((nds) => applyNodeChanges(changes, nds));
      if (changes.some((c) => c.type !== "select" && c.type !== "dimensions")) markDirty();
    },
    [markDirty],
  );
  const onEdgesChange = useCallback(
    (changes: EdgeChange[]) => {
      setEdges((eds) => applyEdgeChanges(changes, eds));
      if (changes.some((c) => c.type !== "select")) markDirty();
    },
    [markDirty],
  );
  const onConnect = useCallback(
    (c: Connection) => {
      setEdges((eds) => addEdge({ ...c, id: crypto.randomUUID() }, eds));
      markDirty();
    },
    [markDirty],
  );

  const addCard = () => {
    const cn: api.CanvasNode = {
      id: crypto.randomUUID(),
      type: "text",
      text: "New card",
      x: 40,
      y: 40,
      width: 220,
      height: 100,
    };
    setNodes((nds) => [...nds, toFlowNode(cn)]);
    markDirty();
  };

  const newCanvas = async () => {
    await flush();
    try {
      const rel = await api.createCanvas("Untitled.canvas");
      setCanvases((c) => [...new Set([...c, rel])].sort());
      await load(rel);
    } catch (e) {
      toast.error(`Could not create a canvas: ${e}`);
    }
  };

  const changeOpen = (next: boolean) => {
    if (!next) void flush();
    onOpenChange(next);
  };

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent className="flex h-[88vh] w-[92vw] max-w-[92vw] flex-col gap-0 p-0 sm:max-w-[92vw]">
        <DialogHeader className="flex-row items-center gap-2 space-y-0 border-b px-4 py-2">
          <DialogTitle className="mr-2">Canvas</DialogTitle>
          <select
            value={path ?? ""}
            onChange={(e) => e.target.value && void load(e.target.value)}
            aria-label="Select canvas"
            className="h-8 rounded-md border bg-background px-2 text-sm"
          >
            <option value="" disabled>
              {canvases.length ? "Select a canvas…" : "No canvases yet"}
            </option>
            {canvases.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
          <Button size="sm" variant="outline" onClick={() => void newCanvas()}>
            New
          </Button>
          <Button size="sm" variant="outline" disabled={!path} onClick={addCard}>
            <Plus /> Card
          </Button>
        </DialogHeader>
        <div className="min-h-0 flex-1">
          {path ? (
            <ReactFlow
              nodes={nodes}
              edges={edges}
              onNodesChange={onNodesChange}
              onEdgesChange={onEdgesChange}
              onConnect={onConnect}
              onNodeDoubleClick={(_, node) => {
                const cn = (node.data as CardData).canvasNode;
                if (cn.type === "file" && cn.file) {
                  select(cn.file);
                  changeOpen(false);
                }
              }}
              fitView
              colorMode={resolved}
              proOptions={{ hideAttribution: true }}
            >
              <Background />
              <Controls />
            </ReactFlow>
          ) : (
            <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
              Select or create a canvas to start mapping your notes.
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

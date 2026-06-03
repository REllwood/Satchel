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

import * as api from "@/lib/api";
import { useVault } from "@/features/vault/vault-store";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

function labelFor(n: api.CanvasNode): string {
  if (n.type === "file") return n.file ?? "(file)";
  if (n.type === "text") return n.text ?? "";
  if (n.type === "link") return n.url ?? "(link)";
  return n.label ?? "Group";
}

function toFlow(canvas: api.Canvas): { nodes: Node[]; edges: Edge[] } {
  return {
    nodes: canvas.nodes.map((n) => ({
      id: n.id,
      position: { x: n.x, y: n.y },
      data: { label: labelFor(n), canvasNode: n },
      style: { width: n.width, height: n.height },
    })),
    edges: canvas.edges.map((e) => ({
      id: e.id,
      source: e.fromNode,
      target: e.toNode,
      label: e.label,
    })),
  };
}

function fromFlow(nodes: Node[], edges: Edge[]): api.Canvas {
  return {
    nodes: nodes.map((rn) => {
      const cn = (rn.data as { canvasNode: api.CanvasNode }).canvasNode;
      const w = rn.measured?.width ?? (rn.style?.width as number) ?? cn.width;
      const h = rn.measured?.height ?? (rn.style?.height as number) ?? cn.height;
      return {
        ...cn,
        x: Math.round(rn.position.x),
        y: Math.round(rn.position.y),
        width: Math.round(w),
        height: Math.round(h),
      };
    }),
    edges: edges.map((re) => ({
      id: re.id,
      fromNode: re.source,
      toNode: re.target,
      label: typeof re.label === "string" ? re.label : undefined,
    })),
  };
}

export function CanvasView({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { select } = useVault();
  const [canvases, setCanvases] = useState<string[]>([]);
  const [path, setPath] = useState<string | null>(null);
  const [nodes, setNodes] = useState<Node[]>([]);
  const [edges, setEdges] = useState<Edge[]>([]);
  const ready = useRef(false);

  useEffect(() => {
    if (!open) return;
    api.listCanvases().then(setCanvases).catch(() => setCanvases([]));
  }, [open]);

  const load = useCallback(async (rel: string) => {
    ready.current = false;
    const canvas = await api.readCanvas(rel);
    const flow = toFlow(canvas);
    setNodes(flow.nodes);
    setEdges(flow.edges);
    setPath(rel);
    // allow autosave after the initial layout settles
    setTimeout(() => (ready.current = true), 300);
  }, []);

  // Debounced autosave.
  useEffect(() => {
    if (!ready.current || !path) return;
    const timer = setTimeout(() => {
      api.writeCanvas(path, fromFlow(nodes, edges)).catch(() => {});
    }, 500);
    return () => clearTimeout(timer);
  }, [nodes, edges, path]);

  const onNodesChange = useCallback(
    (changes: NodeChange[]) => setNodes((nds) => applyNodeChanges(changes, nds)),
    [],
  );
  const onEdgesChange = useCallback(
    (changes: EdgeChange[]) => setEdges((eds) => applyEdgeChanges(changes, eds)),
    [],
  );
  const onConnect = useCallback(
    (c: Connection) => setEdges((eds) => addEdge({ ...c, id: crypto.randomUUID() }, eds)),
    [],
  );

  const addCard = () => {
    const id = crypto.randomUUID();
    const cn: api.CanvasNode = {
      id,
      type: "text",
      text: "New card",
      x: 40,
      y: 40,
      width: 220,
      height: 100,
    };
    setNodes((nds) => [
      ...nds,
      {
        id,
        position: { x: cn.x, y: cn.y },
        data: { label: cn.text!, canvasNode: cn },
        style: { width: cn.width, height: cn.height },
      },
    ]);
  };

  const newCanvas = async () => {
    const rel = await api.createCanvas("Untitled.canvas");
    setCanvases((c) => [...c, rel].sort());
    await load(rel);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[88vh] w-[92vw] max-w-[92vw] flex-col gap-0 p-0">
        <DialogHeader className="flex-row items-center gap-2 space-y-0 border-b px-4 py-2">
          <DialogTitle className="mr-2">Canvas</DialogTitle>
          <select
            value={path ?? ""}
            onChange={(e) => e.target.value && load(e.target.value)}
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
          <Button size="sm" variant="outline" onClick={newCanvas}>
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
                const cn = (node.data as { canvasNode: api.CanvasNode }).canvasNode;
                if (cn.type === "file" && cn.file) {
                  select(cn.file);
                  onOpenChange(false);
                }
              }}
              fitView
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

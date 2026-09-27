import { useEffect, useMemo, useRef, useState } from "react";
import ForceGraph2D, { type ForceGraphMethods } from "react-force-graph-2d";

import * as api from "@/lib/api";
import { useVault } from "@/features/vault/vault-store";
import { useTheme } from "@/components/theme-provider";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

interface GNode {
  id: string;
  name: string;
}
interface GLink {
  source: string;
  target: string;
}
type Scope = "global" | "local";

const idOf = (end: string | { id: string }) =>
  typeof end === "string" ? end : end.id;

export function GraphView({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { select, selected } = useVault();
  const { resolved } = useTheme();
  const [data, setData] = useState<{ nodes: GNode[]; links: GLink[] }>({
    nodes: [],
    links: [],
  });
  const [scope, setScope] = useState<Scope>("global");
  // Callback ref: the dialog mounts its content after `open` flips, so an
  // effect keyed on `open` would run before the container exists.
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  const graphRef = useRef<ForceGraphMethods | undefined>(undefined);
  const [size, setSize] = useState({ w: 800, h: 600 });

  useEffect(() => {
    if (!open) return;
    api
      .getGraph()
      .then((g) =>
        setData({
          nodes: g.nodes.map((n) => ({ id: n.id, name: n.title || n.id })),
          links: g.edges.map((e) => ({ source: e.source, target: e.target })),
        }),
      )
      .catch(() => {});
  }, [open]);

  useEffect(() => {
    if (!container) return;
    const measure = () => setSize({ w: container.clientWidth, h: container.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(container);
    return () => ro.disconnect();
  }, [container]);

  const colors =
    resolved === "dark"
      ? { node: "#818cf8", active: "#c4b5fd", link: "rgba(255,255,255,0.18)", label: "#d4d4e8" }
      : { node: "#6366f1", active: "#4f46e5", link: "rgba(0,0,0,0.16)", label: "#3a3a52" };

  // Spread linked notes out, and stop isolated notes drifting off-screen.
  useEffect(() => {
    const fg = graphRef.current;
    if (!fg) return;
    const charge = fg.d3Force("charge") as { strength?: (v: number) => unknown; distanceMax?: (v: number) => unknown } | undefined;
    charge?.strength?.(-120);
    charge?.distanceMax?.(140);
    (fg.d3Force("link") as { distance?: (v: number) => unknown } | undefined)?.distance?.(90);
    fg.d3ReheatSimulation();
  }, [data, open]);

  // Well-linked notes draw larger.
  const degree = useMemo(() => {
    const d = new Map<string, number>();
    for (const l of data.links) {
      for (const end of [idOf(l.source as unknown as string), idOf(l.target as unknown as string)]) {
        d.set(end, (d.get(end) ?? 0) + 1);
      }
    }
    return d;
  }, [data]);
  const nodeVal = (id: string) => 1 + (degree.get(id) ?? 0) * 0.6;

  const view = useMemo(() => {
    if (scope === "global" || !selected) return data;
    const keep = new Set<string>([selected]);
    for (const l of data.links) {
      const s = idOf(l.source as unknown as string);
      const t = idOf(l.target as unknown as string);
      if (s === selected) keep.add(t);
      if (t === selected) keep.add(s);
    }
    return {
      nodes: data.nodes.filter((n) => keep.has(n.id)),
      links: data.links.filter((l) => {
        const s = idOf(l.source as unknown as string);
        const t = idOf(l.target as unknown as string);
        return keep.has(s) && keep.has(t);
      }),
    };
  }, [data, scope, selected]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="flex h-[85vh] w-[90vw] max-w-[90vw] flex-col gap-0 p-0 sm:max-w-[90vw]"
        showCloseButton
      >
        <DialogHeader className="flex-row items-center justify-between space-y-0 border-b px-4 py-2">
          <DialogTitle>
            Graph{" "}
            <span className="text-xs font-normal text-muted-foreground">
              {view.nodes.length} notes · {view.links.length} links
            </span>
          </DialogTitle>
          <div className="flex gap-1 pr-6">
            <Button
              size="sm"
              variant={scope === "global" ? "default" : "ghost"}
              onClick={() => setScope("global")}
            >
              Global
            </Button>
            <Button
              size="sm"
              variant={scope === "local" ? "default" : "ghost"}
              disabled={!selected}
              onClick={() => setScope("local")}
            >
              Local
            </Button>
          </div>
        </DialogHeader>
        <div ref={setContainer} className="min-h-0 flex-1 overflow-hidden">
          {open && (
            <ForceGraph2D
              ref={graphRef}
              width={size.w}
              height={size.h}
              graphData={view}
              nodeLabel="name"
              nodeRelSize={5}
              nodeVal={(n: object) => nodeVal((n as GNode).id)}
              nodeColor={(n: object) =>
                (n as GNode).id === selected ? colors.active : colors.node
              }
              nodeCanvasObjectMode={() => "after"}
              nodeCanvasObject={(n: object, ctx: CanvasRenderingContext2D, scale: number) => {
                // Title under each node, kept a constant on-screen size.
                if (scale < 0.6) return;
                const node = n as GNode & { x?: number; y?: number };
                ctx.font = `${12 / scale}px ui-sans-serif, system-ui, sans-serif`;
                ctx.textAlign = "center";
                ctx.textBaseline = "top";
                ctx.fillStyle = colors.label;
                const r = Math.sqrt(nodeVal(node.id)) * 5;
                ctx.fillText(node.name, node.x ?? 0, (node.y ?? 0) + r + 3 / scale);
              }}
              linkColor={() => colors.link}
              linkWidth={1}
              cooldownTicks={120}
              onEngineStop={() => graphRef.current?.zoomToFit(400, 60)}
              onNodeClick={(n: object) => {
                select((n as GNode).id);
                onOpenChange(false);
              }}
            />
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

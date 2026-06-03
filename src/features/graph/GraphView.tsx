import { useEffect, useMemo, useRef, useState } from "react";
import ForceGraph2D from "react-force-graph-2d";

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
  const containerRef = useRef<HTMLDivElement>(null);
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
    if (!open) return;
    const el = containerRef.current;
    if (!el) return;
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [open]);

  const colors =
    resolved === "dark"
      ? { node: "#818cf8", active: "#c4b5fd", link: "rgba(255,255,255,0.13)" }
      : { node: "#6366f1", active: "#4f46e5", link: "rgba(0,0,0,0.13)" };

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
        className="flex h-[85vh] w-[90vw] max-w-[90vw] flex-col gap-0 p-0"
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
        <div ref={containerRef} className="min-h-0 flex-1 overflow-hidden">
          {open && (
            <ForceGraph2D
              width={size.w}
              height={size.h}
              graphData={view}
              nodeLabel="name"
              nodeRelSize={5}
              nodeColor={(n: object) =>
                (n as GNode).id === selected ? colors.active : colors.node
              }
              linkColor={() => colors.link}
              linkWidth={1}
              cooldownTicks={120}
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

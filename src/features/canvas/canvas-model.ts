import type { Edge, Node } from "@xyflow/react";

import type * as api from "@/lib/api";

// Conversion between JSONCanvas files and React Flow. Satchel moves and
// resizes cards, adds and edits text cards and adds or removes connections,
// so saving starts from the objects it loaded and changes only those parts.
// Everything else (colours, edge sides and labels, fields from other apps) is
// kept.

export interface CardData extends Record<string, unknown> {
  label: string;
  canvasNode: api.CanvasNode;
}
export interface LinkData extends Record<string, unknown> {
  canvasEdge?: api.CanvasEdge;
}

function labelFor(n: api.CanvasNode): string {
  if (n.type === "file") return n.file?.split("/").pop()?.replace(/\.md$/i, "") ?? "(file)";
  if (n.type === "text") return n.text ?? "";
  if (n.type === "link") return n.url ?? "(link)";
  return n.label ?? "Group";
}

/** Text cards use the editable `text` card. Other cards use React Flow's default. */
export function toFlowNode(n: api.CanvasNode): Node<CardData> {
  return {
    id: n.id,
    type: n.type === "text" ? "text" : "default",
    position: { x: n.x, y: n.y },
    data: { label: labelFor(n), canvasNode: n },
    className: `canvas-${n.type}`,
    width: n.width,
    height: n.height,
  };
}

/** The card with its text replaced, ready to save. */
export function withCardText(node: Node, text: string): Node {
  const data = node.data as CardData;
  return { ...node, data: { ...data, label: text, canvasNode: { ...data.canvasNode, text } } };
}

export function toFlow(canvas: api.Canvas): { nodes: Node<CardData>[]; edges: Edge<LinkData>[] } {
  return {
    nodes: canvas.nodes.map(toFlowNode),
    edges: canvas.edges.map((e) => ({
      id: e.id,
      source: e.fromNode,
      target: e.toNode,
      label: e.label,
      data: { canvasEdge: e },
    })),
  };
}

/** Back to JSONCanvas, merging into `base` (the canvas as it was loaded). */
export function fromFlow(base: api.Canvas, nodes: Node[], edges: Edge[]): api.Canvas {
  return {
    ...base,
    nodes: nodes.map((rn) => {
      const cn = (rn.data as CardData).canvasNode;
      const width = rn.width ?? cn.width;
      const height = rn.height ?? cn.height;
      const moved = rn.position.x !== cn.x || rn.position.y !== cn.y;
      const resized = width !== cn.width || height !== cn.height;
      // Untouched cards keep their exact numbers, fractional ones included.
      if (!moved && !resized) return cn;
      return {
        ...cn,
        ...(moved && { x: Math.round(rn.position.x), y: Math.round(rn.position.y) }),
        ...(resized && { width: Math.round(width), height: Math.round(height) }),
      };
    }),
    edges: edges.map((re) => ({
      ...(re.data as LinkData | undefined)?.canvasEdge,
      id: re.id,
      fromNode: re.source,
      toNode: re.target,
    })),
  };
}

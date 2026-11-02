import { describe, expect, it } from "vitest";
import type { Edge } from "@xyflow/react";

import type { Canvas } from "@/lib/api";
import { fromFlow, toFlow, withCardText } from "./canvas-model";

const canvas: Canvas = {
  nodes: [
    { id: "a", type: "text", text: "Hi", x: 10.5, y: 20.25, width: 200, height: 80, color: "4" },
    {
      id: "b",
      type: "file",
      file: "Notes/Plan.md",
      x: 300,
      y: 0,
      width: 220,
      height: 64,
      subpath: "#Goals",
    },
  ],
  edges: [
    {
      id: "e1",
      fromNode: "a",
      toNode: "b",
      fromSide: "right",
      toSide: "left",
      color: "#ff0000",
      label: "needs",
      toEnd: "arrow",
    },
  ],
  metadata: { version: "1.0" },
};

describe("canvas model", () => {
  it("round-trips a canvas without losing anything Satchel does not edit", () => {
    const flow = toFlow(canvas);
    expect(fromFlow(canvas, flow.nodes, flow.edges)).toEqual(canvas);
  });

  it("changes only the position of a moved card", () => {
    const flow = toFlow(canvas);
    flow.nodes[0] = { ...flow.nodes[0], position: { x: 40.4, y: 59.6 } };
    const saved = fromFlow(canvas, flow.nodes, flow.edges);
    expect(saved.nodes[0]).toEqual({ ...canvas.nodes[0], x: 40, y: 60 });
    expect(saved.nodes[1]).toBe(canvas.nodes[1]);
  });

  it("saves a card's edited text and keeps its other fields", () => {
    const flow = toFlow(canvas);
    flow.nodes[0] = withCardText(flow.nodes[0], "Hello\nagain") as (typeof flow.nodes)[0];
    expect(flow.nodes[0].data.label).toBe("Hello\nagain");
    const saved = fromFlow(canvas, flow.nodes, flow.edges);
    expect(saved.nodes[0]).toEqual({ ...canvas.nodes[0], text: "Hello\nagain" });
    expect(saved.nodes[1]).toBe(canvas.nodes[1]);
  });

  it("uses the editable card for text cards only", () => {
    const flow = toFlow(canvas);
    expect(flow.nodes.map((n) => n.type)).toEqual(["text", "default"]);
    expect(flow.nodes[0]).toMatchObject({ width: 200, height: 80 });
  });

  it("saves a resized card's size, rounded, and keeps its position", () => {
    const flow = toFlow(canvas);
    flow.nodes[0] = { ...flow.nodes[0], width: 320.6, height: 140.2 };
    const saved = fromFlow(canvas, flow.nodes, flow.edges);
    expect(saved.nodes[0]).toEqual({ ...canvas.nodes[0], width: 321, height: 140 });
  });

  it("saves new connections and drops removed ones", () => {
    const flow = toFlow(canvas);
    const added: Edge = { id: "e2", source: "b", target: "a" };
    const saved = fromFlow(canvas, flow.nodes, [added]);
    expect(saved.edges).toEqual([{ id: "e2", fromNode: "b", toNode: "a" }]);
  });
});

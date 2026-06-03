import { describe, expect, it } from "vitest";

import type { NoteMeta } from "@/lib/api";
import { buildTree, noteLabel } from "./tree-utils";

const note = (rel_path: string, title = ""): NoteMeta => ({
  id: 0,
  rel_path,
  title,
  mtime: 0,
});

describe("buildTree", () => {
  it("nests folders and sorts directories before files", () => {
    const tree = buildTree([
      note("b.md"),
      note("a/x.md"),
      note("a/y.md"),
      note("a.md"),
    ]);
    expect(tree.children.map((c) => c.name)).toEqual(["a", "a.md", "b.md"]);
    const dir = tree.children[0];
    expect(dir.isDir).toBe(true);
    expect(dir.children.map((c) => c.name)).toEqual(["x.md", "y.md"]);
  });

  it("noteLabel prefers the title, else the filename stem", () => {
    const titled = buildTree([note("notes/foo.md", "My Title")]);
    expect(noteLabel(titled.children[0].children[0])).toBe("My Title");
    const untitled = buildTree([note("bar.md")]);
    expect(noteLabel(untitled.children[0])).toBe("bar");
  });
});

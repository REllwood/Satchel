import { describe, expect, it } from "vitest";

import type { NoteMeta } from "@/lib/api";
import {
  buildTree,
  cleanFolderName,
  noteLabel,
  parentFolder,
  rowKey,
  visibleRows,
} from "./tree-utils";

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

  it("shows empty folders and merges them with note folders", () => {
    const tree = buildTree([note("work/plan.md")], ["work", "work/archive", "ideas"]);
    expect(tree.children.map((c) => c.name)).toEqual(["ideas", "work"]);
    const work = tree.children[1];
    expect(work.children.map((c) => c.name)).toEqual(["archive", "plan.md"]);
    expect(work.children[0].path).toBe("work/archive");
    expect(work.children[0].isDir).toBe(true);
  });

  it("noteLabel prefers the title, else the filename stem", () => {
    const titled = buildTree([note("notes/foo.md", "My Title")]);
    expect(noteLabel(titled.children[0].children[0])).toBe("My Title");
    const untitled = buildTree([note("bar.md")]);
    expect(noteLabel(untitled.children[0])).toBe("bar");
  });
});

describe("folder helpers", () => {
  it("parentFolder returns the containing folder", () => {
    expect(parentFolder("a/b/c.md")).toBe("a/b");
    expect(parentFolder("c.md")).toBe("");
  });

  it("cleanFolderName normalises and rejects unsafe names", () => {
    expect(cleanFolderName("  Projects ")).toBe("Projects");
    expect(cleanFolderName("a//b\\c")).toBe("a/b/c");
    expect(cleanFolderName("../escape")).toBeNull();
    expect(cleanFolderName(".hidden")).toBeNull();
    expect(cleanFolderName("   ")).toBeNull();
  });
});

describe("visibleRows", () => {
  const tree = buildTree(
    [note("a/x.md"), note("a/b/y.md"), note("top.md")],
    ["a", "a/b", "empty"],
  );
  const describeRows = (collapsed: string[]) =>
    visibleRows(tree, new Set(collapsed)).map(
      (r) => `${r.level} ${rowKey(r.node)} ${r.parent ?? "-"} ${r.posInSet}/${r.setSize}`,
    );

  it("lists rows in order with their level, parent and position", () => {
    expect(describeRows([])).toEqual([
      "1 d:a - 1/3",
      "2 d:a/b a 1/2",
      "3 f:a/b/y.md a/b 1/1",
      "2 f:a/x.md a 2/2",
      "1 d:empty - 2/3",
      "1 f:top.md - 3/3",
    ]);
  });

  it("leaves out what is inside collapsed folders", () => {
    expect(describeRows(["a/b"])).toEqual([
      "1 d:a - 1/3",
      "2 d:a/b a 1/2",
      "2 f:a/x.md a 2/2",
      "1 d:empty - 2/3",
      "1 f:top.md - 3/3",
    ]);
    expect(describeRows(["a"])).toEqual(["1 d:a - 1/3", "1 d:empty - 2/3", "1 f:top.md - 3/3"]);
  });
});


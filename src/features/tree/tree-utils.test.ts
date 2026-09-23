import { describe, expect, it } from "vitest";

import type { NoteMeta } from "@/lib/api";
import { buildTree, cleanFolderName, noteLabel, parentFolder } from "./tree-utils";

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

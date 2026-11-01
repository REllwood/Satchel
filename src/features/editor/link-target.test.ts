import { describe, expect, it } from "vitest";

import { findHeading, isNoteTarget, linkAction, wikilinkAt } from "./link-target";

describe("linkAction", () => {
  it("opens a note, ignoring the heading and alias", () => {
    expect(linkAction("Meeting")).toEqual({ kind: "note", target: "Meeting" });
    expect(linkAction("Work/Plan#Goals|the plan")).toEqual({ kind: "note", target: "Work/Plan" });
    expect(linkAction(" Spaced ")).toEqual({ kind: "note", target: "Spaced" });
  });

  it("jumps to a heading in the open note for [[#Heading]]", () => {
    expect(linkAction("#Goals")).toEqual({ kind: "heading", heading: "Goals" });
    expect(linkAction("#Goals|see goals")).toEqual({ kind: "heading", heading: "Goals" });
    expect(linkAction("#Plan#Risks")).toEqual({ kind: "heading", heading: "Risks" });
  });

  it("never creates a note for an empty or hidden target", () => {
    expect(linkAction("#")).toBeNull();
    expect(linkAction(" ")).toBeNull();
    expect(linkAction(".md")).toBeNull();
    expect(linkAction(".hidden")).toBeNull();
    expect(linkAction("../escape")).toBeNull();
  });

  it("treats attachments as files, not notes", () => {
    expect(linkAction("x.png")).toEqual({ kind: "file", target: "x.png" });
    expect(linkAction("docs/report.PDF|Report")).toEqual({ kind: "file", target: "docs/report.PDF" });
    expect(linkAction("Board.canvas")).toEqual({ kind: "file", target: "Board.canvas" });
  });

  it("keeps dotted note names that are not file types", () => {
    expect(linkAction("Node.js")).toEqual({ kind: "note", target: "Node.js" });
    expect(linkAction("2026.09.22 Sync")).toEqual({ kind: "note", target: "2026.09.22 Sync" });
    expect(linkAction("Note.md")).toEqual({ kind: "note", target: "Note.md" });
  });
});

describe("isNoteTarget", () => {
  it("accepts note names and rejects files and hidden paths", () => {
    expect(isNoteTarget("Ideas")).toBe(true);
    expect(isNoteTarget("Journal/Ideas.md")).toBe(true);
    expect(isNoteTarget("photo.jpeg")).toBe(false);
    expect(isNoteTarget("")).toBe(false);
    expect(isNoteTarget("a/.b")).toBe(false);
  });
});

describe("wikilinkAt", () => {
  const line = "See [[One]] and [[Two|2]].";
  it("finds the link under the cursor, brackets included", () => {
    expect(wikilinkAt(line, 4)).toBe("One");
    expect(wikilinkAt(line, 8)).toBe("One");
    expect(wikilinkAt(line, 11)).toBe("One");
    expect(wikilinkAt(line, 20)).toBe("Two|2");
  });

  it("counts a cursor only between the outer brackets when asked", () => {
    expect(wikilinkAt(line, 4, true)).toBeNull();
    expect(wikilinkAt(line, 5, true)).toBe("One");
    expect(wikilinkAt(line, 10, true)).toBe("One");
    expect(wikilinkAt(line, 11, true)).toBeNull();
  });

  it("returns null between links", () => {
    expect(wikilinkAt(line, 13)).toBeNull();
    expect(wikilinkAt("no links", 2)).toBeNull();
  });
});

describe("findHeading", () => {
  const doc = [
    "# Title",
    "",
    "```md",
    "## Goals",
    "```",
    "",
    "## Goals ##",
    "text",
    "Setext heading",
    "---",
    "### **Bold** risks",
  ].join("\n");

  it("finds an ATX heading outside code, ignoring case and closing hashes", () => {
    expect(findHeading(doc, "goals")).toBe(doc.indexOf("## Goals ##"));
    expect(findHeading(doc, "Title")).toBe(0);
  });

  it("finds setext headings and headings with formatting", () => {
    expect(findHeading(doc, "Setext heading")).toBe(doc.indexOf("Setext heading"));
    expect(findHeading(doc, "Bold risks")).toBe(doc.indexOf("### **Bold**"));
  });

  it("returns null when there is no such heading", () => {
    expect(findHeading(doc, "Missing")).toBeNull();
    expect(findHeading("```\n# In code\n```", "In code")).toBeNull();
    expect(findHeading(doc, "")).toBeNull();
  });
});

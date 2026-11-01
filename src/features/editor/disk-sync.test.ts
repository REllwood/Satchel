import { describe, expect, it } from "vitest";

import {
  DiskSync,
  findMovedNote,
  moveCandidates,
  recoveryPath,
  type VaultChange,
} from "./disk-sync";

const NOTE = "Journal/Ideas.md";

describe("DiskSync", () => {
  it("ignores the echo of a save when the user has typed since", () => {
    const sync = new DiskSync();
    sync.opened(NOTE, "a");
    sync.saving(NOTE, "ab");
    sync.saved(NOTE, "ab", true);
    // The watcher reports the save after more typing.
    expect(sync.diskChanged(NOTE, "ab", "abc", true)).toBe("ignore");
  });

  it("ignores a save that has landed but not yet returned", () => {
    const sync = new DiskSync();
    sync.opened(NOTE, "a");
    sync.saving(NOTE, "ab");
    sync.saving(NOTE, "abc");
    expect(sync.diskChanged(NOTE, "ab", "abcd", true)).toBe("ignore");
    expect(sync.diskChanged(NOTE, "abc", "abcd", true)).toBe("ignore");
  });

  it("reloads an outside edit when there are no unsaved changes", () => {
    const sync = new DiskSync();
    sync.opened(NOTE, "a");
    expect(sync.diskChanged(NOTE, "from my phone", "a", false)).toBe("reload");
  });

  it("reports a conflict for an outside edit over unsaved changes, once", () => {
    const sync = new DiskSync();
    sync.opened(NOTE, "a");
    expect(sync.diskChanged(NOTE, "from my phone", "ab", true)).toBe("conflict");
    expect(sync.diskChanged(NOTE, "from my phone", "ab", true)).toBe("ignore");
  });

  it("treats restoring an older version from outside as an outside edit", () => {
    const sync = new DiskSync();
    sync.opened(NOTE, "original");
    sync.saving(NOTE, "edited");
    sync.saved(NOTE, "edited", true);
    expect(sync.diskChanged(NOTE, "original", "edited", false)).toBe("reload");
  });

  it("does not count a failed save as being on disk", () => {
    const sync = new DiskSync();
    sync.opened(NOTE, "a");
    sync.saving(NOTE, "ab");
    sync.saved(NOTE, "ab", false);
    expect(sync.diskChanged(NOTE, "a", "ab", true)).toBe("ignore");
    expect(sync.diskChanged(NOTE, "ab", "abc", true)).toBe("conflict");
  });

  it("ignores saves and changes for other notes", () => {
    const sync = new DiskSync();
    sync.opened(NOTE, "a");
    sync.saving("Other.md", "x");
    sync.saved("Other.md", "x", true);
    expect(sync.diskChanged("Other.md", "y", "a", true)).toBe("ignore");
    expect(sync.diskChanged(NOTE, "x", "a", false)).toBe("reload");
  });

  it("follows the note when it moves on disk", () => {
    const sync = new DiskSync();
    sync.opened(NOTE, "a");
    sync.saving(NOTE, "ab");
    sync.moved(NOTE, "Archive/Ideas.md");
    expect(sync.known(NOTE)).toEqual([]);
    expect(sync.known("Archive/Ideas.md")).toEqual(["a", "ab"]);
    sync.saved("Archive/Ideas.md", "ab", true);
    expect(sync.diskChanged("Archive/Ideas.md", "ab", "ab", false)).toBe("ignore");
  });

  it("compares CRLF text as it is on disk", () => {
    const sync = new DiskSync();
    sync.opened(NOTE, "a\r\nb\r\n");
    sync.saving(NOTE, "a\r\nb\r\nc\r\n");
    sync.saved(NOTE, "a\r\nb\r\nc\r\n", true);
    // The echo of the editor's own CRLF save.
    expect(sync.diskChanged(NOTE, "a\r\nb\r\nc\r\n", "a\r\nb\r\nc\r\n", false)).toBe("ignore");
    expect(sync.diskChanged(NOTE, "a\r\nX\r\n", "a\r\nb\r\nc\r\n", false)).toBe("reload");
  });
});

describe("moved or deleted notes", () => {
  const changes: VaultChange[] = [
    { rel_path: NOTE, removed: true },
    { rel_path: "Other/Plan.md", removed: false },
    { rel_path: "Archive/Ideas.md", removed: false },
    { rel_path: "Archive/board.canvas", removed: false },
    { rel_path: "Gone.md", removed: true },
  ];

  it("checks added notes with the same name first", () => {
    expect(moveCandidates(NOTE, changes)).toEqual(["Archive/Ideas.md", "Other/Plan.md"]);
  });

  it("finds the note whose text matches what was on disk", async () => {
    const disk: Record<string, string> = {
      "Other/Plan.md": "something else",
      "Archive/Ideas.md": "my ideas\r\n",
    };
    const read = async (p: string) => disk[p];
    expect(await findMovedNote(NOTE, changes, ["my ideas\n"], read)).toBe("Archive/Ideas.md");
  });

  it("matches the editor text or a renamed note", async () => {
    const renamed: VaultChange[] = [
      { rel_path: NOTE, removed: true },
      { rel_path: "Journal/Thoughts.md", removed: false },
    ];
    const read = async () => "typed";
    expect(await findMovedNote(NOTE, renamed, ["on disk", "typed"], read)).toBe(
      "Journal/Thoughts.md",
    );
  });

  it("reports a delete when nothing added matches", async () => {
    const read = async (p: string) => {
      if (p === "Other/Plan.md") throw new Error("gone");
      return "different";
    };
    expect(await findMovedNote(NOTE, changes, ["my ideas"], read)).toBeNull();
    expect(await findMovedNote(NOTE, [{ rel_path: NOTE, removed: true }], ["x"], read)).toBeNull();
    expect(await findMovedNote(NOTE, changes, [], read)).toBeNull();
  });

  it("puts recovered edits beside the deleted note", () => {
    expect(recoveryPath(NOTE, 1700000000000)).toBe("Journal/Ideas.recovered-1700000000000.md");
    expect(recoveryPath("Top.md", 5)).toBe("Top.recovered-5.md");
  });
});

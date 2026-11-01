import { describe, expect, it } from "vitest";

import { DiskSync } from "./disk-sync";

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
});

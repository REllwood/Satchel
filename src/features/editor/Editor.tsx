import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";

import * as api from "@/lib/api";
import { isTauri } from "@/lib/ipc";
import { useVault } from "@/features/vault/vault-store";
import { MarkdownEditor } from "./MarkdownEditor";
import { wikilinkExtension } from "./wikilink";
import { currentNote } from "./note-context";
import { clearImageCache } from "./images";
import { registerFlush } from "./pending-save";
import { DiskSync, findMovedNote, recoveryPath, type VaultChange } from "./disk-sync";
import { detectEol, toLF, withEol, type Eol } from "./eol";

type SaveState = "idle" | "saving" | "saved";

/** An unsaved edit: the whole note, in its own line endings, and where it goes. */
interface Edit {
  path: string;
  body: string;
}

/** Note editor: loads the selected note, renders CodeMirror, and autosaves. */
export function Editor() {
  const { selected, select, notes, openOrCreate } = useVault();
  // Keep live refs so completion and links use the current note list (and the
  // `openOrCreate` that closes over it) without rebuilding the editor.
  const notesRef = useRef(notes);
  notesRef.current = notes;
  const openRef = useRef(openOrCreate);
  openRef.current = openOrCreate;
  const activePath = useRef<string | null>(null);
  const editorExtensions = useMemo(
    () => [
      wikilinkExtension({
        getNotes: () => notesRef.current,
        onOpen: (target) => void openRef.current(target),
      }),
      currentNote.of(() => activePath.current),
    ],
    [],
  );

  // `content` is the editor text, always with LF line endings. `loadedFor` is
  // the note it belongs to. `session` keys the editor, so a note that moves on
  // disk keeps its editor (cursor and undo history) at the new path.
  const [content, setContent] = useState("");
  const [loadedFor, setLoadedFor] = useState<string | null>(null);
  const [session, setSession] = useState(0);
  const [dirty, setDirty] = useState(false);
  const [save, setSave] = useState<SaveState>("idle");
  // Live mirrors for the vault-changed listener (avoids stale closures).
  const contentRef = useRef(content);
  contentRef.current = content;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  const loadedRef = useRef(loadedFor);
  loadedRef.current = loadedFor;
  /** The open note's line ending, kept when it is written back. */
  const eol = useRef<Eol>("\n");
  // The unsaved edit, tied to the note it belongs to, and its debounce timer.
  const pending = useRef<Edit | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Saves run one at a time, in order, so an older save can never land last.
  const saves = useRef<Promise<void>>(Promise.resolve());
  const diskSync = useRef(new DiskSync());
  // While a note that vanished from disk is checked for a move, its edits wait
  // here instead of being written, so they never recreate the old path.
  const held = useRef<string | null>(null);
  const heldEdit = useRef<Edit | null>(null);
  /** The new path of a note that moved on disk, set just before selecting it. */
  const movedTo = useRef<string | null>(null);

  /** Write `edit` after any earlier save. */
  const write = useCallback((edit: Edit): Promise<void> => {
    diskSync.current.saving(edit.path, edit.body);
    saves.current = saves.current.then(async () => {
      if (edit.path === held.current) {
        // The note vanished while this waited: the move check decides where it goes.
        heldEdit.current ??= edit;
        diskSync.current.saved(edit.path, edit.body, false);
        return;
      }
      let ok = false;
      try {
        await api.writeNote(edit.path, edit.body);
        ok = true;
        if (activePath.current === edit.path && !pending.current) {
          setSave("saved");
          setDirty(false);
        }
      } catch (e) {
        toast.error(`Save failed: ${e}`);
        pending.current ??= edit; // keep it for the next attempt
        if (activePath.current === edit.path) setSave("idle");
      } finally {
        diskSync.current.saved(edit.path, edit.body, ok);
      }
    });
    return saves.current;
  }, []);

  /** Write the pending edit (to the note it was typed in) after any earlier save. */
  const flush = useCallback((): Promise<void> => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    const edit = pending.current;
    if (!edit) return saves.current;
    pending.current = null;
    if (edit.path === held.current) {
      heldEdit.current = edit;
      return saves.current;
    }
    return write(edit);
  }, [write]);

  const schedule = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => void flush(), 600);
  }, [flush]);

  const onEdit = useCallback(
    (body: string) => {
      const path = activePath.current;
      if (!path) return;
      // Update the mirrors now: a disk change handled before the next render
      // must not mistake this keystroke for a clean editor.
      contentRef.current = body;
      dirtyRef.current = true;
      setContent(body);
      setDirty(true);
      setSave("saving");
      pending.current = { path, body: withEol(body, eol.current) };
      schedule();
    },
    [schedule],
  );

  useEffect(() => {
    if (selected !== null && movedTo.current === selected) {
      // Followed a move on disk: the editor already shows this note.
      movedTo.current = null;
      return;
    }
    movedTo.current = null;
    // Save what was typed in the previous note before loading the next one.
    const saved = flush();
    activePath.current = selected;
    if (!selected) {
      setContent("");
      setLoadedFor(null);
      setDirty(false);
      return;
    }
    let cancelled = false;
    // Read once queued saves are done, in case one of them is for this note.
    saved
      .then(() => (cancelled ? null : api.readNote(selected)))
      .then((n) => {
        if (!n || cancelled || activePath.current !== selected) return;
        eol.current = detectEol(n.content);
        diskSync.current.opened(selected, n.content);
        setContent(toLF(n.content));
        setLoadedFor(selected);
        setSession((s) => s + 1);
        setDirty(false);
        setSave("idle");
      })
      .catch((e) => !cancelled && toast.error(`Couldn't open note: ${e}`));
    return () => {
      cancelled = true;
    };
  }, [selected, flush]);

  // Don't lose the last keystrokes when the app is hidden or closed.
  useEffect(() => {
    const unregister = registerFlush(flush);
    const onHide = () => void flush();
    window.addEventListener("blur", onHide);
    window.addEventListener("beforeunload", onHide);
    document.addEventListener("visibilitychange", onHide);
    return () => {
      window.removeEventListener("blur", onHide);
      window.removeEventListener("beforeunload", onHide);
      document.removeEventListener("visibilitychange", onHide);
      unregister();
      void flush();
    };
  }, [flush]);

  // Cached images belong to this vault.
  useEffect(() => () => clearImageCache(), []);

  /** Bring `path` up to date with `disk`, never clobbering unsaved edits. */
  const reconcile = useCallback(async (path: string, disk: string) => {
    const change = diskSync.current.diskChanged(
      path,
      disk,
      withEol(contentRef.current, eol.current),
      dirtyRef.current,
    );
    if (change === "ignore") return; // our own save, or nothing new
    if (change === "reload") {
      // No local edits: silently load the new version.
      eol.current = detectEol(disk);
      setContent(toLF(disk));
      return;
    }
    // Conflict: keep the user's edits in the editor, but preserve the
    // external version to a sibling file so nothing is lost.
    const conflictPath = `${path.replace(/\.md$/i, "")}.conflict-${Date.now()}.md`;
    await api.writeNote(conflictPath, disk);
    toast.warning(
      `"${path}" changed on disk while you had unsaved edits. The external version was saved to ${conflictPath}.`,
      { duration: 10000 },
    );
  }, []);

  /** Put back an edit that was held, unless a newer one for its note is pending. */
  const restore = useCallback(
    (edit: Edit) => {
      if (!pending.current) {
        pending.current = edit;
        schedule();
      } else if (pending.current.path !== edit.path) {
        void write(edit);
      }
    },
    [schedule, write],
  );

  /**
   * The open note `path` was reported removed. If a note added in the same
   * batch holds its text, it moved: follow it and carry any unsaved edit
   * there. Otherwise it was deleted: keep unsaved edits in a recovery note
   * beside it, then close it. Nothing is ever written to the old path.
   */
  const vanished = useCallback(
    async (path: string, changes: VaultChange[]) => {
      held.current = path;
      if (pending.current?.path === path) {
        if (timer.current) clearTimeout(timer.current);
        timer.current = null;
        heldEdit.current = pending.current;
        pending.current = null;
      }
      // The newest unsaved edit typed in `path`, which also ends the hold.
      const takeEdit = () => {
        const edit = pending.current?.path === path ? pending.current : heldEdit.current;
        if (pending.current?.path === path) pending.current = null;
        heldEdit.current = null;
        held.current = null;
        return edit;
      };

      // Apps that save by replacing the file report it removed, though it is back.
      const disk = await api.readNote(path).then(
        (n) => n.content,
        () => null,
      );
      if (disk !== null) {
        const edit = takeEdit();
        if (edit) restore(edit);
        await reconcile(path, disk).catch(() => {});
        return;
      }

      const editorText = loadedRef.current === path ? contentRef.current : null;
      const known = [...diskSync.current.known(path)];
      if (editorText !== null) known.push(editorText);
      const to = await findMovedNote(
        path,
        changes,
        known,
        async (p) => (await api.readNote(p)).content,
      );
      const edit = takeEdit();
      const viewing = activePath.current === path;

      if (to) {
        diskSync.current.moved(path, to);
        if (viewing) {
          activePath.current = to;
          movedTo.current = to;
          setLoadedFor(to);
          select(to);
        }
        if (edit) void write({ path: to, body: edit.body });
        return;
      }

      if (edit) {
        const body =
          viewing && loadedRef.current === path
            ? withEol(contentRef.current, eol.current)
            : edit.body;
        const copy = recoveryPath(path, Date.now());
        try {
          await api.writeNote(copy, body);
        } catch (e) {
          // Keep the edit open: saving it recreates the note rather than losing it.
          restore(edit);
          toast.error(
            `"${path}" was deleted outside Satchel, and your unsaved edits could not be saved beside it: ${e}. They are still open here.`,
            { duration: 10000 },
          );
          return;
        }
        toast.warning(
          `"${path}" was deleted outside Satchel. Your unsaved edits were saved to ${copy}.`,
          { duration: 10000 },
        );
      } else {
        toast(`"${path}" was deleted outside Satchel.`);
      }
      if (activePath.current === path) select(null);
    },
    [reconcile, restore, select, write],
  );

  // Reconcile external changes to the open note.
  useEffect(() => {
    if (!isTauri || !selected) return;
    const path = selected;
    const unlisten = listen<VaultChange[]>("vault-changed", (e) => {
      const hit = e.payload.find((c) => c.rel_path === path);
      // While a vanished note is being checked, later reports for it wait.
      if (!hit || held.current === path) return;
      if (hit.removed) {
        void vanished(path, e.payload);
        return;
      }
      api
        .readNote(path)
        .then((n) => reconcile(path, n.content))
        .catch(() => {});
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, [selected, vanished, reconcile]);

  if (!selected) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-center text-sm text-muted-foreground">
        Select a note from the sidebar, or create a new one to start writing.
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-8 shrink-0 items-center justify-between border-b px-3 text-xs text-muted-foreground">
        <span className="truncate font-medium">{selected}</span>
        <span aria-live="polite" className="tabular-nums">
          {save === "saving" ? "saving…" : dirty ? "unsaved" : save === "saved" ? "saved" : ""}
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden">
        {loadedFor === selected && (
          <MarkdownEditor
            key={session}
            doc={content}
            extensions={editorExtensions}
            onChange={onEdit}
          />
        )}
      </div>
    </div>
  );
}

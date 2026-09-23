import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";

import * as api from "@/lib/api";
import { isTauri } from "@/lib/ipc";
import { useVault } from "@/features/vault/vault-store";
import { MarkdownEditor } from "./MarkdownEditor";
import { wikilinkExtension } from "./wikilink";
import { registerFlush } from "./pending-save";

type SaveState = "idle" | "saving" | "saved";

/** Note editor: loads the selected note, renders CodeMirror, and autosaves. */
export function Editor() {
  const { selected, notes, openOrCreate } = useVault();
  // Keep a live ref so completion/links use the current note list without
  // rebuilding the editor.
  const notesRef = useRef(notes);
  notesRef.current = notes;
  const editorExtensions = useMemo(
    () =>
      wikilinkExtension({
        getNotes: () => notesRef.current,
        onOpen: openOrCreate,
      }),
    [openOrCreate],
  );

  const [content, setContent] = useState("");
  const [loadedFor, setLoadedFor] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [save, setSave] = useState<SaveState>("idle");
  const activePath = useRef<string | null>(null);
  // Live mirrors for the vault-changed listener (avoids stale closures).
  const contentRef = useRef(content);
  contentRef.current = content;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  // The unsaved edit, tied to the note it belongs to, and its debounce timer.
  const pending = useRef<{ path: string; body: string } | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  /** Write the pending edit now (to the note it was typed in). */
  const flush = useCallback(async () => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    const edit = pending.current;
    if (!edit) return;
    pending.current = null;
    try {
      await api.writeNote(edit.path, edit.body);
      if (activePath.current === edit.path && !pending.current) {
        setSave("saved");
        setDirty(false);
      }
    } catch (e) {
      toast.error(`Save failed: ${e}`);
      pending.current ??= edit; // keep it for the next attempt
      if (activePath.current === edit.path) setSave("idle");
    }
  }, []);

  const onEdit = useCallback(
    (path: string, body: string) => {
      setContent(body);
      setDirty(true);
      setSave("saving");
      pending.current = { path, body };
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => void flush(), 600);
    },
    [flush],
  );

  useEffect(() => {
    // Save what was typed in the previous note before loading the next one.
    void flush();
    activePath.current = selected;
    if (!selected) {
      setContent("");
      setLoadedFor(null);
      setDirty(false);
      return;
    }
    let cancelled = false;
    api
      .readNote(selected)
      .then((n) => {
        if (cancelled || activePath.current !== selected) return;
        setContent(n.content);
        setLoadedFor(selected);
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
    registerFlush(flush);
    const onHide = () => void flush();
    window.addEventListener("blur", onHide);
    window.addEventListener("beforeunload", onHide);
    document.addEventListener("visibilitychange", onHide);
    return () => {
      window.removeEventListener("blur", onHide);
      window.removeEventListener("beforeunload", onHide);
      document.removeEventListener("visibilitychange", onHide);
      registerFlush(null);
      void flush();
    };
  }, [flush]);

  // Reconcile external changes to the open note (never clobber unsaved edits).
  useEffect(() => {
    if (!isTauri || !selected) return;
    const path = selected;
    const unlisten = listen<{ rel_path: string; removed: boolean }[]>(
      "vault-changed",
      async (e) => {
        const hit = e.payload.find((c) => c.rel_path === path);
        if (!hit) return;
        if (hit.removed) {
          toast.warning(`"${path}" was deleted on disk.`);
          return;
        }
        try {
          const disk = (await api.readNote(path)).content;
          if (disk === contentRef.current) return; // our own save — no-op
          if (!dirtyRef.current) {
            setContent(disk); // no local edits: silently load the new version
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
        } catch {
          /* ignore */
        }
      },
    );
    return () => {
      void unlisten.then((f) => f());
    };
  }, [selected]);

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
            key={selected}
            doc={content}
            extensions={editorExtensions}
            onChange={(v) => onEdit(selected, v)}
          />
        )}
      </div>
    </div>
  );
}

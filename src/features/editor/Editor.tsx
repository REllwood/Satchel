import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";

import * as api from "@/lib/api";
import { isTauri } from "@/lib/ipc";
import { useVault } from "@/features/vault/vault-store";
import { MarkdownEditor } from "./MarkdownEditor";
import { wikilinkExtension } from "./wikilink";

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

  useEffect(() => {
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
  }, [selected]);

  useEffect(() => {
    if (!dirty || !selected) return;
    const path = selected;
    const body = content;
    setSave("saving");
    const timer = setTimeout(async () => {
      try {
        await api.writeNote(path, body);
        // Only mark clean if no newer edits arrived while the write was in flight.
        if (activePath.current === path && body === contentRef.current) {
          setSave("saved");
          setDirty(false);
        }
      } catch (e) {
        toast.error(`Save failed: ${e}`);
        setSave("idle");
      }
    }, 600);
    return () => clearTimeout(timer);
  }, [content, dirty, selected]);

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
            onChange={(v) => {
              setContent(v);
              setDirty(true);
            }}
          />
        )}
      </div>
    </div>
  );
}

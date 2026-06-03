import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";

import * as api from "@/lib/api";
import { isTauri } from "@/lib/ipc";

interface VaultContextValue {
  vault: api.VaultInfo | null;
  notes: api.NoteMeta[];
  selected: string | null;
  loading: boolean;
  embedding: boolean;
  /** Active tag filter (null = show all). */
  tagFilter: string | null;
  /** rel_paths matching the active tag filter. */
  tagPaths: Set<string>;
  setTagFilter: (tag: string | null) => Promise<void>;
  pickAndOpen: () => Promise<void>;
  openPath: (path: string) => Promise<void>;
  closeVault: () => Promise<void>;
  refresh: () => Promise<void>;
  select: (relPath: string | null) => void;
  createNote: () => Promise<void>;
  renameNote: (from: string, to: string) => Promise<void>;
  removeNote: (relPath: string) => Promise<void>;
  /** Resolve a `[[target]]` to an existing note (and select it), or create it. */
  openOrCreate: (target: string) => Promise<void>;
  buildEmbeddings: () => Promise<void>;
}

const VaultContext = createContext<VaultContextValue | null>(null);

export function VaultProvider({ children }: { children: ReactNode }) {
  const [vault, setVault] = useState<api.VaultInfo | null>(null);
  const [notes, setNotes] = useState<api.NoteMeta[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [embedding, setEmbedding] = useState(false);
  const [tagFilter, setTagFilterState] = useState<string | null>(null);
  const [tagPaths, setTagPaths] = useState<Set<string>>(new Set());

  const setTagFilter = useCallback(async (tag: string | null) => {
    if (!tag) {
      setTagFilterState(null);
      setTagPaths(new Set());
      return;
    }
    try {
      const paths = await api.notesForTag(tag);
      setTagPaths(new Set(paths));
      setTagFilterState(tag);
    } catch (e) {
      toast.error(`Couldn't filter by tag: ${e}`);
    }
  }, []);

  const refresh = useCallback(async () => {
    try {
      setNotes(await api.listNotes());
    } catch {
      /* no vault open yet */
    }
  }, []);

  const openPath = useCallback(async (path: string) => {
    setLoading(true);
    try {
      const info = await api.openVault(path);
      setVault(info);
      setNotes(await api.listNotes());
      setSelected(null);
      setTagFilterState(null);
      setTagPaths(new Set());
      toast.success(`Opened vault — ${info.note_count} notes`);
    } catch (e) {
      toast.error(`Couldn't open vault: ${e}`);
    } finally {
      setLoading(false);
    }
  }, []);

  const closeVault = useCallback(async () => {
    try {
      await api.closeVault();
    } catch {
      /* ignore */
    }
    setVault(null);
    setNotes([]);
    setSelected(null);
    setTagFilterState(null);
    setTagPaths(new Set());
  }, []);

  const pickAndOpen = useCallback(async () => {
    const picked = await openFolderDialog({
      directory: true,
      multiple: false,
      title: "Choose a vault folder",
    });
    if (typeof picked === "string") await openPath(picked);
  }, [openPath]);

  const createNote = useCallback(async () => {
    try {
      const rel = await api.createNote("Untitled.md");
      await refresh();
      setSelected(rel);
    } catch (e) {
      toast.error(`Couldn't create note: ${e}`);
    }
  }, [refresh]);

  const renameNote = useCallback(
    async (from: string, to: string) => {
      if (from === to) return;
      try {
        await api.renameNote(from, to);
        await refresh();
        setSelected((s) => (s === from ? to : s));
      } catch (e) {
        toast.error(`Rename failed: ${e}`);
      }
    },
    [refresh],
  );

  const removeNote = useCallback(
    async (relPath: string) => {
      try {
        await api.deleteNote(relPath);
        await refresh();
        setSelected((s) => (s === relPath ? null : s));
        toast.success("Note deleted");
      } catch (e) {
        toast.error(`Delete failed: ${e}`);
      }
    },
    [refresh],
  );

  const openOrCreate = useCallback(
    async (target: string) => {
      const t = target.toLowerCase();
      const match = notes.find((n) => {
        const stem = (n.rel_path.split("/").pop() ?? n.rel_path)
          .replace(/\.md$/i, "")
          .toLowerCase();
        return (
          stem === t ||
          n.title.trim().toLowerCase() === t ||
          n.rel_path.toLowerCase() === t ||
          n.rel_path.replace(/\.md$/i, "").toLowerCase() === t
        );
      });
      if (match) {
        setSelected(match.rel_path);
        return;
      }
      try {
        const rel = await api.createNote(`${target}.md`);
        await refresh();
        setSelected(rel);
      } catch (e) {
        toast.error(`Couldn't open "${target}": ${e}`);
      }
    },
    [notes, refresh],
  );

  const buildEmbeddings = useCallback(async () => {
    setEmbedding(true);
    try {
      const n = await api.embedVault();
      toast.success(`Semantic index ready — ${n} chunks embedded`);
    } catch (e) {
      toast.error(`Embedding failed: ${e}`);
    } finally {
      setEmbedding(false);
    }
  }, []);

  // Auto-reopen the last vault on launch.
  useEffect(() => {
    if (!isTauri) return;
    (async () => {
      try {
        const last = await api.getLastVault();
        if (last) await openPath(last);
      } catch {
        /* ignore */
      }
    })();
  }, [openPath]);

  // External changes (AI tools, cloud sync) → refresh the note list.
  useEffect(() => {
    if (!isTauri) return;
    const unlisten = listen("vault-changed", () => {
      void refresh();
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, [refresh]);

  const value = useMemo<VaultContextValue>(
    () => ({
      vault,
      notes,
      selected,
      loading,
      embedding,
      tagFilter,
      tagPaths,
      setTagFilter,
      pickAndOpen,
      openPath,
      closeVault,
      refresh,
      select: setSelected,
      createNote,
      renameNote,
      removeNote,
      openOrCreate,
      buildEmbeddings,
    }),
    [
      vault,
      notes,
      selected,
      loading,
      embedding,
      tagFilter,
      tagPaths,
      setTagFilter,
      pickAndOpen,
      openPath,
      closeVault,
      refresh,
      createNote,
      renameNote,
      removeNote,
      openOrCreate,
      buildEmbeddings,
    ],
  );

  return <VaultContext.Provider value={value}>{children}</VaultContext.Provider>;
}

// eslint-disable-next-line react-refresh/only-export-components
export function useVault(): VaultContextValue {
  const ctx = useContext(VaultContext);
  if (!ctx) throw new Error("useVault must be used within <VaultProvider>");
  return ctx;
}

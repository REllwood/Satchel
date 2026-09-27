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
import { flushPendingSave } from "@/features/editor/pending-save";

interface VaultContextValue {
  vault: api.VaultInfo | null;
  /** True until the last-used vault has been reopened (or found missing). */
  booting: boolean;
  notes: api.NoteMeta[];
  /** All folders ("notebooks"), including empty ones. */
  folders: string[];
  selected: string | null;
  loading: boolean;
  /** Background semantic-indexing progress (null before the first report). */
  embedStatus: api.EmbedStatus | null;
  embedding: boolean;
  /** Active tag filter (null = show all). */
  tagFilter: string | null;
  /** rel_paths matching the active tag filter. */
  tagPaths: Set<string>;
  setTagFilter: (tag: string | null) => Promise<void>;
  /** Create (if needed) and open a vault at an absolute path. */
  createVaultAt: (path: string) => Promise<void>;
  /** Pick any folder with the system dialog and use it as the vault. */
  pickAndOpen: () => Promise<void>;
  openPath: (path: string) => Promise<void>;
  closeVault: () => Promise<void>;
  refresh: () => Promise<void>;
  select: (relPath: string | null) => void;
  /** New note at the vault root, or inside `folder`. */
  createNote: (folder?: string) => Promise<void>;
  renameNote: (from: string, to: string) => Promise<void>;
  /** Move a note into `folder` ("" = vault root). */
  moveNote: (relPath: string, folder: string) => Promise<void>;
  removeNote: (relPath: string) => Promise<void>;
  createFolder: (relPath: string) => Promise<boolean>;
  renameFolder: (from: string, to: string) => Promise<void>;
  removeFolder: (relPath: string) => Promise<void>;
  /** Resolve a `[[target]]` to an existing note (and select it), or create it. */
  openOrCreate: (target: string) => Promise<void>;
  buildEmbeddings: () => Promise<void>;
  addAgentDocs: () => Promise<void>;
}

const VaultContext = createContext<VaultContextValue | null>(null);

const fileName = (relPath: string) => relPath.split("/").pop() ?? relPath;
const join = (folder: string, name: string) => (folder ? `${folder}/${name}` : name);

export function VaultProvider({ children }: { children: ReactNode }) {
  const [vault, setVault] = useState<api.VaultInfo | null>(null);
  const [booting, setBooting] = useState(isTauri);
  const [notes, setNotes] = useState<api.NoteMeta[]>([]);
  const [folders, setFolders] = useState<string[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [embedStatus, setEmbedStatus] = useState<api.EmbedStatus | null>(null);
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
      const [n, f] = await Promise.all([api.listNotes(), api.listFolders()]);
      setNotes(n);
      setFolders(f);
    } catch {
      /* no vault open yet */
    }
  }, []);

  const adopt = useCallback(
    async (info: api.VaultInfo) => {
      setVault(info);
      setSelected(null);
      setTagFilterState(null);
      setTagPaths(new Set());
      await refresh();
    },
    [refresh],
  );

  const openPath = useCallback(
    async (path: string) => {
      setLoading(true);
      try {
        await adopt(await api.openVault(path));
      } catch (e) {
        toast.error(`Couldn't open your notes: ${e}`);
      } finally {
        setLoading(false);
      }
    },
    [adopt],
  );

  const createVaultAt = useCallback(
    async (path: string) => {
      setLoading(true);
      try {
        const info = await api.createVault(path);
        await adopt(info);
        // Open the welcome note in a brand-new vault.
        if (info.note_count === 1) setSelected("Welcome to Satchel.md");
      } catch (e) {
        toast.error(`${e}`);
      } finally {
        setLoading(false);
      }
    },
    [adopt],
  );

  const pickAndOpen = useCallback(async () => {
    const picked = await openFolderDialog({
      directory: true,
      multiple: false,
      title: "Choose a folder for your notes",
    });
    if (typeof picked === "string") await createVaultAt(picked);
  }, [createVaultAt]);

  const closeVault = useCallback(async () => {
    try {
      await flushPendingSave();
      await api.closeVault();
    } catch {
      /* ignore */
    }
    setVault(null);
    setNotes([]);
    setFolders([]);
    setSelected(null);
    setTagFilterState(null);
    setTagPaths(new Set());
  }, []);

  const createNote = useCallback(
    async (folder = "") => {
      try {
        const rel = await api.createNote(join(folder, "Untitled.md"));
        await refresh();
        setSelected(rel);
      } catch (e) {
        toast.error(`Couldn't create note: ${e}`);
      }
    },
    [refresh],
  );

  const renameNote = useCallback(
    async (from: string, to: string) => {
      if (from === to) return;
      try {
        await flushPendingSave();
        await api.renameNote(from, to);
        await refresh();
        setSelected((s) => (s === from ? to : s));
      } catch (e) {
        toast.error(`Rename failed: ${e}`);
      }
    },
    [refresh],
  );

  const moveNote = useCallback(
    (relPath: string, folder: string) => renameNote(relPath, join(folder, fileName(relPath))),
    [renameNote],
  );

  const removeNote = useCallback(
    async (relPath: string) => {
      try {
        await flushPendingSave();
        await api.deleteNote(relPath);
        await refresh();
        setSelected((s) => (s === relPath ? null : s));
        toast.success("Moved to Trash");
      } catch (e) {
        toast.error(`Delete failed: ${e}`);
      }
    },
    [refresh],
  );

  const createFolder = useCallback(
    async (relPath: string) => {
      try {
        await api.createFolder(relPath);
        await refresh();
        return true;
      } catch (e) {
        toast.error(`Couldn't create folder: ${e}`);
        return false;
      }
    },
    [refresh],
  );

  const renameFolder = useCallback(
    async (from: string, to: string) => {
      if (from === to) return;
      try {
        await flushPendingSave();
        await api.renameFolder(from, to);
        await refresh();
        setSelected((s) => (s && s.startsWith(`${from}/`) ? `${to}${s.slice(from.length)}` : s));
      } catch (e) {
        toast.error(`Rename failed: ${e}`);
      }
    },
    [refresh],
  );

  const removeFolder = useCallback(
    async (relPath: string) => {
      try {
        await flushPendingSave();
        await api.deleteFolder(relPath);
        await refresh();
        setSelected((s) => (s && s.startsWith(`${relPath}/`) ? null : s));
        toast.success("Folder moved to Trash");
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
        const stem = fileName(n.rel_path).replace(/\.md$/i, "").toLowerCase();
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
    try {
      await api.embedVault();
      toast("Rebuilding the semantic index in the background…");
    } catch (e) {
      toast.error(`${e}`);
    }
  }, []);

  const addAgentDocs = useCallback(async () => {
    try {
      await api.addAgentDocs();
      await refresh();
      toast.success("Added AGENTS.md and CLAUDE.md to your notes folder");
    } catch (e) {
      toast.error(`${e}`);
    }
  }, [refresh]);

  // Reopen the last vault on launch; show setup only once we know there's none.
  useEffect(() => {
    if (!isTauri) return;
    (async () => {
      try {
        const last = await api.getLastVault();
        if (last) await openPath(last);
      } catch {
        /* ignore */
      } finally {
        setBooting(false);
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

  // Background semantic indexing progress.
  useEffect(() => {
    if (!isTauri) return;
    const unlisten = listen<api.EmbedStatus>("embed-status", (e) => {
      setEmbedStatus(e.payload);
      if (e.payload.error) toast.error(`Semantic search unavailable: ${e.payload.error}`);
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);

  const embedding = embedStatus?.running ?? false;

  const value = useMemo<VaultContextValue>(
    () => ({
      vault,
      booting,
      notes,
      folders,
      selected,
      loading,
      embedStatus,
      embedding,
      tagFilter,
      tagPaths,
      setTagFilter,
      createVaultAt,
      pickAndOpen,
      openPath,
      closeVault,
      refresh,
      select: setSelected,
      createNote,
      renameNote,
      moveNote,
      removeNote,
      createFolder,
      renameFolder,
      removeFolder,
      openOrCreate,
      buildEmbeddings,
      addAgentDocs,
    }),
    [
      vault,
      booting,
      notes,
      folders,
      selected,
      loading,
      embedStatus,
      embedding,
      tagFilter,
      tagPaths,
      setTagFilter,
      createVaultAt,
      pickAndOpen,
      openPath,
      closeVault,
      refresh,
      createNote,
      renameNote,
      moveNote,
      removeNote,
      createFolder,
      renameFolder,
      removeFolder,
      openOrCreate,
      buildEmbeddings,
      addAgentDocs,
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

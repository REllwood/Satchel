import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

import * as api from "@/lib/api";
import { isTauri } from "@/lib/ipc";
import { AppShell } from "@/components/layout/AppShell";
import { VaultProvider } from "@/features/vault/vault-store";
import { flushPendingSave } from "@/features/editor/pending-save";
import { Toaster } from "@/components/ui/sonner";

/** Save pending edits before the window closes or the app quits, then let it go. */
function useSaveBeforeExit() {
  useEffect(() => {
    if (!isTauri) return;
    const unlisten = listen(api.APP_EXIT_REQUESTED, async () => {
      try {
        await flushPendingSave();
      } finally {
        await api.confirmExit().catch(() => {});
      }
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);
}

function App() {
  useSaveBeforeExit();
  return (
    <VaultProvider>
      <AppShell />
      <Toaster position="bottom-right" />
    </VaultProvider>
  );
}

export default App;

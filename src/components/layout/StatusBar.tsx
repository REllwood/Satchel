import { useEffect, useState } from "react";
import { FolderOpen } from "lucide-react";

import { coreVersion } from "@/lib/ipc";
import { useVault } from "@/features/vault/vault-store";

/** Slim bottom status bar: vault status + core version. */
export function StatusBar() {
  const [version, setVersion] = useState("…");
  const { vault, embedding } = useVault();

  useEffect(() => {
    coreVersion()
      .then(setVersion)
      .catch(() => setVersion("?"));
  }, []);

  return (
    <footer className="flex h-6 shrink-0 items-center gap-3 border-t bg-sidebar px-3 text-[11px] text-muted-foreground">
      <span className="flex min-w-0 items-center gap-1">
        <FolderOpen className="size-3 shrink-0" aria-hidden />
        <span className="truncate">
          {vault ? `${vault.note_count} notes` : "No vault open"}
        </span>
      </span>
      {embedding && <span className="text-primary">building semantic index…</span>}
      <span className="ml-auto tabular-nums">core v{version}</span>
    </footer>
  );
}

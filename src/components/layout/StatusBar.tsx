import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";

import { coreVersion } from "@/lib/ipc";
import { useVault } from "@/features/vault/vault-store";
import { ProviderIcon } from "@/features/vault/ProviderIcon";

/** Slim bottom status bar: where notes sync, note count, indexing progress. */
export function StatusBar() {
  const [version, setVersion] = useState("…");
  const { vault, notes, embedStatus } = useVault();

  useEffect(() => {
    coreVersion()
      .then(setVersion)
      .catch(() => setVersion("?"));
  }, []);

  const syncLabel =
    vault && vault.provider !== "local"
      ? `Syncing via ${vault.provider_name}`
      : "Stored on this computer";

  return (
    <footer className="flex h-6 shrink-0 items-center gap-3 border-t bg-sidebar px-3 text-[11px] text-muted-foreground">
      {vault && (
        <span className="flex min-w-0 items-center gap-1" title={vault.root}>
          <ProviderIcon provider={vault.provider} className="size-3 shrink-0" aria-hidden />
          <span className="truncate">{syncLabel}</span>
        </span>
      )}
      {vault && <span className="tabular-nums">{notes.length} notes</span>}
      {embedStatus?.running && embedStatus.total > 0 && (
        <span className="flex items-center gap-1 text-primary" role="status">
          <Loader2 className="size-3 animate-spin" aria-hidden />
          <span className="tabular-nums">
            Indexing for semantic search {embedStatus.done}/{embedStatus.total}
          </span>
        </span>
      )}
      <span className="ml-auto tabular-nums">v{version}</span>
    </footer>
  );
}

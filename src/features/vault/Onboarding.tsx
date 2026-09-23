import { useCallback, useEffect, useState } from "react";
import { homeDir } from "@tauri-apps/api/path";
import { FolderOpen, Loader2, RefreshCw } from "lucide-react";

import * as api from "@/lib/api";
import { isTauri } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useVault } from "@/features/vault/vault-store";
import { ProviderIcon } from "@/features/vault/ProviderIcon";

function tildify(path: string, home: string | null): string {
  if (!home) return path;
  const h = home.replace(/[\\/]+$/, "");
  return path.startsWith(h) ? `~${path.slice(h.length)}` : path;
}

/**
 * First-run screen: choose where notes live. Cloud options are the sync
 * folders already on this computer — the provider's own app does the syncing.
 */
export function Onboarding() {
  const { createVaultAt, pickAndOpen, loading } = useVault();
  const [locations, setLocations] = useState<api.SyncLocation[] | null>(null);
  const [home, setHome] = useState<string | null>(null);
  const [busyPath, setBusyPath] = useState<string | null>(null);

  const detect = useCallback(async () => {
    if (!isTauri) {
      setLocations([]);
      return;
    }
    try {
      setLocations(await api.detectSyncLocations());
    } catch {
      setLocations([]);
    }
  }, []);

  useEffect(() => {
    void detect();
    if (isTauri) homeDir().then(setHome).catch(() => setHome(null));
  }, [detect]);

  const choose = async (loc: api.SyncLocation) => {
    setBusyPath(loc.vault_path);
    try {
      await createVaultAt(loc.vault_path);
    } finally {
      setBusyPath(null);
    }
  };

  const hasCloud = locations?.some((l) => l.provider !== "local") ?? false;
  const hasGoogle = locations?.some((l) => l.provider === "google_drive") ?? false;

  return (
    <div className="flex h-full items-start justify-center overflow-y-auto bg-background px-4 py-12 sm:items-center">
      <div className="w-full max-w-xl">
        <p className="text-sm font-semibold tracking-tight text-primary">MyNote</p>
        <h1 className="mt-2 text-2xl font-semibold tracking-tight">
          Where should your notes live?
        </h1>
        <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
          Your notes are plain Markdown files in a folder you choose. Pick a
          cloud folder and they sync to your other devices through that
          service — MyNote itself never uploads anything.
        </p>

        <div className="mt-6 flex flex-col gap-2" role="list" aria-label="Locations">
          {locations === null ? (
            <div className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden />
              Looking for iCloud Drive, Google Drive and other sync folders…
            </div>
          ) : (
            locations.map((loc) => {
              const busy = busyPath === loc.vault_path;
              return (
                <button
                  key={loc.vault_path}
                  type="button"
                  role="listitem"
                  disabled={loading}
                  onClick={() => void choose(loc)}
                  className={cn(
                    "group flex w-full items-center gap-3 rounded-lg border bg-card p-3 text-left transition-colors",
                    "hover:border-primary/50 hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    "disabled:pointer-events-none disabled:opacity-60",
                  )}
                >
                  <span
                    className={cn(
                      "flex size-9 shrink-0 items-center justify-center rounded-md",
                      loc.provider === "local"
                        ? "bg-muted text-muted-foreground"
                        : "bg-primary/10 text-primary",
                    )}
                  >
                    <ProviderIcon provider={loc.provider} className="size-4.5" aria-hidden />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center gap-2">
                      <span className="truncate text-sm font-medium">{loc.label}</span>
                      {loc.vault_exists && (
                        <Badge variant="secondary" className="shrink-0">
                          Existing folder
                        </Badge>
                      )}
                    </span>
                    <span
                      className="mt-0.5 block truncate text-xs text-muted-foreground"
                      title={loc.vault_path}
                    >
                      {tildify(loc.vault_path, home)}
                    </span>
                  </span>
                  <span className="shrink-0 text-xs font-medium text-muted-foreground transition-colors group-hover:text-primary group-focus-visible:text-primary">
                    {busy ? (
                      <Loader2 className="size-4 animate-spin" aria-label="Opening" />
                    ) : loc.vault_exists ? (
                      "Open"
                    ) : (
                      "Use this"
                    )}
                  </span>
                </button>
              );
            })
          )}
        </div>

        <div className="mt-4 flex flex-wrap items-center gap-2">
          <Button variant="outline" disabled={loading} onClick={() => void pickAndOpen()}>
            <FolderOpen />
            Choose another folder…
          </Button>
          <Button variant="ghost" disabled={loading} onClick={() => void detect()}>
            <RefreshCw />
            Refresh
          </Button>
        </div>

        {locations !== null && (!hasCloud || !hasGoogle) && (
          <p className="mt-6 text-xs leading-relaxed text-muted-foreground">
            {hasCloud ? "Want Google Drive instead? " : "No cloud folder found. "}
            Install <span className="font-medium text-foreground">Google Drive for desktop</span>
            {hasCloud ? "" : " or turn on iCloud Drive"}, sign in, then click Refresh.
            Dropbox and OneDrive folders work too.
          </p>
        )}
      </div>
    </div>
  );
}

import { useEffect, useState } from "react";
import { toast } from "sonner";

import * as api from "@/lib/api";
import { coreVersion } from "@/lib/ipc";
import { useVault } from "@/features/vault/vault-store";
import { ProviderIcon } from "@/features/vault/ProviderIcon";
import { useTheme, type Theme } from "@/components/theme-provider";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4 py-2.5">
      <div className="min-w-0">
        <div className="text-sm font-medium">{label}</div>
        {hint && <div className="mt-0.5 text-xs text-muted-foreground">{hint}</div>}
      </div>
      <div className="flex shrink-0 items-center gap-2">{children}</div>
    </div>
  );
}

const THEMES: Theme[] = ["system", "light", "dark"];

async function copy(text: string, what: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(`Copied ${what}`);
  } catch {
    toast.error("Couldn't copy to the clipboard");
  }
}

export function SettingsDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { vault, closeVault, buildEmbeddings, embedStatus, addAgentDocs, refresh } =
    useVault();
  const { theme, setTheme } = useTheme();
  const [version, setVersion] = useState("…");
  const [reindexing, setReindexing] = useState(false);
  const [setup, setSetup] = useState<api.AgentSetup | null>(null);

  useEffect(() => {
    coreVersion().then(setVersion).catch(() => setVersion("?"));
  }, []);

  useEffect(() => {
    if (!open || !vault) return;
    api.agentSetup().then(setSetup).catch(() => setSetup(null));
  }, [open, vault]);

  const changeLocation = async () => {
    onOpenChange(false);
    await closeVault();
  };

  const rebuildIndex = async () => {
    setReindexing(true);
    try {
      const stats = await api.reindex();
      await refresh();
      toast.success(`Reindexed ${stats.total} notes`);
    } catch (e) {
      toast.error(`Reindex failed: ${e}`);
    } finally {
      setReindexing(false);
    }
  };

  const embedding = embedStatus?.running ?? false;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Settings</DialogTitle>
          <DialogDescription>
            Everything runs on this computer. MyNote never sends your notes anywhere.
          </DialogDescription>
        </DialogHeader>

        <div className="divide-y">
          <Row
            label="Notes location"
            hint={
              vault ? (
                <span className="flex min-w-0 items-center gap-1" title={vault.root}>
                  <ProviderIcon provider={vault.provider} className="size-3 shrink-0" />
                  <span className="truncate">
                    {vault.provider === "local"
                      ? "This computer only"
                      : `Synced by ${vault.provider_name}`}
                  </span>
                </span>
              ) : (
                "none"
              )
            }
          >
            <Button size="sm" variant="outline" onClick={() => void changeLocation()}>
              Change…
            </Button>
          </Row>

          <Row label="Appearance">
            {THEMES.map((t) => (
              <Button
                key={t}
                size="sm"
                variant={theme === t ? "default" : "ghost"}
                className="capitalize"
                onClick={() => setTheme(t)}
              >
                {t}
              </Button>
            ))}
          </Row>

          <Row
            label="Semantic search"
            hint={
              embedding
                ? `Indexing ${embedStatus?.done ?? 0}/${embedStatus?.total ?? 0} notes…`
                : "Updates automatically as you write."
            }
          >
            <Button
              size="sm"
              variant="outline"
              disabled={!vault || embedding}
              onClick={() => void buildEmbeddings()}
            >
              Rebuild
            </Button>
          </Row>

          <Row label="Search index" hint="Rescan the notes folder from scratch.">
            <Button
              size="sm"
              variant="outline"
              disabled={!vault || reindexing}
              onClick={() => void rebuildIndex()}
            >
              {reindexing ? "Reindexing…" : "Rebuild"}
            </Button>
          </Row>

          <Row
            label="Claude Code"
            hint="Lets Claude Code search and read these notes (MCP)."
          >
            <Button
              size="sm"
              variant="outline"
              disabled={!setup}
              onClick={() => setup && void copy(setup.claude_code, "Claude Code command")}
            >
              Copy command
            </Button>
          </Row>

          <Row label="Codex" hint={<>Paste into <code>~/.codex/config.toml</code>.</>}>
            <Button
              size="sm"
              variant="outline"
              disabled={!setup}
              onClick={() => setup && void copy(setup.codex, "Codex config")}
            >
              Copy config
            </Button>
          </Row>

          <Row
            label="Agent guide"
            hint="Add AGENTS.md and CLAUDE.md so AI tools understand this folder."
          >
            <Button
              size="sm"
              variant="outline"
              disabled={!vault}
              onClick={() => void addAgentDocs()}
            >
              Add
            </Button>
          </Row>

          <Row label="About" hint="Open source under the MIT license.">
            <span className="text-xs tabular-nums text-muted-foreground">
              MyNote v{version}
            </span>
          </Row>
        </div>
      </DialogContent>
    </Dialog>
  );
}

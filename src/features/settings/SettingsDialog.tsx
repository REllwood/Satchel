import { useEffect, useState } from "react";
import { toast } from "sonner";

import * as api from "@/lib/api";
import { coreVersion } from "@/lib/ipc";
import { useVault } from "@/features/vault/vault-store";
import { useTheme, type Theme } from "@/components/theme-provider";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 py-2">
      <span className="text-sm font-medium">{label}</span>
      <div className="flex items-center gap-2">{children}</div>
    </div>
  );
}

const THEMES: Theme[] = ["system", "light", "dark"];

export function SettingsDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { vault, pickAndOpen, buildEmbeddings, embedding, refresh } = useVault();
  const { theme, setTheme } = useTheme();
  const [version, setVersion] = useState("…");
  const [reindexing, setReindexing] = useState(false);

  useEffect(() => {
    coreVersion().then(setVersion).catch(() => setVersion("?"));
  }, []);

  const mcpCommand = vault
    ? `claude mcp add --transport stdio mynote -- mynote mcp --vault "${vault.root}"`
    : "";

  const copyMcp = async () => {
    try {
      await navigator.clipboard.writeText(mcpCommand);
      toast.success("Copied MCP setup command");
    } catch {
      toast.error("Couldn't copy to clipboard");
    }
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

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Settings</DialogTitle>
          <DialogDescription>
            Everything stays on your machine. Nothing is sent out.
          </DialogDescription>
        </DialogHeader>

        <div className="divide-y">
          <Row label="Vault">
            <span className="max-w-[18rem] truncate text-xs text-muted-foreground">
              {vault?.root ?? "none"}
            </span>
            <Button size="sm" variant="outline" onClick={pickAndOpen}>
              Switch…
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

          <Row label="Semantic search">
            <Button
              size="sm"
              variant="outline"
              disabled={!vault || embedding}
              onClick={buildEmbeddings}
            >
              {embedding ? "Building…" : "Build / rebuild index"}
            </Button>
          </Row>

          <Row label="Search index">
            <Button
              size="sm"
              variant="outline"
              disabled={!vault || reindexing}
              onClick={rebuildIndex}
            >
              {reindexing ? "Reindexing…" : "Rebuild index"}
            </Button>
          </Row>

          <Row label="AI tools (MCP)">
            <Button size="sm" variant="outline" disabled={!vault} onClick={copyMcp}>
              Copy setup command
            </Button>
          </Row>

          <Row label="About">
            <span className="text-xs text-muted-foreground">MyNote · core v{version}</span>
          </Row>
        </div>
      </DialogContent>
    </Dialog>
  );
}

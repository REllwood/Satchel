import { useState } from "react";
import { toast } from "sonner";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import * as api from "@/lib/api";

const STARTERS: { label: string; sql: string }[] = [
  {
    label: "Recently updated",
    sql: "SELECT rel_path, datetime(mtime / 1000, 'unixepoch', 'localtime') AS updated\nFROM notes ORDER BY mtime DESC LIMIT 20",
  },
  {
    label: "Notes per tag",
    sql: "SELECT tag, count(*) AS notes FROM tags GROUP BY tag ORDER BY notes DESC",
  },
  {
    label: "Orphan notes",
    sql: "SELECT rel_path FROM notes n\nWHERE NOT EXISTS (SELECT 1 FROM links l WHERE l.src_note_id = n.id OR l.target_note_id = n.id)\nORDER BY rel_path",
  },
];

const STORAGE_KEY = "satchel-saved-queries";

interface SavedQuery {
  name: string;
  sql: string;
}

function loadSaved(): SavedQuery[] {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY) || "[]");
  } catch {
    return [];
  }
}

export function QueryDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [sql, setSql] = useState(STARTERS[0].sql);
  const [result, setResult] = useState<api.QueryResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [saved, setSaved] = useState<SavedQuery[]>(loadSaved);
  const [name, setName] = useState("");

  const run = async () => {
    setRunning(true);
    setError(null);
    try {
      setResult(await api.runQuery(sql, 500));
    } catch (e) {
      setError(String(e));
      setResult(null);
    } finally {
      setRunning(false);
    }
  };

  const save = () => {
    const n = name.trim();
    if (!n) return;
    const next = [...saved.filter((s) => s.name !== n), { name: n, sql }];
    setSaved(next);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    setName("");
    toast.success(`Saved query "${n}"`);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[80vh] w-[80vw] max-w-[80vw] flex-col gap-0 p-0 sm:max-w-[80vw]">
        <DialogHeader className="border-b px-4 py-2">
          <DialogTitle>
            Query{" "}
            <span className="text-xs font-normal text-muted-foreground">
              read-only SQL over your notes index
            </span>
          </DialogTitle>
        </DialogHeader>
        <div className="flex min-h-0 flex-1 flex-col gap-2 p-3">
          <div className="flex flex-wrap gap-1">
            {STARTERS.map((s) => (
              <Button
                key={s.label}
                size="sm"
                variant="secondary"
                onClick={() => setSql(s.sql)}
              >
                {s.label}
              </Button>
            ))}
            {saved.map((s) => (
              <Button
                key={s.name}
                size="sm"
                variant="outline"
                onClick={() => setSql(s.sql)}
                title={s.sql}
              >
                {s.name}
              </Button>
            ))}
          </div>
          <textarea
            value={sql}
            spellCheck={false}
            onChange={(e) => setSql(e.target.value)}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") run();
            }}
            aria-label="SQL query"
            className="h-28 w-full resize-none rounded-md border bg-background p-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button onClick={run} disabled={running}>
              {running ? "Running…" : "Run (⌘↵)"}
            </Button>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="save as…"
              className="h-9 w-40"
            />
            <Button variant="outline" onClick={save} disabled={!name.trim()}>
              Save
            </Button>
            {result && (
              <span className="text-xs text-muted-foreground">
                {result.rows.length} rows{result.truncated ? " (capped at 500)" : ""}
              </span>
            )}
          </div>
          {error && (
            <p className="rounded-md bg-destructive/10 px-2 py-1 text-sm text-destructive">
              {error}
            </p>
          )}
          <div className="min-h-0 flex-1 overflow-auto rounded-md border">
            {result ? (
              <Table>
                <TableHeader>
                  <TableRow>
                    {result.columns.map((c) => (
                      <TableHead key={c}>{c}</TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {result.rows.map((row, i) => (
                    <TableRow key={i}>
                      {row.map((cell, j) => (
                        <TableCell key={j} className="font-mono text-xs">
                          {cell === null ? (
                            <span className="text-muted-foreground">null</span>
                          ) : (
                            String(cell)
                          )}
                        </TableCell>
                      ))}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <p className="p-3 text-sm text-muted-foreground">
                Run a query to see results.
              </p>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

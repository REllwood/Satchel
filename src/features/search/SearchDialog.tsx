import { useEffect, useRef, useState } from "react";

import {
  Command,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  Dialog,
  DialogContent,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import * as api from "@/lib/api";
import { useVault } from "@/features/vault/vault-store";

type Mode = "fulltext" | "semantic" | "hybrid";
const MODES: Mode[] = ["fulltext", "semantic", "hybrid"];

export function SearchDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { select } = useVault();
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const [mode, setMode] = useState<Mode>("fulltext");
  const [hits, setHits] = useState<api.SearchHit[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (open) {
      setQuery("");
      setHits([]);
    }
  }, [open]);

  useEffect(() => {
    if (!open || !query.trim()) {
      setHits([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    const timer = setTimeout(async () => {
      try {
        const fn =
          mode === "semantic"
            ? api.searchSemantic
            : mode === "hybrid"
              ? api.searchHybrid
              : api.searchFulltext;
        const results = await fn(query, 30);
        if (!cancelled) setHits(results);
      } catch {
        if (!cancelled) setHits([]);
      } finally {
        if (!cancelled) setLoading(false);
      }
    }, 200);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query, mode, open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="top-[14%] translate-y-0 overflow-hidden p-0"
        showCloseButton={false}
      >
        <DialogTitle className="sr-only">Search notes</DialogTitle>
        <Command shouldFilter={false}>
          <div className="flex gap-1 border-b px-2 py-1.5">
        {MODES.map((m) => (
          <Button
            key={m}
            size="sm"
            variant={mode === m ? "default" : "ghost"}
            className="h-6 px-2 text-xs capitalize"
            onClick={() => {
              setMode(m);
              inputRef.current?.focus(); // keep typing without another click
            }}
          >
            {m === "fulltext" ? "Full-text" : m}
          </Button>
        ))}
      </div>
      <CommandInput
        ref={inputRef}
        autoFocus
        value={query}
        onValueChange={setQuery}
        placeholder="Search notes…"
      />
      <CommandList>
        {/* Keep the previous results on screen while the next search runs. */}
        {loading && hits.length === 0 && (
          <div className="px-3 py-3 text-sm text-muted-foreground">searching…</div>
        )}
        {!loading && query.trim() !== "" && hits.length === 0 && (
          <CommandEmpty>No matches.</CommandEmpty>
        )}
        {hits.map((hit) => (
          <CommandItem
            key={hit.rel_path}
            value={hit.rel_path}
            onSelect={() => {
              select(hit.rel_path);
              onOpenChange(false);
            }}
            className="flex-col items-start gap-0.5"
          >
            <span className="truncate font-medium">{hit.title || hit.rel_path}</span>
            {hit.snippet && (
              <span className="line-clamp-1 text-xs text-muted-foreground">
                <Snippet text={hit.snippet} />
              </span>
            )}
          </CommandItem>
          ))}
          </CommandList>
        </Command>
      </DialogContent>
    </Dialog>
  );
}

/** Render a snippet, highlighting matches the backend wrapped in \u0002…\u0003. */
function Snippet({ text }: { text: string }) {
  const parts = text.split(/\u0002([^\u0003]*)\u0003/);
  return (
    <>
      {parts.map((part, i) =>
        i % 2 === 1 ? (
          <mark key={i} className="rounded-sm bg-primary/15 px-0.5 font-medium text-foreground">
            {part}
          </mark>
        ) : (
          part
        ),
      )}
    </>
  );
}

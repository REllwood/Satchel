import { useEffect, useState, type ReactNode } from "react";

import * as api from "@/lib/api";
import { useVault } from "@/features/vault/vault-store";

interface Heading {
  level: number;
  text: string;
}

/** Headings from markdown, ignoring fenced code blocks. */
function parseHeadings(md: string): Heading[] {
  const out: Heading[] = [];
  let inFence = false;
  for (const line of md.split("\n")) {
    if (/^\s*(```|~~~)/.test(line)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    const m = /^(#{1,6})\s+(.*\S)\s*$/.exec(line);
    if (m) out.push({ level: m[1].length, text: m[2].trim() });
  }
  return out;
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1">
      <h2 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {title}
      </h2>
      {children}
    </section>
  );
}

export function ContextPanel() {
  // `notes` changes whenever the vault does (own saves included), which keeps
  // the outline and backlinks current while you type.
  const { selected, select, notes } = useVault();
  const [backlinks, setBacklinks] = useState<api.Backlink[]>([]);
  const [headings, setHeadings] = useState<Heading[]>([]);

  useEffect(() => {
    if (!selected) {
      setBacklinks([]);
      setHeadings([]);
      return;
    }
    let cancelled = false;
    api
      .getBacklinks(selected)
      .then((b) => !cancelled && setBacklinks(b))
      .catch(() => {});
    api
      .readNote(selected)
      .then((n) => !cancelled && setHeadings(parseHeadings(n.content)))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [selected, notes]);

  if (!selected) {
    return (
      <p className="text-sm text-muted-foreground">No note selected.</p>
    );
  }

  const linked = backlinks.filter((b) => !b.unlinked);
  const mentions = backlinks.filter((b) => b.unlinked);

  return (
    <div className="flex flex-col gap-5">
      <Section title="Outline">
        {headings.length === 0 ? (
          <p className="text-sm text-muted-foreground">No headings.</p>
        ) : (
          headings.map((h, i) => (
            <div
              key={i}
              style={{ paddingLeft: `${(h.level - 1) * 10}px` }}
              className="truncate text-sm text-muted-foreground"
              title={h.text}
            >
              {h.text}
            </div>
          ))
        )}
      </Section>

      <Section title={`Backlinks (${linked.length})`}>
        {linked.length === 0 ? (
          <p className="text-sm text-muted-foreground">No backlinks yet.</p>
        ) : (
          linked.map((b) => (
            <button
              key={b.rel_path}
              type="button"
              onClick={() => select(b.rel_path)}
              className="block w-full truncate rounded-sm py-0.5 text-left text-sm text-primary hover:underline"
              title={b.rel_path}
            >
              {b.title || b.rel_path}
            </button>
          ))
        )}
      </Section>

      {mentions.length > 0 && (
        <Section title={`Unlinked mentions (${mentions.length})`}>
          {mentions.map((b) => (
            <button
              key={b.rel_path}
              type="button"
              onClick={() => select(b.rel_path)}
              className="block w-full truncate rounded-sm py-0.5 text-left text-sm text-muted-foreground hover:underline"
              title={b.rel_path}
            >
              {b.title || b.rel_path}
            </button>
          ))}
        </Section>
      )}
    </div>
  );
}

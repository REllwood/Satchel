import { useEffect, useState } from "react";
import { X } from "lucide-react";

import * as api from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { useVault } from "@/features/vault/vault-store";

/** Horizontal tag filter shown above the file tree. */
export function TagsBar() {
  const { tagFilter, setTagFilter, notes } = useVault();
  const [tags, setTags] = useState<api.TagCount[]>([]);

  useEffect(() => {
    let cancelled = false;
    api
      .getTags()
      .then((t) => !cancelled && setTags(t))
      .catch(() => !cancelled && setTags([]));
    return () => {
      cancelled = true;
    };
  }, [notes]);

  // The active filter stays visible (so it can be cleared) even when no note
  // has the tag any more, or it is past the first 40.
  const shown = tags.slice(0, 40);
  if (tagFilter && !shown.some((t) => t.tag === tagFilter)) {
    const active = tags.find((t) => t.tag === tagFilter) ?? { tag: tagFilter, count: 0 };
    shown.unshift(active);
  }
  if (shown.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-1 px-2 pb-2">
      {shown.map((t) => {
        const active = tagFilter === t.tag;
        return (
          <button
            key={t.tag}
            type="button"
            aria-pressed={active}
            title={active ? "Clear the filter" : undefined}
            onClick={() => void setTagFilter(active ? null : t.tag)}
          >
            <Badge
              variant={active ? "default" : "secondary"}
              className="cursor-default gap-1 text-[11px] font-normal"
            >
              #{t.tag}
              <span className="opacity-60">{t.count}</span>
              {active && <X className="size-3" />}
            </Badge>
          </button>
        );
      })}
    </div>
  );
}

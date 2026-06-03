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
    api
      .getTags()
      .then(setTags)
      .catch(() => setTags([]));
  }, [notes]);

  if (tags.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-1 px-2 pb-2">
      {tags.slice(0, 40).map((t) => {
        const active = tagFilter === t.tag;
        return (
          <button
            key={t.tag}
            type="button"
            aria-pressed={active}
            onClick={() => setTagFilter(active ? null : t.tag)}
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

import { type EditorState, type Range, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";

import * as api from "@/lib/api";
import { currentNote } from "./note-context";
import { findImageEmbeds, imageMime, isExternal } from "./image-embeds";
import { ObjectUrlCache } from "./object-url-cache";

// Render image embeds inline (below their source line): `![[path]]` and
// `![alt](path)`. The embed syntax is hidden unless the cursor is on its line.
// Images load via the read_attachment command, resolved from the open note,
// as blob URLs kept in a small cache.
//
// Block widgets must be provided via a StateField (CodeMirror forbids block
// decorations from view plugins), so we scan the whole doc on change — images
// are sparse, so this is cheap.

const cache = new ObjectUrlCache(24, (url) => URL.revokeObjectURL(url));
/** Releases the cached URL each widget's DOM holds. */
const handles = new WeakMap<HTMLElement, () => void>();

/** Forget every cached image (when the vault closes). */
export function clearImageCache(): void {
  cache.clear();
}

/** Relative paths resolve from the note's folder, so that is part of the key. */
function cacheKey(target: string, note: string | null): string {
  const folder = note && note.includes("/") ? note.slice(0, note.lastIndexOf("/")) : "";
  return `${folder}\n${target}`;
}

class ImageWidget extends WidgetType {
  constructor(
    readonly target: string,
    readonly note: string | null,
  ) {
    super();
  }
  eq(other: ImageWidget) {
    return other.target === this.target && other.note === this.note;
  }
  toDOM() {
    const wrap = document.createElement("div");
    wrap.className = "cm-image-embed";
    const img = document.createElement("img");
    img.alt = this.target;
    const { target, note } = this;
    const handle = cache.acquire(cacheKey(target, note), async () => {
      const bytes = await api.readAttachment(target, note);
      return URL.createObjectURL(new Blob([bytes], { type: imageMime(target) ?? "" }));
    });
    handles.set(wrap, handle.release);
    handle.url.then(
      (url) => {
        img.src = url;
      },
      () => {
        wrap.textContent = `⚠ image not found: ${target}`;
      },
    );
    wrap.appendChild(img);
    return wrap;
  }
  destroy(dom: HTMLElement) {
    handles.get(dom)?.();
    handles.delete(dom);
  }
  ignoreEvent() {
    return true;
  }
}

function build(state: EditorState): DecorationSet {
  const decos: Range<Decoration>[] = [];
  const note = state.facet(currentNote)();
  const activeLines = new Set(
    state.selection.ranges.flatMap((r) => {
      const a = state.doc.lineAt(r.from).number;
      const b = state.doc.lineAt(r.to).number;
      return Array.from({ length: b - a + 1 }, (_, i) => a + i);
    }),
  );
  for (let i = 1; i <= state.doc.lines; i++) {
    const line = state.doc.line(i);
    if (!line.text.includes("![")) continue;
    for (const embed of findImageEmbeds(line.text)) {
      if (!imageMime(embed.target) || isExternal(embed.target)) continue;
      if (!activeLines.has(i)) {
        decos.push(Decoration.replace({}).range(line.from + embed.from, line.from + embed.to));
      }
      decos.push(
        Decoration.widget({
          widget: new ImageWidget(embed.target, note),
          block: true,
          side: 1,
        }).range(line.to),
      );
    }
  }
  return Decoration.set(decos, true);
}

export const imagePreview = StateField.define<DecorationSet>({
  create: (state) => build(state),
  update(value, tr) {
    return tr.docChanged || tr.selection ? build(tr.state) : value;
  },
  provide: (f) => EditorView.decorations.from(f),
});

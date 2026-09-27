import { type EditorState, type Range, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";

import * as api from "@/lib/api";

// Render image embeds inline (below their source line): `![[path]]` and
// `![alt](path)`. The embed syntax is hidden unless the cursor is on its line.
// Images load via the read_attachment command as blob URLs.
//
// Block widgets must be provided via a StateField (CodeMirror forbids block
// decorations from view plugins), so we scan the whole doc on change — images
// are sparse, so this is cheap.

const IMG_RE = /!\[\[([^\]\n]+?)\]\]|!\[[^\]\n]*\]\(([^)\n]+)\)/g;

function isImage(rel: string): boolean {
  return /\.(png|jpe?g|gif|webp|svg|bmp|avif)$/i.test(rel);
}

class ImageWidget extends WidgetType {
  constructor(readonly rel: string) {
    super();
  }
  eq(other: ImageWidget) {
    return other.rel === this.rel;
  }
  toDOM() {
    const wrap = document.createElement("div");
    wrap.className = "cm-image-embed";
    const img = document.createElement("img");
    img.alt = this.rel;
    img.loading = "lazy";
    api
      .readAttachment(this.rel)
      .then((bytes) => {
        const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)]));
        img.src = url;
        // Track for revocation on destroy (avoids leaking object URLs).
        img.dataset.objurl = url;
      })
      .catch(() => {
        wrap.textContent = `⚠ image not found: ${this.rel}`;
      });
    wrap.appendChild(img);
    return wrap;
  }
  destroy(dom: HTMLElement) {
    const url = dom.querySelector("img")?.dataset.objurl;
    if (url) URL.revokeObjectURL(url);
  }
  ignoreEvent() {
    return true;
  }
}

function build(state: EditorState): DecorationSet {
  const decos: Range<Decoration>[] = [];
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
    IMG_RE.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = IMG_RE.exec(line.text))) {
      const rel = (m[1] || m[2] || "").split("|")[0].trim();
      if (!isImage(rel)) continue;
      if (!activeLines.has(i)) {
        const start = line.from + m.index;
        decos.push(Decoration.replace({}).range(start, start + m[0].length));
      }
      decos.push(
        Decoration.widget({ widget: new ImageWidget(rel), block: true, side: 1 }).range(line.to),
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

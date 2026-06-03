import { type EditorState, RangeSetBuilder, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import DOMPurify from "dompurify";

// Render ```mermaid fences as diagrams. Mermaid has a real XSS history, so:
//  - securityLevel 'strict', and
//  - the rendered SVG is sanitized with DOMPurify before it touches the DOM.
// Mermaid is dynamically imported so it stays out of the main bundle.

type MermaidApi = typeof import("mermaid").default;
let mermaidPromise: Promise<MermaidApi> | null = null;

async function getMermaid(): Promise<MermaidApi> {
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then((mod) => {
      mod.default.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: "neutral",
      });
      return mod.default;
    });
  }
  return mermaidPromise;
}

let counter = 0;
const FENCE_RE = /^```mermaid[^\n]*\n([\s\S]*?)\n```/gm;

class MermaidWidget extends WidgetType {
  constructor(readonly code: string) {
    super();
  }
  eq(other: MermaidWidget) {
    return other.code === this.code;
  }
  toDOM() {
    const wrap = document.createElement("div");
    wrap.className = "cm-mermaid";
    const id = `mmd-${counter++}`;
    getMermaid()
      .then((mermaid) => mermaid.render(id, this.code))
      .then(({ svg }) => {
        wrap.innerHTML = DOMPurify.sanitize(svg, {
          USE_PROFILES: { svg: true, svgFilters: true },
        });
      })
      .catch((err: unknown) => {
        wrap.className = "cm-mermaid-error";
        wrap.textContent = `Mermaid error: ${
          err instanceof Error ? err.message : String(err)
        }`;
      });
    return wrap;
  }
  ignoreEvent() {
    return true;
  }
}

function build(state: EditorState): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const text = state.doc.toString();
  if (!text.includes("```mermaid")) return builder.finish();
  FENCE_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = FENCE_RE.exec(text))) {
    const code = m[1].trim();
    if (!code) continue;
    const endLine = state.doc.lineAt(m.index + m[0].length);
    builder.add(
      endLine.to,
      endLine.to,
      Decoration.widget({
        widget: new MermaidWidget(code),
        block: true,
        side: 1,
      }),
    );
  }
  return builder.finish();
}

export const mermaidPreview = StateField.define<DecorationSet>({
  create: build,
  update(value, tr) {
    return tr.docChanged ? build(tr.state) : value;
  },
  provide: (f) => EditorView.decorations.from(f),
});

import { type EditorState, type Range, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import DOMPurify from "dompurify";

// Render ```mermaid fences as diagrams. The diagram replaces its source until
// the cursor enters the block (click the diagram to edit it). Mermaid has a
// real XSS history, so:
//  - securityLevel 'strict', and
//  - the rendered SVG is sanitized with DOMPurify before it touches the DOM.
// Labels are drawn as SVG text (htmlLabels off): DOMPurify's SVG profile strips
// the <foreignObject> HTML labels Mermaid uses by default, leaving empty boxes.
// Mermaid is dynamically imported so it stays out of the main bundle.

type MermaidApi = typeof import("mermaid").default;
let mermaidPromise: Promise<MermaidApi> | null = null;

async function getMermaid(): Promise<MermaidApi> {
  mermaidPromise ??= import("mermaid").then((mod) => mod.default);
  return mermaidPromise;
}

/** Diagram colours that follow the app theme (read when a diagram renders). */
function themeConfig() {
  const dark = document.documentElement.classList.contains("dark");
  return {
    startOnLoad: false,
    securityLevel: "strict" as const,
    htmlLabels: false,
    flowchart: { htmlLabels: false, curve: "basis" as const },
    theme: "base" as const,
    themeVariables: dark
      ? {
          background: "transparent",
          primaryColor: "#27264a",
          primaryBorderColor: "#8b86ff",
          primaryTextColor: "#ecebff",
          secondaryColor: "#1f1e3a",
          tertiaryColor: "#1f1e3a",
          lineColor: "#8f8cc9",
          textColor: "#ecebff",
          fontFamily: "inherit",
        }
      : {
          background: "transparent",
          primaryColor: "#eeedff",
          primaryBorderColor: "#6d67f5",
          primaryTextColor: "#1c1b3a",
          secondaryColor: "#f6f5ff",
          tertiaryColor: "#f6f5ff",
          lineColor: "#8a87c9",
          textColor: "#1c1b3a",
          fontFamily: "inherit",
        },
  };
}

let counter = 0;
// CommonMark allows fences to be indented by up to three spaces.
const FENCE_RE = /^ {0,3}```+[ \t]*mermaid[^\n]*\n([\s\S]*?)\n {0,3}```+[ \t]*$/gm;

class MermaidWidget extends WidgetType {
  constructor(readonly code: string) {
    super();
  }
  eq(other: MermaidWidget) {
    return other.code === this.code;
  }
  toDOM(view: EditorView) {
    const wrap = document.createElement("div");
    wrap.className = "cm-mermaid";
    wrap.title = "Click to edit";
    // Clicking the diagram moves the cursor into its source to edit it.
    wrap.addEventListener("mousedown", (e) => {
      e.preventDefault();
      const pos = view.posAtDOM(wrap);
      view.dispatch({ selection: { anchor: pos } });
      view.focus();
    });
    const id = `mmd-${counter++}`;
    getMermaid()
      .then((mermaid) => {
        mermaid.initialize(themeConfig());
        return mermaid.render(id, this.code);
      })
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
  const text = state.doc.toString();
  if (!text.includes("mermaid")) return Decoration.none;
  const sel = state.selection.main;
  const decos: Range<Decoration>[] = [];
  FENCE_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = FENCE_RE.exec(text))) {
    const code = m[1].trim();
    if (!code) continue;
    const from = m.index;
    const to = m.index + m[0].length;
    const widget = new MermaidWidget(code);
    if (sel.from <= to && sel.to >= from) {
      // Editing: show the source with the live diagram underneath.
      decos.push(Decoration.widget({ widget, block: true, side: 1 }).range(to));
    } else {
      decos.push(Decoration.replace({ widget, block: true }).range(from, to));
    }
  }
  return Decoration.set(decos, true);
}

export const mermaidPreview = StateField.define<DecorationSet>({
  create: build,
  update(value, tr) {
    return tr.docChanged || tr.selection ? build(tr.state) : value;
  },
  provide: (f) => EditorView.decorations.from(f),
});

import { syntaxTree } from "@codemirror/language";
import { type EditorState, type Range, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, WidgetType } from "@codemirror/view";
import { IterMode, type SyntaxNode } from "@lezer/common";
import DOMPurify from "dompurify";

// Render ```mermaid fences as diagrams. The diagram replaces its source until
// the cursor enters the block (click the diagram to edit it). Mermaid has a
// real XSS history, so:
//  - securityLevel 'strict', and
//  - the rendered SVG is sanitized with DOMPurify before it touches the DOM.
// Labels are drawn as SVG text (htmlLabels off): DOMPurify's SVG profile strips
// the <foreignObject> HTML labels Mermaid uses by default, leaving empty boxes.
// Mermaid is dynamically imported so it stays out of the main bundle.
// Diagrams come from the Markdown syntax tree, so only real, closed ```mermaid
// fences render, and they redraw when the app switches between light and dark.

type MermaidApi = typeof import("mermaid").default;
let mermaidPromise: Promise<MermaidApi> | null = null;

async function getMermaid(): Promise<MermaidApi> {
  mermaidPromise ??= import("mermaid").then((mod) => mod.default);
  return mermaidPromise;
}

/** Diagram colours that follow the app theme. */
function themeConfig(dark: boolean) {
  return {
    startOnLoad: false,
    securityLevel: "strict" as const,
    // Throw on a bad diagram instead of drawing an error graphic into the page.
    suppressErrorRendering: true,
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

/**
 * Remove anything Mermaid left in the page while rendering `id`: it draws in a
 * temporary element on the body, which a failed render can leave behind.
 */
function removeLeftovers(id: string, keep: HTMLElement) {
  for (const leftover of [id, `d${id}`, `i${id}`]) {
    const el = document.getElementById(leftover);
    if (el && !keep.contains(el)) el.remove();
  }
}

const isDark = () => document.documentElement.classList.contains("dark");
const setDark = StateEffect.define<boolean>();

/** Whether the app is dark. Part of the state, so a theme switch redraws diagrams. */
const darkMode = StateField.define<boolean>({
  create: isDark,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setDark)) value = e.value;
    return value;
  },
});

/** Tells the editor when the theme class on <html> changes. */
const themeWatcher = ViewPlugin.define((view) => {
  const observer = new MutationObserver(() => {
    const dark = isDark();
    if (dark !== view.state.field(darkMode)) view.dispatch({ effects: setDark.of(dark) });
  });
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
  return { destroy: () => observer.disconnect() };
});

class MermaidWidget extends WidgetType {
  constructor(
    readonly code: string,
    readonly dark: boolean,
  ) {
    super();
  }
  eq(other: MermaidWidget) {
    return other.code === this.code && other.dark === this.dark;
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
    const { code, dark } = this;
    getMermaid()
      .then(async (mermaid) => {
        mermaid.initialize(themeConfig(dark));
        try {
          return await mermaid.render(id, code);
        } finally {
          removeLeftovers(id, wrap);
        }
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

interface MermaidBlock {
  /** Whole lines, from the opening fence to the closing one. */
  from: number;
  to: number;
  code: string;
}

/**
 * Blocks that can hold a diagram fence. Block quotes are left out, as their
 * `>` marks sit inside the code.
 */
const CONTAINERS = new Set(["Document", "BulletList", "OrderedList", "ListItem"]);

/** A closed ```mermaid fence, or null. An unclosed one would swallow the rest of the note. */
function mermaidBlock(state: EditorState, node: SyntaxNode): MermaidBlock | null {
  const info = node.getChild("CodeInfo");
  const lang = info ? state.sliceDoc(info.from, info.to).trim().split(/\s+/)[0] : "";
  if (lang.toLowerCase() !== "mermaid") return null;
  if (node.getChildren("CodeMark").length < 2) return null;
  const text = node.getChildren("CodeText");
  if (text.length === 0) return null;
  const code = state.sliceDoc(text[0].from, text[text.length - 1].to).trim();
  if (!code) return null;
  return { from: state.doc.lineAt(node.from).from, to: state.doc.lineAt(node.to).to, code };
}

function build(state: EditorState): DecorationSet {
  const blocks: MermaidBlock[] = [];
  // Mounted trees are other languages inside code blocks: a mermaid fence shown
  // as an example inside a longer fence is code, not a diagram.
  syntaxTree(state).iterate({
    mode: IterMode.IgnoreMounts,
    enter: (node) => {
      if (node.name !== "FencedCode") return CONTAINERS.has(node.name);
      const block = mermaidBlock(state, node.node);
      if (block) blocks.push(block);
      return false;
    },
  });
  if (blocks.length === 0) return Decoration.none;
  const dark = state.field(darkMode);
  const sel = state.selection.main;
  const decos: Range<Decoration>[] = [];
  for (const { from, to, code } of blocks) {
    const widget = new MermaidWidget(code, dark);
    if (sel.from <= to && sel.to >= from) {
      // Editing: show the source with the live diagram underneath.
      decos.push(Decoration.widget({ widget, block: true, side: 1 }).range(to));
    } else {
      decos.push(Decoration.replace({ widget, block: true }).range(from, to));
    }
  }
  return Decoration.set(decos, true);
}

const diagrams = StateField.define<DecorationSet>({
  create: build,
  update(value, tr) {
    const rebuild =
      tr.docChanged ||
      tr.selection ||
      tr.effects.some((e) => e.is(setDark)) ||
      // The parser works in the background: newly parsed fences can render.
      syntaxTree(tr.state) !== syntaxTree(tr.startState);
    return rebuild ? build(tr.state) : value;
  },
  provide: (f) => EditorView.decorations.from(f),
});

export const mermaidPreview = [darkMode, diagrams, themeWatcher];

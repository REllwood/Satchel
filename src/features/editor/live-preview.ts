import { syntaxTree } from "@codemirror/language";
import type { Range } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";

// Obsidian-style "live preview": formatting marks are hidden on lines the
// cursor isn't on, and revealed (so you can edit them) on the active line.
// Fenced code gets a continuous block background, and `#tags` render as chips.
// The heading sizes / bold / italic styling itself comes from `mdHighlight`.

const HIDE = Decoration.replace({});
const TAG = Decoration.mark({ class: "cm-tag" });
const CODE_INFO = Decoration.mark({ class: "cm-code-info" });
const CODE_LINE = Decoration.line({ class: "cm-codeblock" });
const CODE_FIRST = Decoration.line({ class: "cm-codeblock cm-codeblock-first" });
const CODE_LAST = Decoration.line({ class: "cm-codeblock cm-codeblock-last" });

/** Lezer-markdown node names for the formatting marks we hide. */
const MARK_NODES = new Set([
  "HeaderMark",
  "EmphasisMark",
  "StrongEmphasisMark",
  "CodeMark",
  "QuoteMark",
  "StrikethroughMark",
]);

const CODE_NODES = new Set(["FencedCode", "CodeBlock", "InlineCode", "CodeText"]);
const TAG_RE = /(^|\s)(#[A-Za-z][\w/-]*)/g;

function activeLines(view: EditorView): Set<number> {
  const lines = new Set<number>();
  for (const range of view.state.selection.ranges) {
    const from = view.state.doc.lineAt(range.from).number;
    const to = view.state.doc.lineAt(range.to).number;
    for (let n = from; n <= to; n++) lines.add(n);
  }
  return lines;
}

function inCode(view: EditorView, pos: number): boolean {
  for (let n: { name: string; parent: unknown } | null = syntaxTree(view.state).resolveInner(pos, 1); n; ) {
    if (CODE_NODES.has(n.name)) return true;
    n = n.parent as typeof n;
  }
  return false;
}

function buildDecorations(view: EditorView): DecorationSet {
  const decos: Range<Decoration>[] = [];
  const active = activeLines(view);
  const { doc } = view.state;
  let lastTagLine = 0; // visible ranges can split a line; tag each line once

  for (const { from, to } of view.visibleRanges) {
    syntaxTree(view.state).iterate({
      from,
      to,
      enter: (node) => {
        if (node.name === "FencedCode") {
          const first = doc.lineAt(node.from).number;
          const last = doc.lineAt(node.to).number;
          for (let n = first; n <= last; n++) {
            const deco = n === first ? CODE_FIRST : n === last ? CODE_LAST : CODE_LINE;
            decos.push(deco.range(doc.line(n).from));
          }
          return;
        }
        const line = doc.lineAt(node.from).number;
        if (node.name === "CodeInfo" && !active.has(line)) {
          decos.push(CODE_INFO.range(node.from, node.to));
          return;
        }
        if (!MARK_NODES.has(node.name)) return;
        if (node.to <= node.from) return;
        if (active.has(line)) return; // reveal on the active line
        // Include a single trailing space after a heading's `#`s so the text
        // isn't pushed right when the marks are hidden.
        let end = node.to;
        if (node.name === "HeaderMark" && doc.sliceString(end, end + 1) === " ") {
          end += 1;
        }
        decos.push(HIDE.range(node.from, end));
      },
    });

    // `#tags` outside code (headings need a space after `#`, so never match).
    for (let pos = from; pos <= to; ) {
      const line = doc.lineAt(pos);
      if (line.number > lastTagLine && line.text.includes("#")) {
        lastTagLine = line.number;
        TAG_RE.lastIndex = 0;
        let m: RegExpExecArray | null;
        while ((m = TAG_RE.exec(line.text))) {
          const start = line.from + m.index + m[1].length;
          if (!inCode(view, start)) decos.push(TAG.range(start, start + m[2].length));
        }
      }
      pos = line.to + 1;
    }
  }
  return Decoration.set(decos, true);
}

/** ViewPlugin that maintains the live-preview decoration set. */
export const livePreview = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildDecorations(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged || update.selectionSet) {
        this.decorations = buildDecorations(update.view);
      }
    }
  },
  { decorations: (v) => v.decorations },
);

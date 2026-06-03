import { syntaxTree } from "@codemirror/language";
import { RangeSetBuilder } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";

// Obsidian-style "live preview": formatting marks are hidden on lines the
// cursor isn't on, and revealed (so you can edit them) on the active line.
// The heading sizes / bold / italic styling itself comes from `mdHighlight`.

const HIDE = Decoration.replace({});

/** Lezer-markdown node names for the formatting marks we hide. */
const MARK_NODES = new Set([
  "HeaderMark",
  "EmphasisMark",
  "StrongEmphasisMark",
  "CodeMark",
  "QuoteMark",
  "StrikethroughMark",
]);

function activeLines(view: EditorView): Set<number> {
  const lines = new Set<number>();
  for (const range of view.state.selection.ranges) {
    const from = view.state.doc.lineAt(range.from).number;
    const to = view.state.doc.lineAt(range.to).number;
    for (let n = from; n <= to; n++) lines.add(n);
  }
  return lines;
}

function buildDecorations(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const active = activeLines(view);

  for (const { from, to } of view.visibleRanges) {
    syntaxTree(view.state).iterate({
      from,
      to,
      enter: (node) => {
        if (!MARK_NODES.has(node.name)) return;
        if (node.to <= node.from) return;
        const line = view.state.doc.lineAt(node.from).number;
        if (active.has(line)) return; // reveal on the active line
        // Include a single trailing space after a heading's `#`s so the text
        // isn't pushed right when the marks are hidden.
        let end = node.to;
        if (
          node.name === "HeaderMark" &&
          view.state.doc.sliceString(end, end + 1) === " "
        ) {
          end += 1;
        }
        builder.add(node.from, end, HIDE);
      },
    });
  }
  return builder.finish();
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

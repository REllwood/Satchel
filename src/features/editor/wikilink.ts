import {
  autocompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import { Prec, RangeSetBuilder } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  keymap,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";

import type { NoteMeta } from "@/lib/api";
import { findHeading, linkAction, wikilinkAt } from "./link-target";

export interface WikilinkOptions {
  /** Current notes (read fresh each call so completion stays up to date). */
  getNotes: () => NoteMeta[];
  /** Open (or create) the note a `[[target]]` points at. */
  onOpen: (target: string) => void;
}

const WIKILINK_RE = /\[\[([^[\]\n]+?)\]\]/g;

/** Filename stem (no folder, no `.md`). */
function stem(relPath: string): string {
  return (relPath.split("/").pop() ?? relPath).replace(/\.md$/i, "");
}

function labelFor(note: NoteMeta): string {
  return note.title.trim() || stem(note.rel_path);
}

/** The link target portion of a `[[target#heading|alias]]` inner string. */
function targetOf(inner: string): string {
  return inner.split("|")[0].split("#")[0].trim();
}

function completionSource(opts: WikilinkOptions) {
  return (ctx: CompletionContext): CompletionResult | null => {
    const before = ctx.matchBefore(/\[\[([^\]\n]*)$/);
    if (!before) return null;
    if (before.from === before.to && !ctx.explicit) return null;
    const options: Completion[] = opts.getNotes().map((note) => ({
      label: labelFor(note),
      detail: note.rel_path,
      type: "text",
      apply: `${stem(note.rel_path)}]]`,
    }));
    return {
      from: before.from + 2, // right after the "[["
      options,
      validFor: /^[^\]\n]*$/,
    };
  };
}

function resolvedSet(notes: NoteMeta[]): Set<string> {
  const set = new Set<string>();
  for (const n of notes) {
    set.add(stem(n.rel_path).toLowerCase());
    if (n.title.trim()) set.add(n.title.trim().toLowerCase());
    set.add(n.rel_path.toLowerCase());
    set.add(n.rel_path.replace(/\.md$/i, "").toLowerCase());
  }
  return set;
}

/** Style `[[links]]`; off the cursor's line, hide the brackets (and an alias's
 *  target) so only the label shows, as in a rendered note. */
function decorations(view: EditorView, opts: WikilinkOptions): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const resolved = resolvedSet(opts.getNotes());
  const { state } = view;
  const active = new Set(
    state.selection.ranges.flatMap((r) => {
      const a = state.doc.lineAt(r.from).number;
      const b = state.doc.lineAt(r.to).number;
      return Array.from({ length: b - a + 1 }, (_, i) => a + i);
    }),
  );
  // Visible ranges can split a line (e.g. around a hidden image embed), so
  // remember the last line handled; the builder needs strictly sorted ranges.
  let lastLine = 0;
  for (const { from, to } of view.visibleRanges) {
    let pos = from;
    while (pos <= to) {
      const line = state.doc.lineAt(pos);
      pos = line.to + 1;
      if (line.number <= lastLine) continue;
      lastLine = line.number;
      WIKILINK_RE.lastIndex = 0;
      let m: RegExpExecArray | null;
      while ((m = WIKILINK_RE.exec(line.text))) {
        const start = line.from + m.index;
        const end = start + m[0].length;
        // `[[#Heading]]` links within this note, so it has no target to resolve.
        const target = targetOf(m[1]).toLowerCase();
        const found =
          !target || resolved.has(target) || resolved.has(target.replace(/\.md$/, ""));
        const mark = Decoration.mark({
          class: found ? "cm-wikilink" : "cm-wikilink cm-wikilink-unresolved",
        });
        if (active.has(line.number)) {
          builder.add(start, end, mark);
          continue;
        }
        const pipe = m[1].indexOf("|");
        const labelFrom = start + 2 + (pipe >= 0 ? pipe + 1 : 0);
        builder.add(start, labelFrom, Decoration.replace({}));
        builder.add(labelFrom, end - 2, mark);
        builder.add(end - 2, end, Decoration.replace({}));
      }
    }
  }
  return builder.finish();
}

/**
 * Follow the `[[link]]` at `pos`. Returns false when there is no link there.
 * `[[#Heading]]` jumps within this note, and a link to an attachment such as
 * `[[doc.pdf]]` does nothing, so neither ever creates a note.
 */
function followLinkAt(
  view: EditorView,
  pos: number,
  opts: WikilinkOptions,
  inside = false,
): boolean {
  const line = view.state.doc.lineAt(pos);
  const inner = wikilinkAt(line.text, pos - line.from, inside);
  if (inner == null) return false;
  const action = linkAction(inner);
  if (action?.kind === "note") {
    opts.onOpen(action.target);
  } else if (action?.kind === "heading") {
    const at = findHeading(view.state.doc.toString(), action.heading);
    if (at != null) {
      view.dispatch({
        selection: { anchor: at },
        effects: EditorView.scrollIntoView(at, { y: "start", yMargin: 24 }),
        userEvent: "select",
      });
      view.focus();
    }
  }
  return true;
}

/** Build the wikilink extension set: autocomplete, ⌘/Ctrl-click and Mod-Enter nav, styling. */
export function wikilinkExtension(opts: WikilinkOptions) {
  const styler = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = decorations(view, opts);
      }
      update(u: ViewUpdate) {
        if (u.docChanged || u.viewportChanged || u.selectionSet) {
          this.decorations = decorations(u.view, opts);
        }
      }
    },
    { decorations: (v) => v.decorations },
  );

  const clickToOpen = EditorView.domEventHandlers({
    mousedown(event, view) {
      if (!(event.metaKey || event.ctrlKey)) return false;
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      if (pos == null || !followLinkAt(view, pos, opts)) return false;
      event.preventDefault();
      return true;
    },
  });

  // Ahead of the default keymap, whose Mod-Enter inserts a blank line. That
  // still happens when the cursor is not in a link.
  const keyboardFollow = Prec.high(
    keymap.of([
      {
        key: "Mod-Enter",
        run: (view) => followLinkAt(view, view.state.selection.main.head, opts, true),
      },
    ]),
  );

  return [
    autocompletion({ override: [completionSource(opts)] }),
    styler,
    clickToOpen,
    keyboardFollow,
  ];
}

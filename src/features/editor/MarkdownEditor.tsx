import { useEffect, useRef } from "react";
import { Annotation, EditorState, type Extension } from "@codemirror/state";
import { EditorView, drawSelection, keymap } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { languages } from "@codemirror/language-data";
import {
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language";

import { editorTheme, mdHighlight } from "./theme";
import { livePreview } from "./live-preview";
import { imagePreview } from "./images";
import { attachmentHandler } from "./attachments";
import { mermaidPreview } from "./mermaid";
import { figmaPreview } from "./figma";

interface Props {
  /** Initial document; external changes are synced in. */
  doc: string;
  onChange: (value: string) => void;
  /** Extra extensions (live preview, wikilinks, attachments). Read once on mount. */
  extensions?: Extension[];
}

/** Marks a transaction that loads text from disk, which isn't a user edit. */
const fromDisk = Annotation.define<boolean>();

/** A thin React host around a CodeMirror 6 markdown editor. */
export function MarkdownEditor({ doc, onChange, extensions = [] }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  // Create the view once.
  useEffect(() => {
    if (!host.current) return;
    const state = EditorState.create({
      doc,
      extensions: [
        history(),
        drawSelection(),
        EditorView.lineWrapping,
        indentOnInput(),
        bracketMatching(),
        markdown({ base: markdownLanguage, codeLanguages: languages }),
        syntaxHighlighting(mdHighlight),
        livePreview,
        imagePreview,
        mermaidPreview,
        figmaPreview,
        attachmentHandler,
        editorTheme,
        EditorView.contentAttributes.of({
          "aria-label": "Note editor",
          role: "textbox",
          "aria-multiline": "true",
        }),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged && !u.transactions.some((t) => t.annotation(fromDisk))) {
            onChangeRef.current(u.state.doc.toString());
          }
        }),
        ...extensions,
      ],
    });
    const v = new EditorView({ state, parent: host.current });
    view.current = v;
    return () => {
      v.destroy();
      view.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Sync external doc changes (e.g. the file changed on disk) without disturbing
  // the cursor when the text already matches.
  useEffect(() => {
    const v = view.current;
    if (!v) return;
    const current = v.state.doc.toString();
    if (doc !== current) {
      // Preserve the caret (clamped) so an external reload doesn't jump to top.
      const sel = v.state.selection.main;
      v.dispatch({
        changes: { from: 0, to: current.length, insert: doc },
        selection: {
          anchor: Math.min(sel.anchor, doc.length),
          head: Math.min(sel.head, doc.length),
        },
        annotations: fromDisk.of(true),
      });
    }
  }, [doc]);

  return <div ref={host} className="h-full" />;
}

import { HighlightStyle } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

/** CodeMirror theme bound to the app's design tokens (light/dark via CSS vars). */
export const editorTheme = EditorView.theme({
  "&": {
    backgroundColor: "transparent",
    color: "var(--foreground)",
    height: "100%",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    fontFamily: "var(--font-sans)",
    lineHeight: "1.7",
    overflow: "auto",
  },
  ".cm-content": {
    padding: "0.5rem 1rem 40vh",
    caretColor: "var(--foreground)",
    maxWidth: "56rem",
  },
  ".cm-line": { padding: "0 2px" },
  "&.cm-focused .cm-cursor": { borderLeftColor: "var(--foreground)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "color-mix(in oklch, var(--primary) 25%, transparent)",
  },
  ".cm-activeLine": { backgroundColor: "transparent" },
  ".cm-gutters": { display: "none" },
  ".cm-wikilink": {
    color: "var(--primary)",
    cursor: "pointer",
    textDecoration: "underline",
    textUnderlineOffset: "2px",
  },
  ".cm-wikilink-unresolved": {
    color: "var(--muted-foreground)",
    textDecorationStyle: "dashed",
  },
  ".cm-image-embed": { padding: "0.35rem 0" },
  ".cm-image-embed img": {
    maxWidth: "min(100%, 40rem)",
    maxHeight: "60vh",
    borderRadius: "var(--radius)",
    border: "1px solid var(--border)",
  },
  ".cm-mermaid": {
    padding: "0.5rem 0",
    display: "flex",
    justifyContent: "center",
  },
  ".cm-mermaid svg": { maxWidth: "100%" },
  ".cm-mermaid-error": {
    padding: "0.5rem 0.75rem",
    color: "var(--destructive)",
    fontFamily: "var(--font-mono)",
    fontSize: "0.85em",
  },
  ".cm-figma-embed": { padding: "0.5rem 0" },
  ".cm-figma-embed iframe": {
    width: "100%",
    height: "450px",
    border: "1px solid var(--border)",
    borderRadius: "var(--radius)",
  },
});

/** Markdown syntax highlighting mapped to tokens. */
export const mdHighlight = HighlightStyle.define([
  { tag: t.heading1, fontSize: "1.6em", fontWeight: "700", lineHeight: "1.3" },
  { tag: t.heading2, fontSize: "1.4em", fontWeight: "700", lineHeight: "1.3" },
  { tag: t.heading3, fontSize: "1.2em", fontWeight: "700" },
  { tag: [t.heading4, t.heading5, t.heading6], fontWeight: "700" },
  { tag: t.strong, fontWeight: "700" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: [t.link, t.url], color: "var(--primary)", textDecoration: "underline" },
  {
    tag: t.monospace,
    fontFamily: "var(--font-mono)",
    fontSize: "0.9em",
    backgroundColor: "color-mix(in oklch, var(--muted) 70%, transparent)",
    borderRadius: "3px",
    padding: "0.05em 0.25em",
  },
  { tag: t.quote, color: "var(--muted-foreground)", fontStyle: "italic" },
  { tag: [t.processingInstruction, t.list], color: "var(--muted-foreground)" },
  { tag: t.contentSeparator, color: "var(--muted-foreground)" },
]);

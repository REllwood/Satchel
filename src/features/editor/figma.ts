import { type EditorState, RangeSetBuilder, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";

import { toEmbedUrl } from "./figma-url";

// Embed a Figma share link that's on its own line. Any live Figma embed loads
// from Figma's servers (the one intended remote-content exception; allowed in
// CSP via frame-src). Only public ("anyone with the link") files render.

class FigmaWidget extends WidgetType {
  constructor(readonly src: string) {
    super();
  }
  eq(other: FigmaWidget) {
    return other.src === this.src;
  }
  toDOM() {
    const wrap = document.createElement("div");
    wrap.className = "cm-figma-embed";
    const iframe = document.createElement("iframe");
    iframe.src = this.src;
    iframe.loading = "lazy";
    // allow-same-origin is required for Figma's embedded app to run; the frame
    // is restricted to figma.com by CSP frame-src and is isolated from Tauri IPC.
    iframe.setAttribute("sandbox", "allow-scripts allow-same-origin allow-popups");
    iframe.setAttribute("referrerpolicy", "no-referrer");
    iframe.setAttribute("allowfullscreen", "true");
    wrap.appendChild(iframe);
    return wrap;
  }
  ignoreEvent() {
    return true;
  }
}

function build(state: EditorState): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (let i = 1; i <= state.doc.lines; i++) {
    const line = state.doc.line(i);
    const trimmed = line.text.trim();
    if (!trimmed.includes("figma.com/")) continue;
    const src = toEmbedUrl(trimmed);
    if (src) {
      builder.add(
        line.to,
        line.to,
        Decoration.widget({
          widget: new FigmaWidget(src),
          block: true,
          side: 1,
        }),
      );
    }
  }
  return builder.finish();
}

export const figmaPreview = StateField.define<DecorationSet>({
  create: build,
  update(value, tr) {
    return tr.docChanged ? build(tr.state) : value;
  },
  provide: (f) => EditorView.decorations.from(f),
});

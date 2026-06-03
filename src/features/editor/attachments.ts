import { EditorView } from "@codemirror/view";
import { toast } from "sonner";

import * as api from "@/lib/api";

async function saveAndInsert(files: File[], view: EditorView) {
  for (const file of files) {
    if (!file.type.startsWith("image/")) continue;
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const ext = (file.type.split("/")[1] || "png").split("+")[0];
      const rel = await api.saveAttachment(Array.from(bytes), ext);
      const text = `![[${rel}]]`;
      const pos = view.state.selection.main.head;
      view.dispatch({
        changes: { from: pos, insert: text },
        selection: { anchor: pos + text.length },
      });
    } catch (e) {
      toast.error(`Couldn't save image: ${e}`);
    }
  }
}

/** Paste or drop an image → store it in the vault's attachments/ and embed it. */
export const attachmentHandler = EditorView.domEventHandlers({
  paste(event, view) {
    const items = event.clipboardData?.items;
    if (!items) return false;
    const files: File[] = [];
    for (const item of Array.from(items)) {
      if (item.kind === "file") {
        const f = item.getAsFile();
        if (f && f.type.startsWith("image/")) files.push(f);
      }
    }
    if (files.length === 0) return false;
    event.preventDefault();
    void saveAndInsert(files, view);
    return true;
  },
  drop(event, view) {
    const files = Array.from(event.dataTransfer?.files ?? []).filter((f) =>
      f.type.startsWith("image/"),
    );
    if (files.length === 0) return false;
    event.preventDefault();
    void saveAndInsert(files, view);
    return true;
  },
});

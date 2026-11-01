import { EditorView } from "@codemirror/view";
import { toast } from "sonner";

import * as api from "@/lib/api";
import { currentNote } from "./note-context";
import { attachmentExt } from "./image-embeds";
import { flushPendingSave } from "./pending-save";
import { detectEol } from "./eol";

interface Upload {
  file: File;
  ext: string;
}

/** The image files among `files` (pasted, or dropped from Finder). */
function imageUploads(files: Iterable<File>): Upload[] {
  const out: Upload[] = [];
  for (const file of files) {
    const ext = attachmentExt(file.name, file.type);
    if (ext) out.push({ file, ext });
  }
  return out;
}

/**
 * Add `embed` to the end of `note` on disk, for an upload that finished after
 * the user switched to another note.
 */
async function appendToNote(note: string, embed: string) {
  try {
    await flushPendingSave();
    const { content } = await api.readNote(note);
    const eol = detectEol(content);
    const gap = content === "" || content.endsWith("\n") ? "" : eol;
    await api.writeNote(note, `${content}${gap}${embed}${eol}`);
    toast.success(`Added the image to the end of "${note}".`);
  } catch (e) {
    toast.error(`Could not add ${embed} to "${note}": ${e}`);
  }
}

async function saveAndInsert(uploads: Upload[], view: EditorView, note: string | null) {
  for (const { file, ext } of uploads) {
    let rel: string;
    try {
      rel = await api.saveAttachment(new Uint8Array(await file.arrayBuffer()), ext);
    } catch (e) {
      toast.error(`Could not save the image: ${e}`);
      continue;
    }
    const text = `![[${rel}]]`;
    // A destroyed editor has left the page: the user opened another note.
    if (view.dom.isConnected) {
      const pos = view.state.selection.main.head;
      view.dispatch({
        changes: { from: pos, insert: text },
        selection: { anchor: pos + text.length },
      });
    } else if (note) {
      await appendToNote(note, text);
    }
  }
}

/** Paste or drop an image → store it in the vault's attachments/ and embed it. */
export const attachmentHandler = EditorView.domEventHandlers({
  paste(event, view) {
    const files: File[] = [];
    for (const item of Array.from(event.clipboardData?.items ?? [])) {
      const file = item.kind === "file" ? item.getAsFile() : null;
      if (file) files.push(file);
    }
    const uploads = imageUploads(files);
    if (uploads.length === 0) return false;
    event.preventDefault();
    void saveAndInsert(uploads, view, view.state.facet(currentNote)());
    return true;
  },
  dragover(event) {
    // Accept files dragged in from Finder (the default would refuse the drop).
    if (event.dataTransfer?.types.includes("Files")) {
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
    }
    return false;
  },
  drop(event, view) {
    const uploads = imageUploads(event.dataTransfer?.files ?? []);
    if (uploads.length === 0) return false;
    event.preventDefault();
    // Embed where the files were dropped.
    const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
    if (pos != null) view.dispatch({ selection: { anchor: pos } });
    void saveAndInsert(uploads, view, view.state.facet(currentNote)());
    return true;
  },
});

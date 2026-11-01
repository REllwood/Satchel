import { Facet } from "@codemirror/state";

/**
 * The vault path of the note an editor shows. It is a getter, read when it is
 * needed, because a note moved on disk keeps its editor at the new path.
 */
export const currentNote = Facet.define<() => string | null, () => string | null>({
  combine: (values) => values[0] ?? (() => null),
});

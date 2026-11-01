// Pure helpers for following `[[wikilinks]]` (Cmd/Ctrl-click and Mod-Enter).

/** What following a link should do. */
export type LinkAction =
  /** `[[#Heading]]`: jump to a heading in the open note. */
  | { kind: "heading"; heading: string }
  /** Open the note, creating it if it does not exist yet. */
  | { kind: "note"; target: string }
  /** An attachment such as `[[doc.pdf]]`. It is not a note, so never create one for it. */
  | { kind: "file"; target: string };

const WIKILINK_RE = /\[\[([^[\]\n]+?)\]\]/g;

/** Files that are not notes. A link to one must never create `x.png.md`. */
const FILE_EXT =
  /\.(png|jpe?g|gif|webp|svg|bmp|avif|ico|heic|tiff?|pdf|mp3|wav|ogg|m4a|flac|aac|mp4|m4v|mov|webm|mkv|avi|canvas|zip|csv|json|txt|html?|docx?|xlsx?|pptx?|key|pages|numbers)$/i;

/**
 * The inner text of the `[[link]]` at `offset` in `line`, if any. A click may
 * land on either edge of a link. A cursor counts only between its outer
 * brackets (`inside`), so Mod-Enter just after a link still adds a line.
 */
export function wikilinkAt(line: string, offset: number, inside = false): string | null {
  WIKILINK_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = WIKILINK_RE.exec(line))) {
    const end = m.index + m[0].length;
    if (inside ? offset > m.index && offset < end : offset >= m.index && offset <= end) {
      return m[1];
    }
  }
  return null;
}

/** True when `target` may name a note that a link can create. */
export function isNoteTarget(target: string): boolean {
  const path = target.trim();
  if (!path.replace(/\.md$/i, "").trim()) return false;
  if (FILE_EXT.test(path)) return false;
  // Hidden names and `..` would make a note the vault never shows.
  return !path.split("/").some((part) => part.trim().startsWith("."));
}

/** What following `[[inner]]` (the text between the brackets) should do. */
export function linkAction(inner: string): LinkAction | null {
  const ref = inner.split("|")[0];
  const hash = ref.indexOf("#");
  const target = (hash === -1 ? ref : ref.slice(0, hash)).trim();
  // `[[#A#B]]` points at heading B under heading A: B is the one to find.
  const heading =
    hash === -1
      ? ""
      : (ref
          .slice(hash + 1)
          .split("#")
          .map((part) => part.trim())
          .filter(Boolean)
          .pop() ?? "");
  if (!target) return heading ? { kind: "heading", heading } : null;
  if (FILE_EXT.test(target)) return { kind: "file", target };
  return isNoteTarget(target) ? { kind: "note", target } : null;
}

/** Heading text compared loosely: case, spacing and inline formatting do not matter. */
function normalise(text: string): string {
  return text
    .replace(/\[\[|\]\]|[*_`]/g, "")
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
}

/**
 * Where the heading called `heading` starts in `doc`, or null. Headings inside
 * fenced code do not count.
 */
export function findHeading(doc: string, heading: string): number | null {
  const want = normalise(heading);
  if (!want) return null;
  let pos = 0;
  let fence: string | null = null;
  let prev: { text: string; start: number } | null = null;
  for (const line of doc.split("\n")) {
    const start = pos;
    pos += line.length + 1;
    const marker = /^ {0,3}(`{3,}|~{3,})/.exec(line)?.[1];
    if (fence) {
      if (marker && marker[0] === fence[0] && marker.length >= fence.length && !line.trim().slice(marker.length).trim()) {
        fence = null;
      }
      prev = null;
      continue;
    }
    if (marker) {
      fence = marker;
      prev = null;
      continue;
    }
    const atx = /^ {0,3}#{1,6}(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*\r?$/.exec(line);
    if (atx) {
      if (normalise(atx[1] ?? "") === want) return start;
      prev = null;
      continue;
    }
    // A setext heading: its text is the line above an `===` or `---` underline.
    if (prev && /^ {0,3}(=+|-+)[ \t]*\r?$/.test(line) && normalise(prev.text) === want) {
      return prev.start;
    }
    prev = line.trim() ? { text: line, start } : null;
  }
  return null;
}

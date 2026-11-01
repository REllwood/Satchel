// Pure helpers for image embeds: finding them in a line of Markdown, and the
// file types and MIME types the editor can show.

export interface ImageEmbed {
  /** Offsets of the whole embed syntax in the line. */
  from: number;
  to: number;
  /** The file path as written, decoded (`img/my%20pic.png` becomes `img/my pic.png`). */
  target: string;
}

const MIME: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
  bmp: "image/bmp",
  avif: "image/avif",
  ico: "image/x-icon",
};

/** The MIME type of an image file the editor can show, by extension, or null. */
export function imageMime(path: string): string | null {
  const ext = /\.([a-z0-9]+)$/i.exec(path)?.[1].toLowerCase();
  return (ext && MIME[ext]) || null;
}

/**
 * The extension to store a pasted or dropped file under, or null when it is
 * not an image. Files from Finder may have no MIME type, so the name counts too.
 */
export function attachmentExt(name: string, type: string): string | null {
  const fromName = /\.([a-z0-9]+)$/i.exec(name)?.[1].toLowerCase();
  if (fromName && MIME[fromName]) return fromName === "jpeg" ? "jpg" : fromName;
  const fromType = Object.entries(MIME).find(([, mime]) => mime === type.toLowerCase())?.[0];
  if (!fromType) return null;
  return fromType === "jpeg" ? "jpg" : fromType;
}

/** True for a link to the web or another scheme, which is not a vault file. */
export function isExternal(target: string): boolean {
  return /^[a-z][a-z0-9+.-]*:/i.test(target) || target.startsWith("//");
}

/** Every `![[name]]` and `![alt](destination "title")` in `line`. */
export function findImageEmbeds(line: string): ImageEmbed[] {
  const out: ImageEmbed[] = [];
  let i = 0;
  while ((i = line.indexOf("![", i)) !== -1) {
    if (isEscaped(line, i)) {
      i += 2;
      continue;
    }
    const embed = line.startsWith("![[", i) ? wikiEmbed(line, i) : markdownImage(line, i);
    if (embed) {
      out.push(embed);
      i = embed.to;
    } else {
      i += 2;
    }
  }
  return out;
}

/** True when the character at `i` is escaped by an odd run of backslashes. */
function isEscaped(text: string, i: number): boolean {
  let n = 0;
  while (i - n - 1 >= 0 && text[i - n - 1] === "\\") n++;
  return n % 2 === 1;
}

/** `![[name.png]]`, `![[name.png|300]]` or `![[name.png#part]]`. */
function wikiEmbed(line: string, start: number): ImageEmbed | null {
  const close = line.indexOf("]]", start + 3);
  if (close === -1) return null;
  const inner = line.slice(start + 3, close);
  if (/[[\]]/.test(inner)) return null;
  const target = inner.split("|")[0].split("#")[0].trim();
  return target ? { from: start, to: close + 2, target } : null;
}

/** `![alt](dest)`, `![alt](<dest with spaces>)` and either with a title. */
function markdownImage(line: string, start: number): ImageEmbed | null {
  // Alt text: brackets may nest, and a backslash escapes the next character.
  let i = start + 2;
  for (let depth = 1; depth > 0; i++) {
    if (i >= line.length) return null;
    if (line[i] === "\\") i++;
    else if (line[i] === "[") depth++;
    else if (line[i] === "]") depth--;
  }
  if (line[i] !== "(") return null;
  i = skipSpaces(line, i + 1);

  let raw: string;
  if (line[i] === "<") {
    const end = closing(line, i + 1, ">");
    if (end === -1) return null;
    raw = line.slice(i + 1, end);
    if (raw.includes("<")) return null;
    i = end + 1;
  } else {
    // A bare destination ends at a space or at a `)` that closes the link.
    const begin = i;
    for (let depth = 0; i < line.length; i++) {
      const c = line[i];
      if (c === "\\") i++;
      else if (c === "(") depth++;
      else if (c === ")") {
        if (depth === 0) break;
        depth--;
      } else if (/\s/.test(c)) break;
    }
    raw = line.slice(begin, i);
  }

  const afterDest = i;
  i = skipSpaces(line, i);
  const quote = line[i];
  if (i > afterDest && (quote === '"' || quote === "'" || quote === "(")) {
    const end = closing(line, i + 1, quote === "(" ? ")" : quote);
    if (end === -1) return null;
    i = skipSpaces(line, end + 1);
  }
  if (line[i] !== ")") return null;
  const target = decodeDestination(raw);
  return target ? { from: start, to: i + 1, target } : null;
}

function skipSpaces(text: string, i: number): number {
  while (i < text.length && (text[i] === " " || text[i] === "\t")) i++;
  return i;
}

/** Index of the first unescaped `char` from `i`, or -1. */
function closing(text: string, i: number, char: string): number {
  for (; i < text.length; i++) {
    if (text[i] === "\\") i++;
    else if (text[i] === char) return i;
  }
  return -1;
}

/** Undo backslash escapes and percent-encoding in a link destination. */
function decodeDestination(raw: string): string {
  const unescaped = raw.replace(/\\([!-/:-@[-`{-~])/g, "$1");
  try {
    return decodeURIComponent(unescaped).trim();
  } catch {
    return unescaped.trim(); // a stray `%` that is not an escape
  }
}

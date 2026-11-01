// The editor always works in LF. A note saved with Windows line endings is
// written back with CRLF, so editing it does not rewrite every line.

export type Eol = "\n" | "\r\n";

/** The line ending a note mostly uses (LF when it has no line breaks). */
export function detectEol(text: string): Eol {
  const crlf = text.match(/\r\n/g)?.length ?? 0;
  const lf = (text.match(/\n/g)?.length ?? 0) - crlf;
  return crlf > lf ? "\r\n" : "\n";
}

/** `text` with every line break as LF, as the editor holds it. */
export function toLF(text: string): string {
  return text.replace(/\r\n?/g, "\n");
}

/** Editor text (LF) in the note's own line endings, ready to write to disk. */
export function withEol(text: string, eol: Eol): string {
  return eol === "\n" ? text : text.replace(/\n/g, "\r\n");
}

// Pure helper: turn a Figma share link into an Embed Kit 2.0 embed URL.
// The name segment is optional: `figma.com/design/KEY?node-id=1-2` works too.
const FIGMA_RE =
  /^https?:\/\/(?:www\.)?figma\.com\/(file|design|board|proto|slides)\/([A-Za-z0-9]+)(?:[/?#]\S*)?$/;

/** Decode a query value, leaving a malformed one as it is. */
function decode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

export function toEmbedUrl(raw: string): string | null {
  const m = FIGMA_RE.exec(raw.trim());
  if (!m) return null;
  const type = m[1] === "file" ? "design" : m[1];
  const key = m[2];
  const node = /[?&]node-id=([^&#\s]+)/.exec(raw);
  // Decode first, so an already encoded `1%3A2` is not encoded twice.
  const nodeParam = node ? `&node-id=${encodeURIComponent(decode(node[1]))}` : "";
  return `https://embed.figma.com/${type}/${key}?embed-host=satchel${nodeParam}`;
}

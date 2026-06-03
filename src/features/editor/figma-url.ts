// Pure helper: turn a Figma share link into an Embed Kit 2.0 embed URL.
const FIGMA_RE =
  /^https?:\/\/(?:www\.)?figma\.com\/(file|design|board|proto|slides)\/([A-Za-z0-9]+)(?:\/[^\s]*)?$/;

export function toEmbedUrl(raw: string): string | null {
  const m = FIGMA_RE.exec(raw.trim());
  if (!m) return null;
  const type = m[1] === "file" ? "design" : m[1];
  const key = m[2];
  const node = /[?&]node-id=([^&\s]+)/.exec(raw);
  const nodeParam = node ? `&node-id=${encodeURIComponent(node[1])}` : "";
  return `https://embed.figma.com/${type}/${key}?embed-host=mynote${nodeParam}`;
}

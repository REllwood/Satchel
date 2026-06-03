import { describe, expect, it } from "vitest";

import { toEmbedUrl } from "./figma-url";

describe("toEmbedUrl", () => {
  it("builds an embed URL from a design share link", () => {
    expect(toEmbedUrl("https://www.figma.com/design/ABC123/My-File")).toBe(
      "https://embed.figma.com/design/ABC123?embed-host=mynote",
    );
  });

  it("maps /file/ to design and preserves node-id", () => {
    expect(toEmbedUrl("https://www.figma.com/file/KEY9/x?node-id=1-2")).toBe(
      "https://embed.figma.com/design/KEY9?embed-host=mynote&node-id=1-2",
    );
  });

  it("returns null for non-Figma or malformed URLs", () => {
    expect(toEmbedUrl("https://example.com/foo")).toBeNull();
    expect(toEmbedUrl("not a url")).toBeNull();
  });
});

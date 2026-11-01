import { describe, expect, it } from "vitest";

import { toEmbedUrl } from "./figma-url";

describe("toEmbedUrl", () => {
  it("builds an embed URL from a design share link", () => {
    expect(toEmbedUrl("https://www.figma.com/design/ABC123/My-File")).toBe(
      "https://embed.figma.com/design/ABC123?embed-host=satchel",
    );
  });

  it("maps /file/ to design and preserves node-id", () => {
    expect(toEmbedUrl("https://www.figma.com/file/KEY9/x?node-id=1-2")).toBe(
      "https://embed.figma.com/design/KEY9?embed-host=satchel&node-id=1-2",
    );
  });

  it("returns null for non-Figma or malformed URLs", () => {
    expect(toEmbedUrl("https://example.com/foo")).toBeNull();
    expect(toEmbedUrl("not a url")).toBeNull();
  });

  it("embeds links with no name segment", () => {
    expect(toEmbedUrl("https://www.figma.com/design/KEY?node-id=1-2")).toBe(
      "https://embed.figma.com/design/KEY?embed-host=satchel&node-id=1-2",
    );
    expect(toEmbedUrl("https://figma.com/board/KEY")).toBe(
      "https://embed.figma.com/board/KEY?embed-host=satchel",
    );
  });

  it("does not encode an already encoded node-id twice", () => {
    expect(toEmbedUrl("https://www.figma.com/design/KEY/Name?node-id=1%3A2&t=abc")).toBe(
      "https://embed.figma.com/design/KEY?embed-host=satchel&node-id=1%3A2",
    );
    expect(toEmbedUrl("https://www.figma.com/design/KEY/Name?node-id=1:2")).toBe(
      "https://embed.figma.com/design/KEY?embed-host=satchel&node-id=1%3A2",
    );
  });

  it("rejects a key followed by anything other than a path, query or fragment", () => {
    expect(toEmbedUrl("https://www.figma.com/design/KEY-extra")).toBeNull();
    expect(toEmbedUrl("https://www.figma.com/design/KEY rest")).toBeNull();
  });
});


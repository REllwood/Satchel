import { describe, expect, it } from "vitest";

import { attachmentExt, findImageEmbeds, imageMime, isExternal } from "./image-embeds";

const targets = (line: string) => findImageEmbeds(line).map((e) => e.target);

describe("findImageEmbeds", () => {
  it("finds wiki embeds without their size or anchor", () => {
    expect(targets("![[attachments/a1.png]]")).toEqual(["attachments/a1.png"]);
    expect(targets("![[pic.png|300]] and ![[other.jpg#x]]")).toEqual(["pic.png", "other.jpg"]);
  });

  it("decodes percent-encoded destinations", () => {
    expect(targets("![a](img/my%20pic.png)")).toEqual(["img/my pic.png"]);
    expect(targets("![a](bad%zz.png)")).toEqual(["bad%zz.png"]);
  });

  it("reads angle-bracket destinations with spaces", () => {
    expect(targets("![a](<my pic.png>)")).toEqual(["my pic.png"]);
    expect(targets('![a](<my pic.png> "Title")')).toEqual(["my pic.png"]);
  });

  it("drops an optional title", () => {
    expect(targets('![a](pic.png "Title")')).toEqual(["pic.png"]);
    expect(targets("![a](pic.png 'Title')")).toEqual(["pic.png"]);
    expect(targets("![a](pic.png (Title))")).toEqual(["pic.png"]);
  });

  it("keeps balanced parentheses and escapes in the destination", () => {
    expect(targets("![a](shots/pic(1).png)")).toEqual(["shots/pic(1).png"]);
    expect(targets("![a](pic\\).png)")).toEqual(["pic).png"]);
  });

  it("allows brackets in the alt text", () => {
    expect(targets("![a [b] c](pic.png)")).toEqual(["pic.png"]);
  });

  it("reports where each embed sits in the line", () => {
    const line = "x ![a](p.png) y ![[q.png]]";
    expect(findImageEmbeds(line)).toEqual([
      { from: 2, to: 13, target: "p.png" },
      { from: 16, to: 26, target: "q.png" },
    ]);
  });

  it("ignores what is not an image embed", () => {
    expect(targets("[link](pic.png)")).toEqual([]);
    expect(targets("\\![a](pic.png)")).toEqual([]);
    expect(targets("![a](my pic.png)")).toEqual([]);
    expect(targets("![a](pic.png")).toEqual([]);
    expect(targets("![[unclosed.png")).toEqual([]);
    expect(targets("![]()")).toEqual([]);
  });
});

describe("image types", () => {
  it("maps extensions to MIME types, SVG included", () => {
    expect(imageMime("a/b.PNG")).toBe("image/png");
    expect(imageMime("x.jpeg")).toBe("image/jpeg");
    expect(imageMime("logo.svg")).toBe("image/svg+xml");
    expect(imageMime("doc.pdf")).toBeNull();
    expect(imageMime("noext")).toBeNull();
  });

  it("names attachments by file name, then by MIME type", () => {
    expect(attachmentExt("Screen Shot.PNG", "")).toBe("png");
    expect(attachmentExt("photo.jpeg", "image/jpeg")).toBe("jpg");
    expect(attachmentExt("image", "image/svg+xml")).toBe("svg");
    expect(attachmentExt("image.png", "image/png")).toBe("png");
    expect(attachmentExt("notes.txt", "text/plain")).toBeNull();
  });

  it("spots external links", () => {
    expect(isExternal("https://example.com/a.png")).toBe(true);
    expect(isExternal("data:image/png;base64,AAA")).toBe(true);
    expect(isExternal("//cdn.example.com/a.png")).toBe(true);
    expect(isExternal("attachments/a.png")).toBe(false);
  });
});

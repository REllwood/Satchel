import { describe, expect, it } from "vitest";

import { detectEol, toLF, withEol } from "./eol";

describe("line endings", () => {
  it("detects the line ending a note mostly uses", () => {
    expect(detectEol("a\r\nb\r\nc")).toBe("\r\n");
    expect(detectEol("a\nb\nc")).toBe("\n");
    expect(detectEol("a\r\nb\nc\nd")).toBe("\n");
    expect(detectEol("no line breaks")).toBe("\n");
    expect(detectEol("")).toBe("\n");
  });

  it("round-trips a CRLF note through the editor unchanged", () => {
    const disk = "# Title\r\n\r\n- one\r\n- two\r\n";
    const editor = toLF(disk);
    expect(editor).toBe("# Title\n\n- one\n- two\n");
    expect(withEol(editor, detectEol(disk))).toBe(disk);
  });

  it("writes new lines typed in a CRLF note as CRLF", () => {
    const editor = `${toLF("a\r\nb\r\n")}c\n`;
    expect(withEol(editor, "\r\n")).toBe("a\r\nb\r\nc\r\n");
  });

  it("leaves LF notes alone", () => {
    expect(withEol("a\nb\n", "\n")).toBe("a\nb\n");
  });
});

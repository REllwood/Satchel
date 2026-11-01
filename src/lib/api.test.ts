import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn(async (..._args: unknown[]) => undefined);
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const api = await import("./api");

describe("api", () => {
  beforeEach(() => invoke.mockClear());

  // Tauri looks up command arguments by camelCase name and rejects a call that
  // uses any other spelling, so a snake_case key breaks the command outright.
  it("sends every command argument under its camelCase name", async () => {
    const calls = Object.entries(api).filter(([, value]) => typeof value === "function");
    expect(calls.length).toBeGreaterThan(20);
    for (const [name, call] of calls) {
      invoke.mockClear();
      await (call as (...args: unknown[]) => Promise<unknown>)("a.md", "b", 1);
      expect(invoke, name).toHaveBeenCalledTimes(1);
      const args = (invoke.mock.calls[0][1] ?? {}) as Record<string, unknown>;
      for (const key of Object.keys(args)) {
        expect(key, `${name} sends "${key}"`).toMatch(/^[a-z][A-Za-z0-9]*$/);
      }
    }
  });
});

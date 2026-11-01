import { describe, expect, it } from "vitest";

import { ObjectUrlCache } from "./object-url-cache";

function setup(capacity = 2) {
  const revoked: string[] = [];
  const loads: string[] = [];
  const cache = new ObjectUrlCache(capacity, (url) => revoked.push(url));
  const load = (key: string) => async () => {
    loads.push(key);
    return `blob:${key}`;
  };
  return { cache, revoked, loads, load };
}

describe("ObjectUrlCache", () => {
  it("shares one load between widgets showing the same file", async () => {
    const { cache, loads, load } = setup();
    const a = cache.acquire("x", load("x"));
    const b = cache.acquire("x", load("x"));
    expect(await a.url).toBe("blob:x");
    expect(await b.url).toBe("blob:x");
    expect(loads).toEqual(["x"]);
  });

  it("keeps released URLs for reuse, revoking the least recently used past capacity", async () => {
    const { cache, revoked, loads, load } = setup(2);
    for (const key of ["a", "b", "c"]) {
      const h = cache.acquire(key, load(key));
      await h.url;
      h.release();
    }
    expect(revoked).toEqual(["blob:a"]);
    expect(cache.size).toBe(2);
    // "b" is still cached: no second read.
    await cache.acquire("b", load("b")).url;
    expect(loads).toEqual(["a", "b", "c"]);
  });

  it("never revokes a URL that is still shown", async () => {
    const { cache, revoked, load } = setup(1);
    const a = cache.acquire("a", load("a"));
    const b = cache.acquire("b", load("b"));
    await Promise.all([a.url, b.url]);
    expect(revoked).toEqual([]);
    a.release();
    expect(revoked).toEqual(["blob:a"]);
    b.release();
    b.release(); // a second release is ignored
    expect(revoked).toEqual(["blob:a"]);
  });

  it("revokes a URL that finishes loading after its widget and the cache let go", async () => {
    const { cache, revoked } = setup(4);
    let finish: (url: string) => void = () => {};
    const h = cache.acquire("slow", () => new Promise<string>((r) => (finish = r)));
    h.release();
    cache.clear();
    finish("blob:slow");
    await h.url;
    expect(revoked).toEqual(["blob:slow"]);
  });

  it("revokes URLs still shown at clear once they are released", async () => {
    const { cache, revoked, load } = setup(4);
    const shown = cache.acquire("a", load("a"));
    const unused = cache.acquire("b", load("b"));
    await Promise.all([shown.url, unused.url]);
    unused.release();
    cache.clear();
    expect(revoked).toEqual(["blob:b"]);
    shown.release();
    expect(revoked).toEqual(["blob:b", "blob:a"]);
  });

  it("forgets failed loads so the next widget retries", async () => {
    const { cache, loads } = setup();
    const failed = cache.acquire("x", async () => {
      loads.push("fail");
      throw new Error("not found");
    });
    await expect(failed.url).rejects.toThrow("not found");
    failed.release();
    expect(cache.size).toBe(0);
    const retry = cache.acquire("x", async () => {
      loads.push("x");
      return "blob:x";
    });
    expect(await retry.url).toBe("blob:x");
    expect(loads).toEqual(["fail", "x"]);
  });
});

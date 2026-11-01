interface Entry {
  url: Promise<string>;
  /** Set once loaded. */
  loaded: string | null;
  /** Widgets showing it. Only unused entries are evicted. */
  refs: number;
}

export interface UrlHandle {
  url: Promise<string>;
  /** Call once when the URL is no longer shown. */
  release: () => void;
}

/**
 * Object URLs for loaded files, shared by every widget showing the same file
 * and kept after the last one goes (up to `capacity` unused entries), so
 * scrolling back does not read the file again. URLs are revoked when evicted,
 * including ones that finish loading after every widget using them is gone.
 */
export class ObjectUrlCache {
  // Map order is least recently used first.
  private entries = new Map<string, Entry>();

  constructor(
    private readonly capacity: number,
    private readonly revoke: (url: string) => void,
  ) {}

  get size(): number {
    return this.entries.size;
  }

  /** The URL for `key`, loaded with `load` unless it is cached or loading. */
  acquire(key: string, load: () => Promise<string>): UrlHandle {
    let entry = this.entries.get(key);
    if (entry) {
      this.entries.delete(key); // re-added below as the most recent
    } else {
      const created: Entry = { url: Promise.resolve(""), loaded: null, refs: 0 };
      created.url = load().then(
        (url) => {
          created.loaded = url;
          // Cleared while loading, and nothing shows it: nothing else will revoke it.
          if (this.entries.get(key) !== created && created.refs === 0) this.revoke(url);
          else this.evict();
          return url;
        },
        (err: unknown) => {
          // Forget failures, so the next widget tries again.
          if (this.entries.get(key) === created) this.entries.delete(key);
          throw err;
        },
      );
      entry = created;
    }
    entry.refs++;
    this.entries.set(key, entry);
    this.evict();

    const held = entry;
    let released = false;
    return {
      url: held.url,
      release: () => {
        if (released) return;
        released = true;
        held.refs--;
        if (this.entries.get(key) === held) this.evict();
        else if (held.refs === 0 && held.loaded) this.revoke(held.loaded);
      },
    };
  }

  /** Drop every entry, revoking URLs nothing shows (the rest go on release). */
  clear(): void {
    for (const entry of this.entries.values()) {
      if (entry.refs === 0 && entry.loaded) this.revoke(entry.loaded);
    }
    this.entries.clear();
  }

  private evict(): void {
    for (const [key, entry] of this.entries) {
      if (this.entries.size <= this.capacity) return;
      if (entry.refs === 0 && entry.loaded) {
        this.entries.delete(key);
        this.revoke(entry.loaded);
      }
    }
  }
}

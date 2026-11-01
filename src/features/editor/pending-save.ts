// Lets file operations (rename, move, delete) and quitting save pending edits
// first, so a debounced autosave never lands on a path that just moved and
// the last edits are not lost when the app closes. The note editor and the
// canvas each register their own flush.
const flushers = new Set<() => Promise<void>>();

/** Register a flush. Returns a function that unregisters it. */
export function registerFlush(fn: () => Promise<void>): () => void {
  flushers.add(fn);
  return () => {
    flushers.delete(fn);
  };
}

export async function flushPendingSave(): Promise<void> {
  await Promise.allSettled([...flushers].map((fn) => fn()));
}

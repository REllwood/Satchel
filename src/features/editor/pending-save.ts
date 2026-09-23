// Lets file operations (rename, move, delete) save the editor's pending edit
// first, so a debounced autosave never lands on a path that just moved.
let flushFn: (() => Promise<void>) | null = null;

export function registerFlush(fn: (() => Promise<void>) | null) {
  flushFn = fn;
}

export async function flushPendingSave(): Promise<void> {
  await flushFn?.();
}

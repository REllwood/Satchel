import { toLF } from "./eol";

/** What the editor should do about the open note changing on disk. */
export type DiskChange = "ignore" | "reload" | "conflict";

/** One entry of a `vault-changed` event from the file watcher. */
export interface VaultChange {
  rel_path: string;
  removed: boolean;
}

/**
 * Tells the editor's own saves apart from edits made elsewhere (another
 * device, an AI tool, a text editor). The watcher reports a save some time
 * after it lands, often after the user has typed more, so the editor's text
 * alone cannot identify it. Instead this tracks the text last known to be on
 * disk plus any saves still in flight. Older saves are deliberately forgotten,
 * so restoring a previous version from outside still counts as an outside edit.
 */
export class DiskSync {
  private path: string | null = null;
  private onDisk: string | null = null;
  private inFlight: string[] = [];

  /** A note was opened with `text` read from disk. */
  opened(path: string, text: string): void {
    this.path = path;
    this.onDisk = text;
    this.inFlight = [];
  }

  /** A save of `text` to `path` was sent. */
  saving(path: string, text: string): void {
    if (path === this.path) this.inFlight.push(text);
  }

  /** The open note moved from `from` to `to` on disk, with the same text. */
  moved(from: string, to: string): void {
    if (from === this.path) this.path = to;
  }

  /** Texts `path` may hold on disk: the last one seen plus saves in flight. */
  known(path: string): string[] {
    if (path !== this.path) return [];
    return this.onDisk === null ? [...this.inFlight] : [this.onDisk, ...this.inFlight];
  }

  /** A save sent with `saving` finished, successfully or not. */
  saved(path: string, text: string, ok: boolean): void {
    if (path !== this.path) return;
    const i = this.inFlight.indexOf(text);
    if (i !== -1) this.inFlight.splice(i, 1);
    if (ok) this.onDisk = text;
  }

  /**
   * `path` changed on disk and now holds `disk`, while the editor shows
   * `editor`. Records `disk` as seen, so a repeated report is ignored.
   */
  diskChanged(path: string, disk: string, editor: string, dirty: boolean): DiskChange {
    if (path !== this.path) return "ignore";
    if (disk === this.onDisk || this.inFlight.includes(disk)) return "ignore";
    this.onDisk = disk;
    if (disk === editor) return "ignore";
    return dirty ? "conflict" : "reload";
  }
}

const fileName = (relPath: string) => (relPath.split("/").pop() ?? relPath).toLowerCase();

/**
 * Notes added in the same batch that removed `path`, in the order to check
 * them for a move. A moved note usually keeps its name, and a folder move
 * reports every note in the folder, so notes with the same name come first.
 */
export function moveCandidates(path: string, changes: VaultChange[]): string[] {
  const name = fileName(path);
  const added = changes
    .filter((c) => !c.removed && c.rel_path !== path && /\.md$/i.test(c.rel_path))
    .map((c) => c.rel_path);
  return [
    ...added.filter((p) => fileName(p) === name),
    ...added.filter((p) => fileName(p) !== name),
  ];
}

/**
 * Where the open note `path` went when it was removed on disk: the first note
 * added in the same batch that holds a text it is known to have (the text last
 * seen on disk, a save in flight, or what the editor shows). Null when nothing
 * matches, which means the note was deleted. Line endings do not matter.
 */
export async function findMovedNote(
  path: string,
  changes: VaultChange[],
  known: string[],
  read: (relPath: string) => Promise<string>,
  limit = 25,
): Promise<string | null> {
  const texts = new Set(known.map(toLF));
  if (texts.size === 0) return null;
  for (const candidate of moveCandidates(path, changes).slice(0, limit)) {
    try {
      if (texts.has(toLF(await read(candidate)))) return candidate;
    } catch {
      /* gone again, or unreadable: not this one */
    }
  }
  return null;
}

/** A sibling path for unsaved edits to a note that was deleted on disk. */
export function recoveryPath(path: string, stamp: number): string {
  return `${path.replace(/\.md$/i, "")}.recovered-${stamp}.md`;
}

/** What the editor should do about the open note changing on disk. */
export type DiskChange = "ignore" | "reload" | "conflict";

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

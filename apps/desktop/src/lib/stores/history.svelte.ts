/**
 * Undo/redo for the project: JSON snapshots taken once edits pause (so a
 * drag or a typed word is one step), capped at 100 steps. The same
 * snapshots tell whether the project has unsaved changes.
 */
const LIMIT = 100;
const SETTLE_MS = 400;

export function snapshotOf(project: unknown): string {
  // modifiedAt changes on every touch(); it is not an edit by itself.
  return JSON.stringify({ ...(project as Record<string, unknown>), modifiedAt: "" });
}

export class ProjectHistory {
  private undoStack: string[] = [];
  private redoStack: string[] = [];
  private committed: string | null = null;
  private saved: string | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private pending: (() => unknown) | null = null;

  canUndo = $state(false);
  canRedo = $state(false);
  dirty = $state(false);

  private update(): void {
    this.canUndo = this.undoStack.length > 0;
    this.canRedo = this.redoStack.length > 0;
    this.dirty = this.saved !== null && this.committed !== this.saved;
  }

  /** New baseline (project opened or created): clears both stacks. */
  reset(project: unknown | null): void {
    this.cancel();
    this.undoStack = [];
    this.redoStack = [];
    this.committed = project ? snapshotOf(project) : null;
    this.saved = this.committed;
    this.update();
  }

  /** A recovered project: everything counts as unsaved. */
  markUnsaved(): void {
    this.saved = "";
    this.update();
  }

  /** The current state was written to disk. */
  markSaved(project: unknown): void {
    this.flush();
    this.saved = snapshotOf(project);
    if (this.committed === null) this.committed = this.saved;
    this.update();
  }

  /** Called after every edit; commits once edits pause. */
  schedule(read: () => unknown): void {
    this.pending = read;
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => this.flush(), SETTLE_MS);
  }

  flush(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    const read = this.pending;
    this.pending = null;
    if (!read) return;
    const project = read();
    if (!project) return;
    const next = snapshotOf(project);
    if (next === this.committed) return;
    if (this.committed !== null) {
      this.undoStack.push(this.committed);
      if (this.undoStack.length > LIMIT) this.undoStack.shift();
    }
    this.redoStack = [];
    this.committed = next;
    this.update();
  }

  private cancel(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.pending = null;
  }

  /** Returns the snapshot to restore, or null. */
  undo(): string | null {
    this.flush();
    const previous = this.undoStack.pop();
    if (previous === undefined || this.committed === null) return null;
    this.redoStack.push(this.committed);
    this.committed = previous;
    this.update();
    return previous;
  }

  redo(): string | null {
    this.flush();
    const next = this.redoStack.pop();
    if (next === undefined || this.committed === null) return null;
    this.undoStack.push(this.committed);
    this.committed = next;
    this.update();
    return next;
  }
}

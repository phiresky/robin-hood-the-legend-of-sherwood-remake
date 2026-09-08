import { MapSession } from "./session.ts";

export interface SessionSnapshot<T> {
  name: string;
  document: T;
  dirty: boolean;
  past: T[];
  future: T[];
}

/** One publication feeds the reactive UI; no caller can forget the dirty/history update. */
export class SessionPublication<T, R> {
  private session = new MapSession<T, R>();
  private disposed = false;
  private changed: (
    snapshot: SessionSnapshot<T>,
    reason: "load" | "revision" | "saved",
  ) => void;
  constructor(
    changed: (
      snapshot: SessionSnapshot<T>,
      reason: "load" | "revision" | "saved",
    ) => void,
  ) {
    this.changed = changed;
  }
  get current() {
    return this.session.current;
  }
  dispose() {
    this.disposed = true;
    this.session.beginLoad();
  }
  beginLoad() {
    return this.session.beginLoad();
  }
  isCurrent(generation: number) {
    return this.session.isCurrent(generation);
  }
  publish(...args: Parameters<MapSession<T, R>["publish"]>) {
    if (this.disposed) return false;
    if (!this.session.publish(...args)) return false;
    this.notify("load");
    return true;
  }
  edit(document: T) {
    this.session.edit(document);
    this.notify("revision");
  }
  undo() {
    this.session.undo();
    this.notify("revision");
  }
  redo() {
    this.session.redo();
    this.notify("revision");
  }
  captureSave() {
    return this.session.captureSave();
  }
  saved(snapshot: ReturnType<MapSession<T, R>["captureSave"]>) {
    this.session.saved(snapshot);
    if (this.current === snapshot.session) this.notify("saved");
  }
  private notify(reason: "load" | "revision" | "saved") {
    if (this.disposed) return;
    const current = this.current;
    if (!current) return; // Undo/redo before any document is an intentional UI no-op.
    this.changed(
      {
        name: current.name,
        document: current.document,
        dirty: this.session.dirty,
        past: [...current.past],
        future: [...current.future],
      },
      reason,
    );
  }
}

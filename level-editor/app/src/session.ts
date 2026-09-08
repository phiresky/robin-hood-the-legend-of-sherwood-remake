/** Identity and immutable revision snapshots travel together across asynchronous I/O. */
export class MapSession<T, R> {
  private generation = 0;
  current: {
    name: string;
    document: T;
    saved: T | null;
    resources: R;
    past: T[];
    future: T[];
  } | null = null;
  beginLoad() {
    return ++this.generation;
  }
  isCurrent(generation: number) {
    return this.generation === generation;
  }
  publish(
    generation: number,
    name: string,
    document: T,
    resources: R,
    saved = true,
  ) {
    if (!this.isCurrent(generation)) return false;
    this.current = {
      name,
      document,
      resources,
      saved: saved ? document : null,
      past: [],
      future: [],
    };
    return true;
  }
  get dirty() {
    return !!this.current && this.current.document !== this.current.saved;
  }
  edit(document: T) {
    const s = this.current;
    if (!s) throw new Error("No loaded map");
    s.past = [...s.past.slice(-99), s.document];
    s.future = [];
    s.document = document;
  }
  undo() {
    const s = this.current;
    if (!s?.past.length) return;
    s.future.unshift(s.document);
    s.document = s.past.pop()!;
  }
  redo() {
    const s = this.current;
    if (!s?.future.length) return;
    s.past.push(s.document);
    s.document = s.future.shift()!;
  }
  captureSave() {
    const session = this.current;
    if (!session) throw new Error("No loaded map");
    return {
      session,
      document: session.document,
      name: session.name,
      resources: session.resources,
    };
  }
  saved(snapshot: ReturnType<MapSession<T, R>["captureSave"]>) {
    if (this.current === snapshot.session)
      snapshot.session.saved = snapshot.document;
  }
}

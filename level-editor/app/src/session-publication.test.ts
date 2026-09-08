import test from "node:test";
import assert from "node:assert/strict";
import {
  SessionPublication,
  type SessionSnapshot,
} from "./session-publication.ts";

test("one publication carries revision identity, dirty and immutable history snapshots", () => {
  const events: { snapshot: SessionSnapshot<object>; reason: string }[] = [];
  const session = new SessionPublication<object, string>((snapshot, reason) =>
    events.push({ snapshot, reason }),
  );
  const first = {},
    second = {},
    third = {};
  session.publish(session.beginLoad(), "A", first, "A-directory");
  session.edit(second);
  const save = session.captureSave();
  session.edit(third);
  session.saved(save);
  assert.equal(events.at(-1)!.snapshot.dirty, true);
  session.undo();
  assert.equal(events.at(-1)!.snapshot.document, second);
  assert.equal(events.at(-1)!.snapshot.dirty, false);
  assert.deepEqual(events[2]!.snapshot.past, [first, second]);
  session.redo();
  assert.equal(events.at(-1)!.snapshot.document, third);
  assert.equal(events.at(-1)!.snapshot.dirty, true);
  assert.deepEqual(
    events.map((event) => event.reason),
    ["load", "revision", "revision", "saved", "revision", "revision"],
  );
});

test("stale load/save completion emits no reactive update for its successor", () => {
  const events: SessionSnapshot<object>[] = [];
  const session = new SessionPublication<object, string>((snapshot) =>
    events.push(snapshot),
  );
  const a = session.beginLoad();
  session.publish(a, "A", {}, "A-directory");
  const save = session.captureSave();
  const b = session.beginLoad();
  session.publish(b, "B", {}, "B-directory", false);
  const count = events.length;
  assert.equal(session.publish(a, "A", {}, "old-directory"), false);
  session.saved(save);
  assert.equal(events.length, count);
  assert.equal(events.at(-1)!.name, "B");
  assert.equal(events.at(-1)!.dirty, true);
});

test("completion after unmount cannot publish into the disposed reactive owner", () => {
  let publications = 0;
  const session = new SessionPublication<object, string>(() => publications++);
  const generation = session.beginLoad();
  session.publish(generation, "A", {}, "A-directory");
  const save = session.captureSave();
  session.dispose();
  session.saved(save);
  assert.equal(session.isCurrent(generation), false);
  assert.equal(session.publish(generation, "A", {}, "old-directory"), false);
  assert.equal(publications, 1);
});

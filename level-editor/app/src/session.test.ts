import test from "node:test";
import assert from "node:assert/strict";
import { MapSession } from "./session.ts";

test("reverse load completion cannot publish old identity or resources", () => {
  const session = new MapSession<object, string>();
  const a = session.beginLoad();
  const b = session.beginLoad();
  assert.equal(session.publish(b, "B", {}, "B-directory"), true);
  assert.equal(session.publish(a, "A", {}, "A-directory"), false);
  assert.equal(session.current?.name, "B");
  assert.equal(session.current?.resources, "B-directory");
});
test("failed replacement preserves edited session", () => {
  const session = new MapSession<object, string>();
  session.publish(session.beginLoad(), "A", {}, "A-directory");
  session.edit({ edited: true });
  const previous = session.current;
  session.beginLoad(); // failed preparation never publishes
  assert.equal(session.current, previous);
  assert.equal(session.dirty, true);
});
test("save acknowledges exactly its revision and undo returns to saved revision", () => {
  const session = new MapSession<object, string>();
  session.publish(session.beginLoad(), "A", {}, "A-directory");
  session.edit({ first: true });
  const snapshot = session.captureSave();
  session.edit({ second: true });
  session.saved(snapshot);
  assert.equal(session.dirty, true);
  session.undo();
  assert.equal(session.dirty, false);
  session.redo();
  assert.equal(session.dirty, true);
  session.publish(session.beginLoad(), "B", {}, "B-directory", false);
  session.saved(snapshot);
  assert.equal(session.dirty, true);
  assert.equal(snapshot.resources, "A-directory");
});

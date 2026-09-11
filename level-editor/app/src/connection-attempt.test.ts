import test from "node:test";
import assert from "node:assert/strict";
import { connectionAttempts, connectLatest } from "./connection-attempt.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test("startup owns its token before restore, so a newer pick wins", async () => {
  const attempts = connectionAttempts();
  const restore = deferred<string>();
  const writes: string[] = [];
  const old = connectLatest(attempts, async (current) => {
    const handle = await restore.promise;
    if (current()) writes.push(handle);
  }, () => assert.fail("unexpected failure"), () => writes.push("old finally"));
  await connectLatest(attempts, async (current) => {
    if (current()) writes.push("picked");
  }, () => assert.fail("unexpected failure"));
  restore.resolve("restored");
  await old;
  assert.deepEqual(writes, ["picked"]);
});

test("stale failures/finalizers and unmounted operations cannot publish", async () => {
  for (const retire of ["replace", "dispose"] as const) {
    const attempts = connectionAttempts();
    const pending = deferred<void>();
    const old = connectLatest(attempts, () => pending.promise,
      () => assert.fail("retired error"), () => assert.fail("retired finalizer"));
    if (retire === "replace") attempts.begin();
    else attempts.dispose();
    pending.reject(new Error("permission failure"));
    await old;
  }
});

test("datadir and library attempts do not invalidate each other", async () => {
  const datadir = connectionAttempts();
  const library = connectionAttempts();
  const dataCurrent = datadir.begin();
  const libraryCurrent = library.begin();
  datadir.begin();
  assert.equal(dataCurrent(), false);
  assert.equal(libraryCurrent(), true);
});

test("current reconnect failures are handled and picker cancellation is quiet", async () => {
  const attempts = connectionAttempts();
  const errors: unknown[] = [];
  let finished = 0;
  const failure = new Error("permission rejected");
  await connectLatest(attempts, async () => { throw failure; },
    (error) => errors.push(error), () => finished++);
  await connectLatest(attempts, async () => { throw new DOMException("cancelled", "AbortError"); },
    (error) => errors.push(error), () => finished++);
  assert.deepEqual(errors, [failure]);
  assert.equal(finished, 2);
  attempts.dispose();
  await connectLatest(attempts, async () => assert.fail("started after unmount"),
    () => assert.fail("unmounted error"));
});

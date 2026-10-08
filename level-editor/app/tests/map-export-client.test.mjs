import test from "node:test";
import assert from "node:assert/strict";
import { MapExportWorker } from "../src/map-export-client.ts";
import { wallSplineFixture } from "../../shared/test-fixtures/wall-spline.ts";

function fakeWorker(t) {
  const original = Object.getOwnPropertyDescriptor(globalThis, "Worker");
  const workers = [];
  class FakeWorker {
    terminated = false;
    constructor() {
      workers.push(this);
    }
    postMessage(request) {
      this.request = request;
    }
    terminate() {
      this.terminated = true;
    }
    emit(data) {
      this.onmessage({ data });
    }
  }
  Object.defineProperty(globalThis, "Worker", { configurable: true, value: FakeWorker });
  t.after(() => {
    if (original) Object.defineProperty(globalThis, "Worker", original);
    else delete globalThis.Worker;
  });
  return workers;
}

test("export progress leaves compilation pending and cancellation rejects it", async (t) => {
  const workers = fakeWorker(t);
  const stages = [];
  const client = new MapExportWorker((stage) => stages.push(stage));
  const worker = workers[0];
  const f = wallSplineFixture();
  let settled = false;
  const pending = client.compile(f.document, f.bounds, f.assets);
  void pending.then(
    () => {
      settled = true;
    },
    () => {
      settled = true;
    },
  );
  worker.emit({ kind: "progress", stage: "Constructing terrain" });
  worker.emit({ kind: "progress", stage: "Constructing masks" });
  await Promise.resolve();
  assert.equal(settled, false);
  assert.deepEqual(stages, ["Constructing terrain", "Constructing masks"]);
  await assert.rejects(client.compile(f.document, f.bounds, f.assets), /already busy/);
  client.dispose();
  await assert.rejects(pending, /cancelled/);
  worker.emit({ kind: "progress", stage: "Late progress" });
  assert.equal(stages.length, 2);
  assert.equal(worker.terminated, true);
});

test("export progress preserves the final response and callback errors reject pending work", async (t) => {
  const workers = fakeWorker(t);
  const f = wallSplineFixture();
  const client = new MapExportWorker();
  const pending = client.compile(f.document, f.bounds, f.assets);
  workers[0].emit({ kind: "progress", stage: "Constructing masks" });
  const compiled = { name: "transport-fixture" };
  workers[0].emit({ kind: "compiled", compiled });
  assert.equal(await pending, compiled);
  client.dispose();

  const failing = new MapExportWorker(() => {
    throw new Error("Progress cancelled");
  });
  const rejected = failing.compile(f.document, f.bounds, f.assets);
  workers[1].emit({ kind: "progress", stage: "Constructing terrain" });
  await assert.rejects(rejected, /Progress cancelled/);
  assert.equal(workers[1].terminated, true);
});

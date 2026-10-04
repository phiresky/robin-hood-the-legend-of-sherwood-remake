import test from "node:test";
import assert from "node:assert/strict";
import { sceneryFrameAtTick } from "./scenery-frames.ts";

test("scenery preview timing matches native sentinel, inclusive delays and loops", () => {
  assert.deepEqual(
    Array.from({ length: 12 }, (_, tick) => sceneryFrameAtTick([2, 4], tick)),
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0],
  );
  assert.deepEqual(
    Array.from({ length: 7 }, (_, tick) => sceneryFrameAtTick([0, 0], tick)),
    [0, 0, 1, 0, 1, 0, 1],
  );
  assert.equal(sceneryFrameAtTick([2, 4], 8_000_001), 0);
});

test("maximum unsigned delay freezes on that frame and malformed timing is rejected", () => {
  assert.equal(sceneryFrameAtTick([0, 65535, 0], 1), 0);
  for (const tick of [2, 65537, 1_000_000])
    assert.equal(sceneryFrameAtTick([0, 65535, 0], tick), 1);
  for (const delays of [[], [-1], [0.5], [65536]])
    assert.throws(() => sceneryFrameAtTick(delays, 1));
  assert.throws(() => sceneryFrameAtTick([0], -1));
});

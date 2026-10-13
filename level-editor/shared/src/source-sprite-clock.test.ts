import test from "node:test";
import assert from "node:assert/strict";
import { sourceSpriteTick } from "./source-sprite-clock.ts";

test("source counters preserve reset, frame boundaries, holds and independent final phases", () => {
  const transition = [{ delay: 1 }, { delay: 1 }, { delay: 1 }];
  assert.equal(sourceSpriteTick(transition, { frame: 0, count: 65535 }), 0);
  assert.equal(sourceSpriteTick(transition, { frame: 0, count: 0 }), 0);
  assert.equal(sourceSpriteTick(transition, { frame: 0, count: 1 }), 1);
  assert.equal(sourceSpriteTick(transition, { frame: 1, count: 0 }), 2);
  assert.equal(sourceSpriteTick(transition, { frame: 2, count: 1 }), 5);
  const final = [{ delay: 3 }, { delay: 0 }];
  assert.equal(sourceSpriteTick(final, { frame: 0, count: 65535 }), 0);
  assert.equal(sourceSpriteTick(final, { frame: 1, count: 0 }), 4);
  assert.equal(sourceSpriteTick(final, { frame: 1, count: 0 }), 4);
  assert.throws(() => sourceSpriteTick(transition, { frame: 1, count: 65535 }), /reset/);
  assert.throws(() => sourceSpriteTick(transition, { frame: 3, count: 0 }), /Invalid/);
  assert.throws(() => sourceSpriteTick(transition, { frame: 0, count: 2 }), /exceeds/);
  assert.throws(() => sourceSpriteTick([], { frame: 0, count: 0 }), /Invalid/);
});

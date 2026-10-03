import test from "node:test";
import assert from "node:assert/strict";
import {
  assembleLiftSegments,
  UnavailableLiftJoin,
  type PlacedLiftSegment,
} from "./assemble-lift-segments.ts";

test("lift sockets reject ambiguous or unpaired authoring instead of picking an owner", () => {
  const a: PlacedLiftSegment = { id: "a", type: 1, direction: 4, joins: [[1, 2, 3]] };
  const b = { ...a, id: "b" };
  assert.throws(() => assembleLiftSegments([a]), /exactly one other/);
  assert.throws(
    () => assembleLiftSegments([a, b, { ...a, id: "c" }]),
    (error) => {
      assert.ok(error instanceof UnavailableLiftJoin);
      assert.deepEqual(error.segments, ["a", "b", "c"]);
      return true;
    },
  );
  assert.throws(
    () =>
      assembleLiftSegments([
        {
          ...a,
          joins: [
            [1, 2, 3],
            [1, 2, 3],
          ],
        },
      ]),
    /exactly one other/,
  );
  const result = assembleLiftSegments([a, b]);
  assert.equal(result.lifts.length, 1);
  assert.equal(result.identities.get("a"), result.identities.get("b"));
});

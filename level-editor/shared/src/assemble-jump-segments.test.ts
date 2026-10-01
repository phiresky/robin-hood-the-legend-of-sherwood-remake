import test from "node:test";
import assert from "node:assert/strict";
import { assembleJumpSegments, type PlacedJumpSegment } from "./assemble-jump-segments.ts";

test("geometric edges connect new neighbours and trim to the overlapping span", () => {
  const rules = { maxGap: 30, maxRise: 20, maxDrop: 20, minOverlap: 5 };
  const a: PlacedJumpSegment = {
    id: "a",
    long: true,
    attachment: rules,
    edge: { zone: "a", a: [0, 40, 0], b: [0, 0, 0] },
  };
  const b: PlacedJumpSegment = {
    id: "new-neighbour",
    long: true,
    attachment: rules,
    edge: { zone: "b", a: [10, 20, 10], b: [10, 60, 10] },
  };
  const expected = [
    { zone: "a", a: [0, 40, 0], b: [0, 10, 0] },
    { zone: "b", a: [10, 20, 10], b: [10, 50, 10] },
  ];
  assert.deepEqual(assembleJumpSegments([a, b]).pairs[0]!.edges, expected);
  assert.deepEqual(assembleJumpSegments([b, a]).pairs[0]!.edges, [...expected].reverse());
  // Rotate in the map plane, then translate and elevate an independent copy.
  const transform = ([x, y, z]: [number, number, number]): [number, number, number] => [
    200 - (y - z),
    300 + x + z + 50,
    z + 50,
  ];
  const copy = (segment: PlacedJumpSegment): PlacedJumpSegment => ({
    ...segment,
    id: `${segment.id}-copy`,
    edge: {
      zone: `${segment.edge.zone}-copy`,
      a: transform(segment.edge.a),
      b: transform(segment.edge.b),
    },
  });
  const duplicated = assembleJumpSegments([a, copy(b), b, copy(a)]);
  assert.equal(duplicated.pairs.length, 2);
  assert.equal(duplicated.unmatched.length, 0);
  assert.deepEqual(
    duplicated.pairs[1]!.edges,
    [...expected].reverse().map((edge) => ({
      zone: `${edge.zone}-copy`,
      a: transform(edge.a as [number, number, number]),
      b: transform(edge.b as [number, number, number]),
    })),
  );
  for (const invalid of [
    { ...b, attachment: undefined },
    { ...b, attachment: { ...rules, maxGap: 5 } },
    { ...b, attachment: { ...rules, maxDrop: 5 } },
    { ...b, attachment: { ...rules, minOverlap: 35 } },
    { ...b, edge: { ...b.edge, a: b.edge.b, b: b.edge.a } },
    { ...b, edge: { ...b.edge, b: [12, 20, 10] as [number, number, number] } },
    {
      ...b,
      edge: {
        zone: "b",
        a: [-10, 20, 10] as [number, number, number],
        b: [-10, 60, 10] as [number, number, number],
      },
    },
    {
      ...b,
      edge: {
        zone: "b",
        a: [10, 60, 10] as [number, number, number],
        b: [10, 100, 10] as [number, number, number],
      },
    },
  ])
    assert.equal(assembleJumpSegments([a, invalid]).pairs.length, 0);
  assert.throws(() => assembleJumpSegments([a, b, { ...b, id: "duplicate" }]), /exactly one/);
  assert.equal(assembleJumpSegments([a, { ...b, long: false }]).pairs.length, 0);
});

test("jump socket matching rejects ambiguity and conflicting traversal rules", () => {
  const a: PlacedJumpSegment = {
    id: "a",
    long: true,
    join: [50, 50, 50],
    edge: { zone: "low", a: [0, 0, 0], b: [0, 10, 0] },
  };
  const b: PlacedJumpSegment = {
    ...a,
    id: "b",
    edge: { zone: "high", a: [100, 0, 100], b: [100, 10, 100] },
  };
  assert.equal(assembleJumpSegments([a, b]).pairs.length, 1);
  assert.deepEqual(assembleJumpSegments([a]), { pairs: [], unmatched: [a], warnings: [] });
  assert.throws(() => assembleJumpSegments([a, b, { ...b, id: "c" }]), /exactly one complementary/);
  assert.throws(() => assembleJumpSegments([a, { ...b, long: false }]), /rules disagree/);
  assert.throws(() => assembleJumpSegments([a, { ...b, edge: a.edge }]), /same landing zone/);
});

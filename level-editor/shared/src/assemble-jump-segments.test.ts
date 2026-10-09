import test from "node:test";
import assert from "node:assert/strict";
import { assembleJumpSegments, type PlacedJumpSegment } from "./assemble-jump-segments.ts";

test("clearance exposed by movement-grid snapping trims again instead of discarding the connection", () => {
  const attachment = { maxGap: 50, maxRise: 20, maxDrop: 20, minOverlap: 8 };
  const segments: PlacedJumpSegment[] = [
    { id: "a", long: true, attachment, edge: { zone: "a", a: [0, 40, 0], b: [0, 0, 0] } },
    { id: "b", long: true, attachment, edge: { zone: "b", a: [30, 0, 0], b: [30, 40, 0] } },
  ];
  // Successive shortened headings can reveal another blocked portion. The
  // clearance contract applies to each snapped candidate, not just the first.
  const result = assembleJumpSegments(segments, (edges) => (edges[0].a[1] > 24 ? [[0, 0.1]] : []));
  assert.equal(result.pairs.length, 1);
  const edge = result.pairs[0]!.edges[0];
  assert.ok(edge.a[1] <= 24);
  assert.equal(edge.b[1], 0, "an unobstructed far endpoint must not be trimmed");
  assert.ok(Math.hypot(edge.b[0] - edge.a[0], edge.b[1] - edge.a[1]) >= attachment.minOverlap);
  assert.equal(assembleJumpSegments(segments, () => [[0, 1]]).pairs.length, 0);
});

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
  assert.equal(assembleJumpSegments([a, b, { ...b, id: "duplicate" }]).pairs.length, 1);
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

test("duplicate jump ledges produce one connection regardless of placement order", () => {
  const attachment = { maxGap: 30, maxRise: 20, maxDrop: 20, minOverlap: 5 };
  const left: PlacedJumpSegment = {
    id: "left",
    long: true,
    attachment,
    edge: { zone: "left-zone", a: [0, 40, 0], b: [0, 0, 0] },
  };
  const right: PlacedJumpSegment = {
    id: "right",
    long: true,
    attachment,
    edge: { zone: "right-zone", a: [10, 0, 0], b: [10, 40, 0] },
  };
  const duplicate = { ...left, id: "left-other-fragment" };
  for (const order of [
    [left, right, duplicate],
    [right, left, duplicate],
    [left, duplicate, right],
    [duplicate, right, left],
    [right, duplicate, left],
    [duplicate, left, right],
  ])
    assert.equal(assembleJumpSegments(order).pairs.length, 1);
  const independent = { ...duplicate, edge: { ...duplicate.edge, zone: "independent-zone" } };
  assert.equal(
    assembleJumpSegments([left, right, independent]).pairs.length,
    2,
    "Coincident but independently owned landing zones must remain distinct",
  );
  const guarded = {
    ...duplicate,
    attachment: { ...attachment, clearance: { radius: 5, height: 70 } },
  };
  assert.equal(
    assembleJumpSegments([left, right, guarded]).pairs.length,
    2,
    "Different clearance policies must reach their own clearance checks",
  );
});

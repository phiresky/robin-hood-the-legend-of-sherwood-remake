import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import clipping, { type MultiPolygon } from "polygon-clipping";
import { assembleMovementContour } from "./assemble-movement-contour.ts";
import { preserveMovementBoundary } from "./preserve-movement-boundary.ts";
import fragmentsFixture from "../test-fixtures/near-coincident-contour-fragments.json" with { type: "json" };

test("near-coincident fragments recover from a sweep failure without changing covered area", () => {
  const fragments = fragmentsFixture as MultiPolygon;
  // Limit only this subprocess's clipping budget so reproducing the captured
  // numerical loop does not allocate a million segments in the test runner.
  const script = `
    import assert from 'node:assert/strict';
    import fs from 'node:fs';
    import clipping from ${JSON.stringify(import.meta.resolve("polygon-clipping"))};
    import { assembleMovementContour } from ${JSON.stringify(new URL("./assemble-movement-contour.ts", import.meta.url).href)};
    const fragments = JSON.parse(fs.readFileSync(0, 'utf8'));
    const before = structuredClone(fragments);
    assert.throws(() => clipping.union(fragments), /Infinite loop when/);
    const result = assembleMovementContour(fragments);
    assert.deepEqual(fragments, before);
    process.stdout.write(JSON.stringify(result));
  `;
  const result: MultiPolygon = JSON.parse(
    execFileSync(process.execPath, ["--input-type=module", "-e", script], {
      input: JSON.stringify(fragments),
      encoding: "utf8",
      timeout: 5000,
      env: { ...process.env, POLYGON_CLIPPING_MAX_SWEEPLINE_SEGMENTS: "10000" },
    }),
  );
  assert.equal(result.length, 1);
  const area = result[0]!.reduce((sum, ring, index) => {
    const signed =
      ring.reduce((area, p, i) => {
        const q = ring[(i + 1) % ring.length]!;
        return area + p[0] * q[1] - q[0] * p[1];
      }, 0) / 2;
    return sum + (index === 0 ? 1 : -1) * Math.abs(signed);
  }, 0);
  assert.ok(Math.abs(area - 114496) < 0.00001);
});

test("matching fragment endpoints tolerate clipping noise without moving separate placements", () => {
  const unit = 1 / 1048576;
  const fragments: MultiPolygon = [
    [
      [
        [0, 0],
        [10, 0],
        [10, 10],
        [0, 10],
      ],
    ],
    [
      [
        [10 + unit, unit],
        [20, 0],
        [20, 10],
        [10 + unit, 10 + unit],
      ],
    ],
  ];
  const before = structuredClone(fragments);
  const assembled = assembleMovementContour(fragments);
  assert.deepEqual(
    clipping.xor(assembled, [
      [
        [0, 0],
        [20, 0],
        [20, 10],
        [0, 10],
      ],
    ]),
    [],
  );
  assert.deepEqual(assembleMovementContour([...fragments].reverse()), assembled);
  assert.deepEqual(fragments, before);
  const rotate = ([x, y]: [number, number]): [number, number] => [
    (x - y) / Math.SQRT2,
    (x + y) / Math.SQRT2,
  ];
  assert.equal(
    assembleMovementContour(fragments.map((p) => p.map((r) => r.map(rotate)))).length,
    1,
  );
  const detached = structuredClone(fragments);
  detached[1] = detached[1]!.map((ring) => ring.map(([x, y]) => [x + 8 * unit, y]));
  assert.equal(assembleMovementContour(detached).length, 2);
});

test("joining noisy contour fragments does not add rounded corners to their shared outline", () => {
  const unit = 1 / 1048576;
  const boundary: [number, number][] = [
    [-20, -20],
    [60, -20],
    [60, 60],
    [-20, 60],
  ];
  const fragments: MultiPolygon = [
    [
      [
        [0, 0],
        [6, 11.7],
        [5, 39.5],
        [-10, 40],
        [-10, 0],
      ],
    ],
    [
      [
        [6 + unit, 11.7 + 2 * unit],
        [20, 39],
        [5, 39.5],
      ],
    ],
  ];
  const joined = preserveMovementBoundary(boundary, fragments, [], ["wall", "wall"]);
  assert.deepEqual(
    clipping.xor(
      joined.blockers.map((p) => [p]),
      [
        [
          [0, 0],
          [20, 39],
          [-10, 40],
          [-10, 0],
        ],
      ],
    ),
    [],
  );
  const separate = preserveMovementBoundary(boundary, fragments, [], ["first", "second"]);
  assert.equal(separate.blockers.length, 2);
});

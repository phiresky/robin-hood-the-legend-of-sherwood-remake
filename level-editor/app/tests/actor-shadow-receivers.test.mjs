import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import {
  projectShadowReceivers,
  projectedTerrainReceivers,
  evaluatePhysicalReceiver,
  shadowCoverageBounds,
} from "../src/actor-shadow-receivers.ts";

{
  const base = {
    anchor: [0, 0, 0],
    bounds: { left: 0, top: 0, width: 10, height: 10 },
    elevation: Math.PI / 4,
  };
  const rectangle = (x0, y0, x1, y1, height) => {
    const p = (x, y) => [x, y, typeof height === "function" ? height(x, y) : height];
    return [
      [p(x0, y0), p(x1, y0), p(x1, y1)],
      [p(x0, y0), p(x1, y1), p(x0, y1)],
    ];
  };
  const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-8, `${a} != ${b}`);
  test("sloped receivers clip to exact frame footprint with matching UVs and heights", () => {
    const got = projectShadowReceivers({
      ...base,
      triangles: rectangle(-5, -5, 15, 15, (x, y) => 2 * x + 3 * y),
    });
    near(got.coveredArea, 100);
    for (let i = 0; i < got.positions.length / 3; i++) {
      const x = got.positions[i * 3],
        u = got.uvs[i * 2],
        v = got.uvs[i * 2 + 1],
        y = 10 * (1 - v);
      near(u, x / 10);
      assert.ok(u >= 0 && u <= 1 && v >= 0 && v <= 1);
      near(got.positions[i * 3 + 1], (2 * x + 3 * y) / Math.cos(base.elevation) + 0.15);
      near(got.positions[i * 3 + 2], (2 * x + 4 * y) / Math.sin(base.elevation));
    }
  });
  test("interior peak and breaklines survive; the four-corner plane limitation is removed", () => {
    const c = [5, 5, 8],
      corners = [
        [0, 0, 0],
        [10, 0, 0],
        [10, 10, 0],
        [0, 10, 0],
      ];
    const got = projectShadowReceivers({
      ...base,
      triangles: corners.map((p, i) => [p, corners[(i + 1) % 4], c]),
    });
    assert.equal(got.ranges.length, 4);
    const peaks = [];
    for (let i = 0; i < got.positions.length / 3; i++)
      if (got.uvs[i * 2] === 0.5 && got.uvs[i * 2 + 1] === 0.5)
        peaks.push(got.positions[i * 3 + 1]);
    assert.equal(peaks.length, 4);
    for (const y of peaks) near(y, 8 / Math.cos(base.elevation) + 0.15);
  });
  test("raised receiver edge remains disconnected with no invented vertical or ramp faces", () => {
    const got = projectShadowReceivers({
      ...base,
      triangles: [...rectangle(0, 0, 5, 10, 0), ...rectangle(5, 0, 10, 10, 7)],
    });
    const heights = new Set();
    for (let i = 0; i < got.indices.length; i += 3) {
      const ys = got.indices.slice(i, i + 3).map((n) => got.positions[n * 3 + 1]);
      near(ys[0], ys[1]);
      near(ys[1], ys[2]);
      heights.add(ys[0]);
    }
    assert.equal(heights.size, 2);
    near(got.coveredArea, 100);
  });
  test("holes and overlapping ambiguous receivers fail even when total areas cancel", () => {
    assert.throws(
      () => projectShadowReceivers({ ...base, triangles: rectangle(0, 0, 9, 10, 0) }),
      /Incomplete support/,
    );
    const lower = rectangle(0, 0, 5, 10, 0);
    assert.throws(
      () => projectShadowReceivers({ ...base, triangles: [...lower, ...lower] }),
      /Overlapping/,
    );
    assert.throws(() => projectShadowReceivers({ ...base, triangles: [] }), /Missing support/);
    assert.throws(
      () =>
        projectShadowReceivers({
          ...base,
          triangles: [
            [
              [0, 0, 0],
              [0, 0, 2],
              [0, 10, 2],
            ],
          ],
        }),
      /Vertical or degenerate/,
    );
  });
  test("reversed winding and elevated nonzero anchor preserve native footprint coordinates", () => {
    const elevation = Math.PI / 6,
      anchor = [100, 20, 80],
      mapY = 80 * Math.sin(elevation) - 20 * Math.cos(elevation);
    const bounds = { left: -2, top: 7, width: 12, height: 9 };
    const triangles = rectangle(98, mapY - 7, 110, mapY + 2, (x, y) => x / 5 + y / 3).map((t) =>
      t.toReversed(),
    );
    const got = projectShadowReceivers({ anchor, bounds, elevation, triangles });
    near(got.coveredArea, 108);
    for (let i = 0; i < got.positions.length / 3; i++) {
      const u = got.uvs[i * 2],
        v = got.uvs[i * 2 + 1],
        x = 98 + 12 * u,
        y = mapY + 2 - 9 * v;
      near(got.positions[i * 3], x - 100);
      near(
        got.positions[i * 3 + 1],
        (x / 5 + y / 3 - 20 * Math.cos(elevation)) / Math.cos(elevation) + 0.15,
      );
    }
    assert.equal(got.indices.length, 6);
  });
  test("shadow-only coverage keeps full-frame UVs without requiring terrain under transparent body pixels", () => {
    const bounds = { left: 0, top: 10, width: 10, height: 10 },
      coverage = { left: 2, top: 4, width: 4, height: 2 };
    const triangles = [
      [
        [2, -4, 0],
        [6, -4, 0],
        [2, -2, 0],
      ],
      [
        [6, -4, 0],
        [6, -2, 0],
        [2, -2, 0],
      ],
    ];
    const args = { anchor: [0, 0, 0], bounds, coverage, elevation: Math.PI / 4, triangles };
    const got = projectShadowReceivers(args);
    assert.equal(got.coveredArea, 8);
    assert.ok(
      got.uvs.every((n, i) => n >= (i % 2 ? 0.2 : 0.2) - 1e-8 && n <= (i % 2 ? 0.4 : 0.6) + 1e-8),
    );
    assert.throws(
      () => projectShadowReceivers({ ...args, coverage: { ...coverage, left: -1 } }),
      /escapes/,
    );
    assert.throws(() => projectShadowReceivers({ ...args, coverage: undefined }), /Incomplete/);
  });
}

{
  test("raised evaluated terrain follows camera rays instead of treating world Y as map Y", () => {
    const rows = [
      {
        id: "a",
        points: [
          [0, 10, 10],
          [10, 10, 10],
          [0, 20, 10],
        ],
      },
      {
        id: "b",
        points: [
          [10, 10, 10],
          [10, 20, 10],
          [0, 20, 10],
        ],
      },
    ];
    const selected = projectedTerrainReceivers(rows, [0, 0, 10, 10]);
    assert.deepEqual(
      selected.map((r) => r.points),
      [
        [
          [0, 0, 10],
          [10, 0, 10],
          [0, 10, 10],
        ],
        [
          [10, 0, 10],
          [10, 10, 10],
          [0, 10, 10],
        ],
      ],
    );
    const result = projectShadowReceivers({
      anchor: [0, 0, 0],
      bounds: { left: 0, top: 0, width: 10, height: 10 },
      elevation: Math.PI / 4,
      triangles: selected.map((r) => r.points),
    });
    assert.equal(result.coveredArea, 100);
    assert.equal(result.ranges.length, 2);
    assert.ok(
      result.positions
        .filter((_, i) => i % 3 === 1)
        .every((y) => Math.abs(y - (10 / Math.cos(Math.PI / 4) + 0.15)) < 1e-9),
    );
  });
  test("overlapping evaluated surfaces remain explicit and are rejected instead of silently flattened", () => {
    const rows = [
      {
        id: "a",
        points: [
          [0, 0, 0],
          [10, 0, 0],
          [0, 10, 0],
        ],
      },
      {
        id: "b",
        points: [
          [0, 4, 4],
          [10, 4, 4],
          [0, 14, 4],
        ],
      },
    ];
    const selected = projectedTerrainReceivers(rows, [0, 0, 10, 10]);
    assert.equal(selected.length, 2);
    assert.throws(
      () =>
        projectShadowReceivers({
          anchor: [0, 0, 0],
          bounds: { left: 0, top: 0, width: 10, height: 10 },
          elevation: Math.PI / 4,
          triangles: selected.map((r) => r.points),
        }),
      /Overlapping/,
    );
    assert.throws(() => projectedTerrainReceivers([rows[0], rows[0]], [0, 0, 10, 10]), /duplicate/);
  });
}

{
  const mesh = (points) => {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.Float32BufferAttribute(points.flat(), 3));
    return new THREE.Mesh(g);
  };
  test("evaluated top triangles retain parent placement and slope but reject nearly vertical sides", () => {
    const child = mesh([
        [0, 0, 0],
        [0, 2, 2],
        [2, 0, 0],
        [0, 0, 0],
        [0, 2, 0],
        [0, 2, 2],
      ]),
      parent = new THREE.Group();
    parent.position.set(10, 20, 30);
    parent.rotation.z = 1e-8;
    parent.add(child);
    const rows = evaluatePhysicalReceiver(child, "bank-ramp", Math.PI / 6);
    assert.equal(rows.length, 1);
    assert.equal(rows[0].id, "bank-ramp:triangle-0");
    assert.ok(Math.abs(rows[0].points[0][0] - 10) < 1e-8);
    assert.ok(Math.abs(rows[0].points[0][1] - 15) < 1e-8);
    assert.ok(Math.abs(rows[0].points[0][2] - 20 * Math.cos(Math.PI / 6)) < 1e-8);
    assert.ok(rows[0].points[1][2] > rows[0].points[0][2]);
  });
  test("unsupported dynamic meshes and missing top support fail explicitly", () => {
    const vertical = mesh([
      [0, 0, 0],
      [0, 2, 0],
      [0, 2, 2],
    ]);
    assert.throws(() => evaluatePhysicalReceiver(vertical, "side", Math.PI / 6), /no upward/);
    vertical.isSkinnedMesh = true;
    assert.throws(() => evaluatePhysicalReceiver(vertical, "side", Math.PI / 6), /static/);
  });
}

{
  test("all nonzero shadow alpha, including distant faint pixels, bounds exact source texel edges", () => {
    const p = new Uint8ClampedArray(6 * 8 * 4),
      b = { left: -3, top: 5, width: 6, height: 8 };
    p[(2 * 6 + 1) * 4 + 3] = 255;
    p[(7 * 6 + 4) * 4 + 3] = 1;
    assert.deepEqual(shadowCoverageBounds(p, b), { left: -2, top: 3, width: 4, height: 6 });
    assert.equal(shadowCoverageBounds(new Uint8Array(p.length), b), null);
    assert.throws(() => shadowCoverageBounds(p.slice(1), b), /Invalid/);
  });
}

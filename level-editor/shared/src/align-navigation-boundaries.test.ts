import test from "node:test";
import assert from "node:assert/strict";
import clipping from "polygon-clipping";
import { alignNavigationBoundaries } from "./align-navigation-boundaries.ts";
import { assembleNavigationJoins, type PlacedNavigationJoin } from "./assemble-navigation-joins.ts";
import type { Vec3 } from "./scene.ts";

test("a partial socket splits a long boundary into the same vertices at every placement", () => {
  for (const angle of [0, 37, 90, 180]) {
    const radians = (angle * Math.PI) / 180;
    const place = (x: number, y: number): [number, number] => [
      429.320707476824 + x * Math.cos(radians) - y * Math.sin(radians),
      1617.9584358822462 + x * Math.sin(radians) + y * Math.cos(radians),
    ];
    const edge = (x: number): Vec3 => [...place(x, 0), 0];
    const surfaces = [
      {
        navigationRegion: "a",
        polygon: [place(-5, -10), place(35, -10), place(35, 0), place(-5, 0)],
      },
      { navigationRegion: "b", polygon: [place(0, 0), place(24, 0), place(24, 10), place(0, 10)] },
      {
        navigationRegion: "unconnected",
        polygon: [place(0, 20), place(24, 20), place(24, 30), place(0, 30)],
      },
    ];
    const untouched = structuredClone(surfaces[2]);
    const joins: PlacedNavigationJoin[] = [
      { owner: "a", region: "a", edge: [edge(24), edge(0)] },
      { owner: "b", region: "b", edge: [edge(0), edge(24)] },
    ];
    alignNavigationBoundaries(surfaces, assembleNavigationJoins(joins).connections);
    assert.equal(surfaces[0]!.polygon.length, 6);
    assert.deepEqual(surfaces[2], untouched);
    const polygons = surfaces.map(({ polygon }) => [[...polygon, polygon[0]!]]);
    assert.equal(clipping.union(polygons[0]!, ...polygons.slice(1)).length, 2);
    for (const point of [place(0, 0), place(24, 0)]) {
      const from = surfaces[0]!.polygon.find(
        (p) => Math.hypot(p[0] - point[0], p[1] - point[1]) < 1e-4,
      );
      const to = surfaces[1]!.polygon.find(
        (p) => Math.hypot(p[0] - point[0], p[1] - point[1]) < 1e-4,
      );
      assert.deepEqual(from, to);
    }
  }
});

test("unmatched nearby boundaries are unchanged", () => {
  const surfaces = [
    {
      navigationRegion: "a",
      polygon: [
        [0, 0],
        [10, 0],
        [10, 10],
      ] as [number, number][],
    },
  ];
  const original = structuredClone(surfaces);
  alignNavigationBoundaries(surfaces, []);
  assert.deepEqual(surfaces, original);
});

// Measure footprint support across joined landings without changing exported geometry.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createRequire } from "node:module";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { placeGameplaySurface } from "../shared/src/place-gameplay-surface.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { compileNavigationGraph } from "../shared/src/compile-navigation-graph.ts";
import { compileLiftApproaches } from "../shared/src/compile-lift-approaches.ts";
import { compileLiftPassageStates } from "../shared/src/compile-passage-states.ts";
import { prepareStairPassages } from "../shared/src/prepare-stair-passages.ts";

const clipping = createRequire(new URL("../shared/package.json", import.meta.url))(
  "polygon-clipping",
);
const [directory, stage] = process.argv.slice(2);
assert.ok(directory && stage, "Usage: check-stair-landing-support.mjs EXPORTS STAGE");
const output = process.argv.includes("--prepare")
  ? await fs.mkdtemp("work/map-compile/joined-stair-support-diagnostic-")
  : undefined;
const manifest = JSON.parse(await fs.readFile(`${directory}/diagnostics.json`, "utf8"));
assert.equal(manifest.complete, true);
const review = JSON.parse(await fs.readFile(`${stage}/review.json`, "utf8"));
const surfaceVersion = process.argv.includes("--authored") ? "before" : "after";
const surface = review[surfaceVersion];
assert.ok(surface?.polygon && surface?.node);
if (surfaceVersion === "after") assert.equal(manifest.gameplayStage, stage);
else assert.equal(manifest.gameplayStage, undefined);
const area = (regions) =>
  regions.reduce(
    (total, polygon) =>
      total +
      polygon.reduce((sum, ring, index) => {
        const signed = ring.reduce((value, p, i) => {
          const q = ring[(i + 1) % ring.length];
          return value + p[0] * q[1] - p[1] * q[0];
        }, 0);
        return sum + (index === 0 ? 1 : -1) * Math.abs(signed / 2);
      }, 0),
    0,
  );
const results = [];
const corners = [
  [-1, -1],
  [1, -1],
  [1, 1],
  [-1, 1],
];
const hull = (points) => {
  const sorted = points.toSorted((a, b) => a[0] - b[0] || a[1] - b[1]);
  const turn = (a, b, c) => (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
  const chain = (points) => {
    const result = [];
    for (const point of points) {
      while (result.length > 1 && turn(result.at(-2), result.at(-1), point) <= 0) result.pop();
      result.push(point);
    }
    return result.slice(0, -1);
  };
  return [...chain(sorted), ...chain(sorted.toReversed())];
};
for (const { file } of manifest.results) {
  const descriptor = JSON.parse(await fs.readFile(`${directory}/${file}`, "utf8"));
  const document = JSON.parse(await fs.readFile(`${directory}/${file}.scene.json`, "utf8"));
  const source = document.assetSources.find((entry) => entry.id === review.source.id);
  assert.equal(source?.descriptor_sha256, review.source.descriptor_sha256);
  assert.equal(source?.model_sha256, review.source.model_sha256);
  const geometry = descriptor.asset_geometry;
  const stairs = geometry.lifts.filter((lift) => lift.lift_type === 1);
  assert.equal(stairs.length, 1, "Diagnostic expects one isolated stair per placement");
  const lift = stairs[0];
  const object = document.objects.find((part) => part.node.endsWith(`:${surface.node}`));
  assert.ok(object);
  const matrix = partMatrix(document.camera, document, object);
  const placed = placeGameplaySurface(surface, (_node, point) => {
    const local = gameToScene(document.camera, ...point);
    const world = [0, 1, 2].map(
      (row) =>
        matrix[row] * local[0] +
        matrix[4 + row] * local[1] +
        matrix[8 + row] * local[2] +
        matrix[12 + row],
    );
    return sceneToGame(document.camera, world);
  });
  const floor = [
    placed.navigationPoints.map(([x, y]) => [x, y]),
    ...placed.navigationHoles.map((hole) => hole.map(([x, y]) => [x, y])),
  ];
  const receivers = geometry.sight_obstacles.filter((obstacle) =>
    lift.doors.some(
      (door) =>
        obstacle.projection_area?.[0] === door.sector_out &&
        obstacle.projection_area?.[1] === door.layer_out,
    ),
  );
  assert.equal(receivers.length, 2, "Diagnostic expects one receiver at each landing");
  const joined = clipping.union(
    floor,
    ...receivers.map((receiver) => [receiver.points.map(({ x, y }) => [x, y])]),
  );
  const [a, b, c] = placed.worldPlane;
  assert.ok(Math.abs(1 - b) > 1e-6);
  const projected = placed.navigationPoints.map(([x, y, z]) => [x, y - z]);
  const xs = projected.map(([x]) => x),
    ys = projected.map(([, y]) => y);
  const counts = [];
  let preparedBoundary;
  for (const half of [
    [6, 3],
    [5, 2],
  ]) {
    let stairOnly = 0;
    const candidates = [];
    for (let x = Math.ceil(Math.min(...xs)); x <= Math.max(...xs); x++)
      for (let y = Math.ceil(Math.min(...ys)); y <= Math.max(...ys); y++) {
        const center = [x, (y + a * x + c) / (1 - b)];
        if (!pointInGameplayPolygon(center, floor[0])) continue;
        const box = [
          [-1, -1],
          [1, -1],
          [1, 1],
          [-1, 1],
        ].map(([dx, dy]) => [center[0] + dx * half[0], center[1] + dy * half[1]]);
        if (area(clipping.difference([box], floor)) < 1e-8) stairOnly++;
        if (area(clipping.difference([box], joined)) < 1e-8) candidates.push([x, y]);
      }
    const nearPassage = (point) => {
      const [first, second] = lift.doors.map((door) => door.point_in);
      const dx = second[0] - first[0],
        dy = second[1] - first[1];
      const length = dx * dx + dy * dy;
      const t =
        length === 0
          ? 0
          : Math.max(
              0,
              Math.min(1, ((point[0] - first[0]) * dx + (point[1] - first[1]) * dy) / length),
            );
      return Math.hypot(point[0] - first[0] - t * dx, point[1] - first[1] - t * dy) <= 2;
    };
    const centers = hull(candidates.filter(nearPassage));
    const worldCenters = centers.map(([x, y]) => [x, (y + a * x + c) / (1 - b)]);
    const footprintHull = hull(
      worldCenters.flatMap(([x, y]) =>
        corners.map(([dx, dy]) => [x + dx * half[0], y + dy * half[1]]),
      ),
    );
    const hullSupported =
      centers.length >= 3 &&
      area(clipping.difference([worldCenters], floor)) < 1e-8 &&
      area(clipping.difference([footprintHull], joined)) < 1e-8;
    // Diagnostic only: preserve centers inside the stair while accounting for
    // footprint support on both landings. Height and collision review is separate.
    if (output && half[0] === 5 && hullSupported)
      preparedBoundary = hull(
        centers.flatMap(([x, y]) => corners.map(([dx, dy]) => [x + dx * 4.99, y + dy * 1.99])),
      );
    counts.push({
      half,
      stairOnly,
      joined: candidates.length,
      hullSupported,
      missingLandingUnsupportedArea: receivers.map((removed) => {
        const remaining = clipping.union(
          floor,
          ...receivers
            .filter((receiver) => receiver !== removed)
            .map((receiver) => [receiver.points.map(({ x, y }) => [x, y])]),
        );
        return {
          receiver: removed.projection_area,
          area: area(clipping.difference([footprintHull], remaining)),
        };
      }),
      nearestApproaches: lift.doors.map((door) => {
        const ranked = candidates
          .map((point) => ({
            point,
            distance: Math.hypot(point[0] - door.point_in[0], point[1] - door.point_in[1]),
          }))
          .sort((a, b) => a.distance - b.distance);
        return { authored: door.point_in, nearest: ranked[0] ?? null };
      }),
    });
  }
  if (output) {
    assert.ok(preparedBoundary, `${file}: sampled hull cannot safely represent joined support`);
    let sector = 0;
    let motion;
    for (const layer of geometry.motion_data.layers)
      for (const entry of layer) {
        if (sector === lift.motion_area_index) motion = entry;
        sector += 1 + entry.obstacles.length;
      }
    assert.ok(motion?.is_lift);
    assert.equal(motion.obstacles.length, 0, "Diagnostic cannot replace collision-bearing floors");
    motion.precise_polygon = preparedBoundary;
    motion.polygon.points = preparedBoundary.map(([x, y]) => [Math.round(x), Math.round(y)]);
    geometry.motion_data.graph_bytes = compileNavigationGraph(geometry.motion_data.layers);
    compileLiftApproaches(geometry);
    prepareStairPassages(geometry);
    const states = compileLiftPassageStates(geometry);
    geometry.lifts.forEach((entry, i) =>
      entry.doors.forEach((door, j) => {
        door.passage_states = states[i][j];
      }),
    );
    await fs.writeFile(`${output}/${file}`, JSON.stringify(descriptor));
  }
  results.push({ file, counts });
}
const report = {
  scope: "joined-footprint-support-only-not-height-collision-or-traversal-proof",
  directory,
  stage,
  surfaceVersion,
  output,
  results,
};
if (output)
  await fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      ...manifest,
      source: directory,
      diagnostic: report.scope,
    }),
  );
await fs.writeFile(
  `${directory}/stair-landing-support-report.json`,
  JSON.stringify(report, null, 2),
);
console.log(JSON.stringify(report));

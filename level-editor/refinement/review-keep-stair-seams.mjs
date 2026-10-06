import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh, maskRecoveryTextures } from "../pipeline/src/mask-recovery-mesh.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { gameTransformMatrix } from "../shared/src/level3d.ts";

const [directory] = process.argv.slice(2);
assert.ok(directory, "Provide staged component edits directory");
const require = createRequire(new URL("../pipeline/package.json", import.meta.url));
const sharp = require("sharp");
const edits = JSON.parse(await fs.readFile(`${directory}/edits.json`, "utf8"));
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const report = [];
// Compare smooth navigation ramps with the actual upper mesh, including treads.
function contains(point, polygon) {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[i],
      b = polygon[j];
    if (
      a[1] > point[1] !== b[1] > point[1] &&
      point[0] < ((b[0] - a[0]) * (point[1] - a[1])) / (b[1] - a[1]) + a[0]
    )
      inside = !inside;
  }
  return inside;
}
function meshHeight(point, triangle) {
  const [a, b, c] = triangle;
  const denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(denominator) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / denominator;
  const v = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / denominator;
  if (u < -1e-8 || v < -1e-8 || u + v > 1 + 1e-8) return undefined;
  return u * a[2] + v * b[2] + (1 - u - v) * c[2];
}
function edgeDistance(point, a, b) {
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const t = Math.max(
    0,
    Math.min(1, ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)),
  );
  return Math.hypot(point[0] - a[0] - t * dx, point[1] - a[1] - t * dy);
}
for (const edit of edits) {
  const entry = index.find((entry) => entry.id === edit.asset);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
  const descriptor = JSON.parse(bytes);
  const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
  const modelSha256 = createHash("sha256").update(modelBytes).digest("hex");
  const model = await loadSceneModel("library", {
    id: edit.asset,
    role: "objects",
    model: `3d-assets/${entry.model}`,
    model_sha256: modelSha256,
    descriptor: `3d-assets/${entry.descriptor}`,
    descriptor_sha256: edit.descriptorSha256,
    resources: descriptor.resources ?? [],
    ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
  });
  const textures = await maskRecoveryTextures(model);
  for (const lift of edit.gameplay.lifts) {
    const before = descriptor.gameplay.surfaces.find((surface) => surface.id === lift.surface);
    const after = edit.gameplay.surfaces.find((surface) => surface.id === lift.surface);
    const plane = heightPlane(
      after.polygon.map(([x, y], i) => [
        x,
        y,
        typeof after.height === "number" ? after.height : after.height[i],
      ]),
    );
    const triangles = maskRecoveryMesh(
      model,
      lift.node,
      (p) => sceneToGame(camera, gltfToScene(p)),
      textures,
    );
    const floorTriangles = triangles.filter((triangle) =>
      triangle.every(([x, y, z]) => Math.abs(z - planeHeight(plane, [x, y])) < 2),
    );
    // Inspect the assembled asset too: an adjacent landing can cover the end
    // of the visible flight, so a stair-only height sample is insufficient.
    const neighbours = descriptor.parts
      .filter((part) => part.node !== lift.node)
      .map((part) => ({
        node: part.node,
        initiallyVisible: !part.default_hidden,
        triangles: maskRecoveryMesh(
          model,
          part.node,
          (p) => sceneToGame(camera, gltfToScene(p)),
          textures,
        ),
      }));
    const visibleAssembly = [
      {
        node: lift.node,
        initiallyVisible: !descriptor.parts.find((p) => p.node === lift.node)?.default_hidden,
        triangles,
      },
      ...neighbours,
    ].filter((part) => part.initiallyVisible);
    const projectedAssembly = visibleAssembly.map(({ node, triangles }) => ({
      node,
      triangles: triangles.map((triangle) => triangle.map(([x, y, z]) => [x, y - z, z])),
    }));
    const occluders = ([x, y], height) =>
      projectedAssembly.flatMap(({ node, triangles }) => {
        const hits = triangles
          .map((triangle) => meshHeight([x, y - height], triangle))
          .filter((z) => z !== undefined && z > height + 0.1);
        return hits.length ? [{ node, height: Math.max(...hits) }] : [];
      });
    const treadProfiles = triangles
      .filter(
        (triangle) =>
          Math.max(...triangle.map((p) => p[2])) - Math.min(...triangle.map((p) => p[2])) < 0.01,
      )
      .map((triangle) => {
        const center = [0, 1, 2].map((axis) => triangle.reduce((sum, p) => sum + p[axis], 0) / 3);
        const covering = neighbours.flatMap(({ node, triangles }) => {
          const hits = triangles
            .map((triangle) => meshHeight(center, triangle))
            .filter((z) => z !== undefined && z > center[2] + 0.1);
          return hits.length ? [{ node, height: Math.max(...hits) }] : [];
        });
        const coveredByFlight = triangles.some((triangle) => {
          const z = meshHeight(center, triangle);
          return z !== undefined && z > center[2] + 0.1;
        });
        return {
          center,
          navigationHeight: planeHeight(plane, center),
          insideNavigation: contains(center, after.polygon),
          coveredByFlight,
          covering,
        };
      });
    const landingEdgeReviews = [];
    const approachReviews = lift.doors.flatMap((door) => {
      const oldDoor = descriptor.gameplay.lifts
        .find((item) => item.id === lift.id)
        ?.doors.find((item) => item.id === door.id);
      if (!oldDoor || oldDoor.outside.every((value, i) => value === door.outside[i])) return [];
      const samples = Array.from({ length: 41 }, (_, i) => {
        const point = door.middle.map((v, axis) => v + ((door.outside[axis] - v) * i) / 40);
        const hits = [triangles, ...neighbours.map((part) => part.triangles)]
          .flat()
          .map((triangle) => meshHeight(point, triangle))
          .filter((z) => z !== undefined);
        return {
          point,
          nearestMeshHeightError: hits.length
            ? Math.min(...hits.map((z) => Math.abs(z - point[2])))
            : null,
        };
      });
      return [
        {
          door: door.id,
          before: oldDoor.outside,
          after: door.outside,
          supported: samples.filter(
            (sample) =>
              sample.nearestMeshHeightError !== null && sample.nearestMeshHeightError < 0.1,
          ).length,
          samples,
        },
      ];
    });
    for (const landing of edit.gameplay.surfaces) {
      const previous = descriptor.gameplay.surfaces.find((surface) => surface.id === landing.id);
      const heights = Array.isArray(landing.height) ? landing.height : [landing.height];
      if (!previous || !heights.every((z) => Math.abs(z - heights[0]) < 1e-6)) continue;
      const mesh = neighbours.find((part) => part.node === landing.node)?.triangles;
      if (!mesh) continue;
      const levelTriangles = [triangles, ...neighbours.map((part) => part.triangles)]
        .flat()
        .filter((triangle) => triangle.every((p) => Math.abs(p[2] - heights[0]) < 0.1));
      for (let i = 0; i < landing.polygon.length; i++) {
        const j = (i + 1) % landing.polygon.length;
        const oldEdges = previous.polygon.map((point, index) => [
          point,
          previous.polygon[(index + 1) % previous.polygon.length],
        ]);
        if (
          oldEdges.some(([a, b]) =>
            [landing.polygon[i], landing.polygon[j]].every((p) => edgeDistance(p, a, b) < 1e-6),
          )
        )
          continue;
        const midpoint = landing.polygon[i].map((v, axis) => (v + landing.polygon[j][axis]) / 2);
        const previousEdge = oldEdges.reduce((best, edge) =>
          edgeDistance(midpoint, ...edge) < edgeDistance(midpoint, ...best) ? edge : best,
        );
        const samples = Array.from({ length: 41 }, (_, step) => {
          const point = landing.polygon[i].map(
            (v, axis) => v + ((landing.polygon[j][axis] - v) * step) / 40,
          );
          const hits = mesh
            .map((triangle) => meshHeight(point, triangle))
            .filter((z) => z !== undefined);
          const assemblyHits = [{ node: lift.node, triangles }, ...neighbours].flatMap((part) => {
            const heights = part.triangles
              .map((triangle) => meshHeight(point, triangle))
              .filter((z) => z !== undefined);
            return heights.length ? [{ node: part.node, heights }] : [];
          });
          const supported = assemblyHits.some((part) =>
            part.heights.some((z) => Math.abs(z - heights[0]) < 0.1),
          );
          const uncoveredDistance = supported
            ? 0
            : levelTriangles.length
              ? Math.min(
                  ...levelTriangles.flatMap((triangle) =>
                    triangle.map((a, k) => edgeDistance(point, a, triangle[(k + 1) % 3])),
                  ),
                )
              : null;
          return { point, hits, assemblyHits, uncoveredDistance };
        });
        landingEdgeReviews.push({
          surface: landing.id,
          node: landing.node,
          height: heights[0],
          before: previousEdge,
          after: [landing.polygon[i], landing.polygon[j]],
          maximumUncoveredDistance: samples.some((sample) => sample.uncoveredDistance === null)
            ? null
            : Math.max(...samples.map((sample) => sample.uncoveredDistance)),
          supported: samples.filter((sample) =>
            sample.hits.some((z) => Math.abs(z - heights[0]) < 0.1),
          ).length,
          assemblySupported: samples.filter((sample) =>
            sample.assemblyHits.some((part) =>
              part.heights.some((z) => Math.abs(z - heights[0]) < 0.1),
            ),
          ).length,
          samples,
        });
      }
    }
    const samples = [];
    const bounds = [0, 1].map((axis) => [
      Math.min(...after.polygon.map((p) => p[axis])),
      Math.max(...after.polygon.map((p) => p[axis])),
    ]);
    for (let ix = 0; ix < 40; ix++)
      for (let iy = 0; iy < 40; iy++) {
        const point = [
          bounds[0][0] + ((ix + 0.5) / 40) * (bounds[0][1] - bounds[0][0]),
          bounds[1][0] + ((iy + 0.5) / 40) * (bounds[1][1] - bounds[1][0]),
        ];
        if (!contains(point, after.polygon)) continue;
        const hits = triangles
          .map((triangle) => meshHeight(point, triangle))
          .filter((z) => z !== undefined);
        const floor = planeHeight(plane, point);
        samples.push({
          point,
          floor,
          mesh: hits.length ? Math.max(...hits) : null,
          ...(hits.length
            ? {}
            : {
                initialFootOccluders: occluders(point, floor),
                initialHeadOccluders: occluders(point, floor + 80),
              }),
        });
      }
    const residuals = samples
      .filter((sample) => sample.mesh !== null)
      .map((sample) => sample.mesh - sample.floor)
      .sort((a, b) => a - b);
    const uncoveredDistances = samples
      .filter((sample) => sample.mesh === null)
      .map((sample) =>
        Math.min(
          ...triangles.flatMap((triangle) =>
            triangle.map((a, i) => edgeDistance(sample.point, a, triangle[(i + 1) % 3])),
          ),
        ),
      );
    const rotatedUncoveredFloorVisibility = [0, 37, 90, 180].map((rotation) => {
      const matrix = gameTransformMatrix(
        camera,
        { dx: 0, dy: 0, dz: 0, rot_deg: rotation },
        [0, 0],
      );
      const rotate = (point) => {
        const [x, y, z] = gameToScene(camera, ...point);
        return sceneToGame(camera, [
          matrix[0] * x + matrix[4] * y + matrix[8] * z + matrix[12],
          matrix[1] * x + matrix[5] * y + matrix[9] * z + matrix[13],
          matrix[2] * x + matrix[6] * y + matrix[10] * z + matrix[14],
        ]);
      };
      const geometry = visibleAssembly.flatMap(({ triangles }) =>
        triangles.map((triangle) =>
          triangle.map((point) => {
            const [x, y, z] = rotate(point);
            return [x, y - z, z];
          }),
        ),
      );
      const uncovered = samples.filter((sample) => sample.mesh === null);
      const visible = (sample, offset) => {
        const [x, y, z] = rotate([...sample.point, sample.floor + offset]);
        return !geometry.some((triangle) => {
          const hit = meshHeight([x, y - z], triangle);
          return hit !== undefined && hit > z + 0.1;
        });
      };
      return {
        rotation,
        samples: uncovered.length,
        visibleFeet: uncovered.filter((s) => visible(s, 0)).length,
        visibleHeads: uncovered.filter((s) => visible(s, 80)).length,
      };
    });
    const treadHeights = [
      ...new Set(
        triangles
          .filter(
            (triangle) =>
              Math.max(...triangle.map((p) => p[2])) - Math.min(...triangle.map((p) => p[2])) <
              0.01,
          )
          .map((triangle) => Math.round(triangle[0][2] * 100) / 100),
      ),
    ].sort((a, b) => a - b);
    const points = [
      ...before.polygon,
      ...after.polygon,
      ...floorTriangles.flat().map(([x, y]) => [x, y]),
    ];
    const x0 = Math.min(...points.map((p) => p[0])) - 4,
      x1 = Math.max(...points.map((p) => p[0])) + 4;
    const y0 = Math.min(...points.map((p) => p[1])) - 4,
      y1 = Math.max(...points.map((p) => p[1])) + 4;
    const scale = Math.min(700 / (x1 - x0), 440 / (y1 - y0));
    const xy = (points) =>
      points.map(([x, y]) => `${40 + (x - x0) * scale},${65 + (y - y0) * scale}`).join(" ");
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="570">
      <rect width="800" height="570" fill="#171d25"/>
      <text x="25" y="28" fill="white" font-family="sans-serif" font-size="18">${edit.asset} / ${lift.node}</text>
      ${triangles.map((triangle) => `<polygon points="${xy(triangle)}" fill="#566171" fill-opacity="0.08" stroke="#718096" stroke-opacity="0.3" stroke-width="0.5"/>`).join("")}
      ${floorTriangles.map((triangle) => `<polygon points="${xy(triangle)}" fill="#566171" fill-opacity="0.3" stroke="#718096" stroke-width="0.7"/>`).join("")}
      <polygon points="${xy(before.polygon)}" fill="none" stroke="#ffb347" stroke-width="2"/>
      <polygon points="${xy(after.polygon)}" fill="none" stroke="#37d7df" stroke-width="2"/>
      ${lift.doors.map((door) => `<circle cx="${40 + (door.middle[0] - x0) * scale}" cy="${65 + (door.middle[1] - y0) * scale}" r="4" fill="#f36a94"/>`).join("")}
      <text x="25" y="535" fill="white" font-family="sans-serif" font-size="15">Gray: full mesh (bright: near floor). Orange: published floor. Cyan: candidate.</text>
      <text x="25" y="557" fill="white" font-family="sans-serif" font-size="15">Pink: corrected door midpoints. Top view in asset-local game coordinates.</text></svg>`;
    const file = `${directory}/${lift.node}-seam-review`;
    await fs.writeFile(`${file}.svg`, svg);
    await sharp(Buffer.from(svg)).png().toFile(`${file}.png`);
    report.push({
      asset: edit.asset,
      node: lift.node,
      modelSha256,
      meshTriangles: triangles.length,
      sampledFloorPoints: samples.length,
      sampledMeshHits: residuals.length,
      maximumUncoveredMeshEdgeDistance: Math.max(0, ...uncoveredDistances),
      initialUncoveredFloorVisibility: {
        scope:
          "Sampled floor/head rays through initially visible asset parts; not full sprite or alternate-state verification",
        samples: uncoveredDistances.length,
        occludedFeet: samples.filter((s) => s.initialFootOccluders?.length).length,
        occludedHeads: samples.filter((s) => s.initialHeadOccluders?.length).length,
      },
      rotatedUncoveredFloorVisibility,
      treadHeights,
      meshMinusFloorQuantiles: [0, 0.1, 0.5, 0.9, 1].map(
        (q) => residuals[Math.round(q * (residuals.length - 1))],
      ),
      sampleFile: `${file}-samples.json`,
      nearFloorTriangles: floorTriangles.length,
      treadProfiles,
      landingEdgeReviews,
      approachReviews,
      maximumFloorXYShift:
        before.polygon.length === after.polygon.length
          ? Math.max(
              ...before.polygon.map((p, i) =>
                Math.hypot(p[0] - after.polygon[i][0], p[1] - after.polygon[i][1]),
              ),
            )
          : null,
      maximumFloorBoundaryVertexDistance: Math.max(
        ...[
          [before.polygon, after.polygon],
          [after.polygon, before.polygon],
        ].flatMap(([from, to]) =>
          from.map((p) =>
            Math.min(...to.map((q, i) => edgeDistance(p, q, to[(i + 1) % to.length]))),
          ),
        ),
      ),
      plane,
      images: [`${file}.svg`, `${file}.png`],
    });
    await fs.writeFile(`${file}-samples.json`, JSON.stringify(samples, null, 2));
  }
}
await fs.writeFile(`${directory}/mesh-review.json`, JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));

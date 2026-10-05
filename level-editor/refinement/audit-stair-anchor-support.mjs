import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

// Read-only local-geometry audit. No source map or inferred ground is consulted.
const [library = "library"] = process.argv.slice(2);
const index = JSON.parse(await fs.readFile(`${library}/3d-assets/index.json`, "utf8")).assets;
const results = [];
function distance(point, polygon) {
  return Math.min(
    ...polygon.map((a, i) => {
      const b = polygon[(i + 1) % polygon.length];
      const dx = b[0] - a[0],
        dy = b[1] - a[1];
      const t = Math.max(
        0,
        Math.min(1, ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / (dx * dx + dy * dy || 1)),
      );
      return Math.hypot(point[0] - a[0] - t * dx, point[1] - a[1] - t * dy);
    }),
  );
}
for (const entry of index) {
  const bytes = await fs.readFile(`${library}/3d-assets/${entry.descriptor}`);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), entry.descriptor_sha256, entry.id);
  const { gameplay } = JSON.parse(bytes);
  for (const lift of gameplay?.lifts ?? []) {
    if (lift.type !== 1) continue;
    const surface = gameplay.surfaces.find((surface) => surface.id === lift.surface);
    assert.ok(surface, `${entry.id}/${lift.surface}`);
    const vertices = surface.polygon.map(([x, y], i) => [
      x,
      y,
      Array.isArray(surface.height) ? surface.height[i] : surface.height,
    ]);
    let plane;
    try {
      plane = heightPlane(vertices);
    } catch (error) {
      results.push({ asset: entry.id, lift: lift.id, error: String(error) });
      continue;
    }
    const doors = lift.doors.map((door) => ({
      id: door.id,
      anchors: ["inside", "middle"].map((kind) => {
        const point = door[kind];
        return {
          kind,
          point,
          supported:
            pointInGameplayPolygon(point, surface.polygon, true) &&
            !(surface.holes ?? []).some((hole) => pointInGameplayPolygon(point, hole, true)),
          edgeDistance: distance(point, surface.polygon),
          heightError: point[2] - planeHeight(plane, point),
        };
      }),
      landingHeightDifference: door.middle[2] - door.outside[2],
      localLandingCandidates: gameplay.surfaces
        .filter(
          (candidate) =>
            candidate !== surface &&
            (Array.isArray(candidate.height) ? candidate.height : [candidate.height]).every(
              (z) => Math.abs(z - door.outside[2]) < 1e-4,
            ) &&
            pointInGameplayPolygon(door.outside, candidate.polygon, true) &&
            !(candidate.holes ?? []).some((hole) =>
              pointInGameplayPolygon(door.outside, hole, true),
            ),
        )
        .map((candidate) => candidate.id),
    }));
    results.push({
      asset: entry.id,
      descriptorSha256: entry.descriptor_sha256,
      lift: lift.id,
      surface: surface.id,
      maximumPlaneResidual: Math.max(
        ...vertices.map((p) => Math.abs(p[2] - planeHeight(plane, p))),
      ),
      doors,
    });
  }
}
const output = await fs.mkdtemp("work/map-compile/stair-anchor-support-");
const unsupported = results.filter(
  (result) =>
    result.error ||
    result.doors.some((door) =>
      door.anchors.some((anchor) => !anchor.supported || Math.abs(anchor.heightError) > 1e-4),
    ),
);
await fs.writeFile(
  `${output}/report.json`,
  JSON.stringify({ scope: "asset-local-anchor-support-not-placed-connectivity", results }, null, 2),
);
console.log(
  JSON.stringify(
    {
      output,
      stairs: results.length,
      unsupported: unsupported.length,
      assets: unsupported.map((result) => ({
        asset: result.asset,
        lift: result.lift,
        error: result.error,
        unsupportedAnchors: result.doors?.flatMap((door) =>
          door.anchors
            .filter((anchor) => !anchor.supported || Math.abs(anchor.heightError) > 1e-4)
            .map((anchor) => ({
              door: door.id,
              kind: anchor.kind,
              edgeDistance: anchor.edgeDistance,
              heightError: anchor.heightError,
            })),
        ),
      })),
    },
    null,
    2,
  ),
);

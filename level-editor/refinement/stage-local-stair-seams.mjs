import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Explicitly selected authoring candidates only. Publication requires mesh and
// placed actor checks; this never repairs a scene silently during compilation.
const [asset, ...arguments_] = process.argv.slice(2);
const localLandingEdges = arguments_.includes("--local-landing-edges");
const climbSeams = arguments_.includes("--climb-seams");
const slopedLandings = arguments_.includes("--sloped-landings");
const outsideExtensions = new Map(
  arguments_
    .filter((value) => value.startsWith("--outside-extension="))
    .map((value) => {
      const [door, distance] = value.slice(20).split("=");
      assert.ok(
        door && Number.isFinite(Number(distance)) && Number(distance) > 0,
        "Invalid outside extension",
      );
      return [door, Number(distance)];
    }),
);
const usedExtensions = new Set();
const groundHeights = arguments_.filter((value) => value.startsWith("--placement-ground-height="));
assert.ok(groundHeights.length <= 1, "Provide at most one placement ground height");
const placementGroundHeight = groundHeights.length
  ? Number(groundHeights[0].split("=")[1])
  : undefined;
assert.ok(
  placementGroundHeight === undefined || Number.isFinite(placementGroundHeight),
  "Invalid placement ground height",
);
const midpointLimits = arguments_.filter((value) => value.startsWith("--midpoint-shift-limit="));
assert.ok(midpointLimits.length <= 1, "Provide at most one midpoint shift limit");
const midpointShiftLimit = midpointLimits.length ? Number(midpointLimits[0].split("=")[1]) : 3;
assert.ok(
  Number.isFinite(midpointShiftLimit) && midpointShiftLimit > 0,
  "Invalid midpoint shift limit",
);
const draftIssues = arguments_
  .filter((value) => value.startsWith("--draft-issue="))
  .map((value) => value.slice(14));
const resolvedIssues = arguments_
  .filter((value) => value.startsWith("--resolve-draft-issue="))
  .map((value) => value.slice(22));
assert.ok(
  draftIssues.every((issue) => issue.trim().length > 0),
  "Draft issues cannot be empty",
);
const external = new Set(
  arguments_.filter((value) => value.startsWith("--external=")).map((value) => value.slice(11)),
);
const floorLimits = arguments_.filter((value) => value.startsWith("--floor-shift-limit="));
assert.ok(floorLimits.length <= 1, "Provide at most one floor shift limit");
const floorShiftLimit = floorLimits.length ? Number(floorLimits[0].split("=")[1]) : 2;
assert.ok(Number.isFinite(floorShiftLimit) && floorShiftLimit > 0, "Invalid floor shift limit");
const landingLimits = arguments_.filter((value) => value.startsWith("--landing-shift-limit="));
assert.ok(landingLimits.length <= 1, "Provide at most one landing shift limit");
const landingShiftLimit = landingLimits.length ? Number(landingLimits[0].split("=")[1]) : 2;
assert.ok(
  Number.isFinite(landingShiftLimit) && landingShiftLimit > 0,
  "Invalid landing shift limit",
);
const ids = arguments_.filter(
  (value) =>
    !value.startsWith("--external=") &&
    value !== "--local-landing-edges" &&
    value !== "--climb-seams" &&
    value !== "--sloped-landings" &&
    !value.startsWith("--outside-extension=") &&
    !value.startsWith("--midpoint-shift-limit=") &&
    !value.startsWith("--placement-ground-height=") &&
    !value.startsWith("--draft-issue=") &&
    !value.startsWith("--resolve-draft-issue=") &&
    !value.startsWith("--floor-shift-limit=") &&
    !value.startsWith("--landing-shift-limit="),
);
const usedExternal = new Set();
assert.ok(asset && ids.length, "Provide an asset and selected lift IDs");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === asset);
assert.ok(entry, asset);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
const descriptorSha256 = createHash("sha256").update(bytes).digest("hex");
assert.equal(descriptorSha256, entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const changes = [];
if (placementGroundHeight !== undefined) {
  changes.push({
    placementGroundHeight: { before: gameplay.placementGroundHeight, after: placementGroundHeight },
  });
  gameplay.placementGroundHeight = placementGroundHeight;
}
for (const id of ids) {
  const lift = gameplay.lifts.find((lift) => lift.id === id);
  assert.ok(
    climbSeams ? lift?.type === 2 || lift?.type === 3 : lift?.type === 1,
    `${id}: not a selected ${climbSeams ? "climb" : "stair"} type`,
  );
  const floor = gameplay.surfaces.find((surface) => surface.id === lift.surface);
  assert.ok(floor && !floor.holes?.length, `${id}: requires separate hole review`);
  const before = structuredClone(floor);
  const heights = floor.polygon.map((_, i) =>
    Array.isArray(floor.height) ? floor.height[i] : floor.height,
  );
  const plane = heightPlane(floor.polygon.map(([x, y], i) => [x, y, heights[i]]));
  const length = Math.hypot(plane[0], plane[1]);
  assert.ok(length > 1e-6);
  const contacts = lift.doors.map((door) => {
    const landings = gameplay.surfaces.filter((surface) => {
      if (surface === floor) return false;
      const landingPlane = heightPlane(
        surface.polygon.map(([x, y], i) => [
          x,
          y,
          Array.isArray(surface.height) ? surface.height[i] : surface.height,
        ]),
      );
      return (
        (slopedLandings || Math.hypot(landingPlane[0], landingPlane[1]) < 1e-8) &&
        Math.abs(planeHeight(landingPlane, door.outside) - door.outside[2]) < 1e-4 &&
        pointInGameplayPolygon(door.outside, surface.polygon, true) &&
        !(surface.holes ?? []).some((hole) => pointInGameplayPolygon(door.outside, hole, true))
      );
    });
    if (external.has(door.id)) {
      assert.equal(landings.length, 0, `${door.id}: external endpoint has a local landing`);
      usedExternal.add(door.id);
    } else {
      assert.equal(landings.length, 1, `${door.id}: requires external or ambiguous landing review`);
    }
    const landing = landings[0];
    const landingPlane = landing
      ? heightPlane(
          landing.polygon.map(([x, y], i) => [
            x,
            y,
            Array.isArray(landing.height) ? landing.height[i] : landing.height,
          ]),
        )
      : [0, 0, door.outside[2]];
    const difference = plane.map((v, i) => v - landingPlane[i]);
    const extension = outsideExtensions.get(door.id);
    if (extension !== undefined) {
      assert.ok(landing, `${door.id}: outside extension requires a local landing`);
      const oldOutside = [...door.outside];
      const dx = door.outside[0] - door.inside[0],
        dy = door.outside[1] - door.inside[1];
      const distance = Math.hypot(dx, dy);
      assert.ok(distance > 1e-6);
      door.outside[0] += (extension * dx) / distance;
      door.outside[1] += (extension * dy) / distance;
      door.outside[2] = planeHeight(landingPlane, door.outside);
      assert.ok(
        pointInGameplayPolygon(door.outside, landing.polygon, true) &&
          !(landing.holes ?? []).some((hole) => pointInGameplayPolygon(door.outside, hole, true)),
        `${door.id}: extended outside anchor leaves its landing`,
      );
      changes.push({ outsideDoor: door.id, before: oldOutside, after: [...door.outside] });
      usedExtensions.add(door.id);
    }
    const seamLength = Math.hypot(difference[0], difference[1]);
    assert.ok(seamLength > 1e-6, `${door.id}: parallel floor and landing`);
    const seat = (point) => {
      const t = -planeHeight(difference, point) / (seamLength * seamLength);
      return [point[0] + t * difference[0], point[1] + t * difference[1]];
    };
    return { door, landing, landingPlane, difference, seamLength, seat };
  });
  // Only end-adjacent vertices move. Bent side boundaries retain their
  // intermediate heights and cannot be flattened into an endpoint plane.
  floor.height = [...heights];
  floor.polygon = floor.polygon.map((point, i) => {
    const distance = (contact) =>
      Math.abs(planeHeight(contact.difference, point)) / contact.seamLength;
    const contact = contacts.reduce((a, b) => (distance(a) < distance(b) ? a : b));
    if (distance(contact) > floorShiftLimit) return point;
    const seated = contact.seat(point);
    floor.height[i] = planeHeight(contact.landingPlane, seated);
    return seated;
  });
  floor.preserveMovementPrecision = true;
  changes.push({ surface: floor.id, before, after: structuredClone(floor) });
  const adjusted = new Set();
  for (const { door, landing, landingPlane, difference, seamLength, seat } of contacts) {
    const a = planeHeight(difference, door.outside),
      b = planeHeight(difference, door.inside);
    const t = -a / (b - a);
    assert.ok(t >= 0 && t <= 1, `${door.id}: landing is outside the approach`);
    const oldMiddle = door.middle;
    door.middle = [
      door.outside[0] + t * (door.inside[0] - door.outside[0]),
      door.outside[1] + t * (door.inside[1] - door.outside[1]),
      0,
    ];
    door.middle[2] = planeHeight(landingPlane, door.middle);
    assert.ok(
      Math.hypot(...door.middle.map((v, i) => v - oldMiddle[i])) < midpointShiftLimit,
      `${door.id}: excessive midpoint correction`,
    );
    assert.ok(
      pointInGameplayPolygon(door.middle, floor.polygon, true),
      `${door.id}: midpoint still unsupported`,
    );
    changes.push({
      door: door.id,
      before: oldMiddle,
      after: door.middle,
      landing: landing?.id ?? "external placement receiver",
    });
    // Preserve the authored approach. The compiler must find real receiving
    // terrain or another placed asset; authoring creates no replacement floor.
    if (!landing) continue;
    if (adjusted.has(landing.id)) continue;
    // An inclined roof can already extend beyond the intersection. Keep its
    // boundary intact; placed actor checks must verify complete foot support.
    if (
      slopedLandings &&
      pointInGameplayPolygon(door.middle, landing.polygon, true) &&
      !(landing.holes ?? []).some((hole) => pointInGameplayPolygon(door.middle, hole, true))
    ) {
      const oldLanding = structuredClone(landing);
      landing.preserveMovementPrecision = true;
      changes.push({ surface: landing.id, before: oldLanding, after: structuredClone(landing) });
      adjusted.add(landing.id);
      continue;
    }
    const sideways = (point) => (-difference[1] * point[0] + difference[0] * point[1]) / seamLength;
    const seam = floor.polygon.filter((point) => Math.abs(planeHeight(difference, point)) < 1e-4);
    assert.ok(seam.length >= 2, `${door.id}: missing complete end edge`);
    const low = Math.min(...seam.map(sideways)),
      high = Math.max(...seam.map(sideways));
    const oldLanding = structuredClone(landing);
    const vertices = new Set();
    const matchingEdges = new Set();
    // A receiving edge can extend beyond both sides of a narrower stair.
    // Adjust the complete near-coplanar edge, not only vertices inside its span.
    landing.polygon.forEach((point, i) => {
      const j = (i + 1) % landing.polygon.length;
      const next = landing.polygon[j];
      if (
        [point, next].every(
          (p) => Math.abs(planeHeight(difference, p)) / seamLength <= landingShiftLimit,
        ) &&
        Math.min(high, Math.max(sideways(point), sideways(next))) >
          Math.max(low, Math.min(sideways(point), sideways(next)))
      ) {
        matchingEdges.add(i);
        vertices.add(i);
        vertices.add(j);
      }
    });
    assert.ok(vertices.size >= 2, `${door.id}: no matching landing edge`);
    const middleSide = sideways(door.middle);
    assert.ok(
      [...matchingEdges].some((i) => {
        const a = sideways(oldLanding.polygon[i]);
        const b = sideways(oldLanding.polygon[(i + 1) % oldLanding.polygon.length]);
        return middleSide >= Math.min(a, b) - 1e-6 && middleSide <= Math.max(a, b) + 1e-6;
      }),
      `${door.id}: matching landing edges miss the door midpoint; review the landing shift limit`,
    );
    if (localLandingEdges) {
      // Keep the rest of a longer receiving edge unchanged. Only its overlap
      // with the physical stair needs to meet the exact seam.
      landing.polygon = oldLanding.polygon.flatMap((point, i) => {
        const j = (i + 1) % oldLanding.polygon.length;
        const next = oldLanding.polygon[j];
        if (!matchingEdges.has(i)) return [point];
        const start = sideways(point),
          end = sideways(next);
        const delta = end - start;
        assert.ok(Math.abs(delta) > 1e-8, "Landing edge is perpendicular to seam");
        const ts = [low, high].map((v) => (v - start) / delta).sort((a, b) => a - b);
        const from = Math.max(0, ts[0]),
          to = Math.min(1, ts[1]);
        if (to <= from) return [point];
        const at = (t) => point.map((v, axis) => v + t * (next[axis] - v));
        return [point, at(from), seat(at(from)), seat(at(to)), at(to)]
          .filter(
            (p, k, points) =>
              k === 0 || Math.hypot(p[0] - points[k - 1][0], p[1] - points[k - 1][1]) > 1e-8,
          )
          .filter((p, k) => k === 0 || Math.hypot(p[0] - next[0], p[1] - next[1]) > 1e-8);
      });
    } else {
      landing.polygon = landing.polygon.map((point, i) => (vertices.has(i) ? seat(point) : point));
    }
    landing.height = landing.polygon.map((point) => planeHeight(landingPlane, point));
    landing.preserveMovementPrecision = true;
    changes.push({ surface: landing.id, before: oldLanding, after: structuredClone(landing) });
    adjusted.add(landing.id);
  }
  gameplay.movementClearances ??= [];
  const clearance = {
    id: `${floor.id}-seam-clearance`,
    node: floor.node,
    polygon: structuredClone(floor.polygon),
    height: [...floor.height],
    holes: [],
  };
  assert.ok(!gameplay.movementClearances.some((value) => value.id === clearance.id));
  gameplay.movementClearances.push(clearance);
  changes.push({ clearance: clearance.id, after: clearance });
}
assert.deepEqual(usedExternal, external, "Unused external endpoint selection");
assert.deepEqual(usedExtensions, new Set(outsideExtensions.keys()), "Unused outside extension");
for (const issue of resolvedIssues) {
  assert.ok(gameplay.draft?.issues.includes(issue), `Unknown resolved draft issue: ${issue}`);
  gameplay.draft.issues = gameplay.draft.issues.filter((value) => value !== issue);
}
if (draftIssues.length) {
  gameplay.draft ??= { issues: [] };
  gameplay.draft.issues = [...new Set([...gameplay.draft.issues, ...draftIssues])];
}
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/map-compile/local-stair-seams-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify([{ asset, descriptorSha256, gameplay }]));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: `unpublished local ${climbSeams ? "climb" : "stair"} seam candidate`,
      asset,
      descriptorSha256,
      floorShiftLimit,
      landingShiftLimit,
      localLandingEdges,
      slopedLandings,
      midpointShiftLimit,
      placementGroundHeight,
      changes,
    },
    null,
    2,
  ),
);
console.log(JSON.stringify({ output, asset, lifts: ids, changes: changes.length }));

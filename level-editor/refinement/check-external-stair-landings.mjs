import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { groupCentroid } from "../shared/src/level3d.ts";

// Synthetic receiving assets test placement connections, not visual fidelity.
// Their floors are authored from the reviewed stair seams; none is published.
const [staged, ...flags] = process.argv.slice(2);
assert.ok(
  staged &&
    flags.every((flag) => ["--published", "--preserve-landings", "--external-only"].includes(flag)),
);
const published = flags.includes("--published");
const preserveLandings = flags.includes("--preserve-landings");
const externalOnly = flags.includes("--external-only");
const review = JSON.parse(await fs.readFile(`${staged}/review.json`, "utf8"));
const externalDoors = externalOnly
  ? new Set(
      review.changes
        .filter((change) => change.landing === "external placement receiver")
        .map((change) => change.door),
    )
  : undefined;
if (externalOnly) assert.ok(externalDoors.size, "No reviewed external doors");
const edits = JSON.parse(await fs.readFile(`${staged}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const edit = edits[0];
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === edit.asset);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(bytes), entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
if (published) assert.deepEqual(descriptor.gameplay, edit.gameplay);
else {
  assert.equal(entry.descriptor_sha256, edit.descriptorSha256);
  descriptor.gameplay = edit.gameplay;
}
assert.equal(descriptor.gameplay.lifts.length, 1);
const lift = descriptor.gameplay.lifts[0];
const floor = descriptor.gameplay.surfaces.find((surface) => surface.id === lift.surface);
const plane = heightPlane(floor.polygon.map(([x, y], i) => [x, y, floor.height[i]]));
const landingDoors = lift.doors.filter((door) => !externalOnly || externalDoors.has(door.id));
if (externalOnly) assert.equal(landingDoors.length, externalDoors.size);
const landings = landingDoors.map((door, number) => {
  const landingPlane = review.changes.find((c) => c.door === door.id)?.landingPlane ?? [
    0,
    0,
    door.outside[2],
  ];
  const difference = plane.map((v, i) => v - landingPlane[i]);
  const length = Math.hypot(difference[0], difference[1]);
  assert.ok(length > 1e-6);
  const onSeam = floor.polygon.map(
    (p, i) => Math.abs(floor.height[i] - planeHeight(landingPlane, p)) < 1e-5,
  );
  let edge = floor.polygon.filter((_, i) => onSeam[i]);
  assert.ok(edge.length >= 2, "Fixture requires a receiving seam");
  assert.equal(
    onSeam.filter((on, i) => on && !onSeam[(i + onSeam.length - 1) % onSeam.length]).length,
    1,
    "Fixture requires one connected straight seam at each endpoint",
  );
  // Boundary clipping can retain intermediate collinear vertices. Preserve
  // the complete contact width without treating each vertex as a new landing.
  if (edge.length > 2) {
    const along = ([x, y]) => -difference[1] * x + difference[0] * y;
    edge.sort((a, b) => along(a) - along(b));
    edge = [edge[0], edge.at(-1)];
  }
  const width = Math.hypot(edge[1][0] - edge[0][0], edge[1][1] - edge[0][1]);
  const tangent = edge[1].map((value, axis) => (value - edge[0][axis]) / width);
  const ends = edge.map((point, i) =>
    point.map((value, axis) => value + tangent[axis] * (i ? 20 : -20)),
  );
  const side = Math.sign(planeHeight(difference, door.outside));
  const normal = [(difference[0] * side) / length, (difference[1] * side) / length];
  const polygon = [
    ends[0],
    ends[1],
    ...[ends[1], ends[0]].map((p) => p.map((v, i) => v + normal[i] * 80)),
  ];
  const id = `authored-landing-${number}`;
  return {
    version: 1,
    kind: "projection-mapped-asset",
    id,
    name: id,
    source_map: "synthetic-placement-fixture",
    model: "landing.glb",
    parts: [{ node: "scenery-landing", name: "Landing", scenery: true }],
    gameplay: {
      version: 1,
      collision: "none",
      placementGroundHeight: 0,
      doors: [],
      surfaces: [
        {
          id: "floor",
          node: "scenery-landing",
          preserveMovementPrecision: true,
          ...(preserveLandings
            ? { preserveMovementBoundary: true, navigationRegion: "landing" }
            : {}),
          polygon,
          height: polygon.map((p) => planeHeight(landingPlane, p)),
        },
      ],
    },
  };
});
const reference = {
  id: entry.id,
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(await fs.readFile(`library/3d-assets/${entry.model}`)),
  resources: descriptor.resources ?? [],
  ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
};
const assets = new Map([descriptor, ...landings].map((asset) => [asset.id, asset]));
const output = await fs.mkdtemp("work/map-compile/external-stair-landings-");
await fs.writeFile(`${output}/fixture-assets.json`, JSON.stringify([...assets]));
const results = [],
  rejected = [];
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: edit.asset,
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      size: [4000, 4000],
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
    };
    const landingGroups = [];
    for (const position of [
      [1000, 1500, height],
      [2200, 2500, height],
    ]) {
      document = insertProjectionAsset(document, descriptor, reference, position).document;
      const stair = document.groups.at(-1);
      stair.transform.rot_deg = rotation;
      const [cx, cy] = groupCentroid(document.objects.filter((part) => part.group === stair.id));
      const angle = (rotation * Math.PI) / 180,
        sinT = Math.sin((35 * Math.PI) / 180);
      // Scenery landings rotate about zero; preserve the stair assembly's pivot
      // while keeping the stair and landing placements independently editable.
      const pivotDx = cx * (1 - Math.cos(angle)) + (cy / sinT) * Math.sin(angle);
      const pivotDy = cy * (1 - Math.cos(angle)) - cx * sinT * Math.sin(angle);
      for (const landing of landings) {
        const ref = {
          id: landing.id,
          descriptor: `fixtures/${landing.id}.json`,
          descriptor_sha256: hash(JSON.stringify(landing)),
          model: "fixtures/landing.glb",
          model_sha256: "0".repeat(64),
        };
        document = insertProjectionAsset(document, landing, ref, position).document;
        const group = document.groups.at(-1);
        group.transform = { ...stair.transform };
        group.transform.dx += pivotDx;
        group.transform.dy += pivotDy;
        landingGroups.push(group.id);
      }
    }
    const compile = (doc) => compileMap(doc, [0, 0, 4000, 4000], assets, { bestEffort: true });
    const compiled = compile(document);
    assert.equal(
      compiled.descriptor.asset_geometry.lifts?.length,
      2,
      JSON.stringify(compiled.warnings),
    );
    assert.ok(
      compiled.descriptor.asset_geometry.lifts.every((lift) => lift.physical_navigation),
      JSON.stringify(compiled.warnings),
    );
    const file = `${edit.asset}-${height}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    results.push({ file, map: file, warnings: compiled.warnings });
    for (const id of landingGroups)
      for (const kind of ["missing", "raised"]) {
        const changed = structuredClone(document);
        if (kind === "missing") {
          changed.objects = changed.objects.filter((part) => part.group !== id);
          changed.groups = changed.groups.filter((group) => group.id !== id);
        } else changed.groups.find((group) => group.id === id).transform.dz += 20;
        const invalid = compile(changed);
        assert.equal(invalid.descriptor.asset_geometry.lifts?.length, 1, `${file}: ${id} ${kind}`);
        assert.ok(invalid.warnings.some((warning) => warning.includes("traversal omitted")));
        rejected.push({ file, landing: id, kind });
      }
  }
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "static-geometry-only-not-gameplay-parity",
    complete: true,
    snapshot_notes: { syntheticExternalLandings: true, preserveLandings, externalOnly },
    results,
  }),
);
await fs.writeFile(`${output}/rejected-landings.json`, JSON.stringify(rejected));
console.log(JSON.stringify({ output, placements: results.length * 2, rejected: rejected.length }));

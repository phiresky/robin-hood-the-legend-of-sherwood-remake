import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { heightPlane } from "../shared/src/gameplay-plane.ts";
import { groupCentroid } from "../shared/src/level3d.ts";

// Synthetic receiving assets test placement connections, not visual fidelity.
// Their floors are authored from the reviewed stair seams; none is published.
const [staged, ...flags] = process.argv.slice(2);
assert.ok(staged && flags.every((flag) => ["--published", "--preserve-landings"].includes(flag)));
const published = flags.includes("--published");
const preserveLandings = flags.includes("--preserve-landings");
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
const length = Math.hypot(plane[0], plane[1]);
const landings = lift.doors.map((door, number) => {
  const edge = floor.polygon.filter((_, i) => Math.abs(floor.height[i] - door.outside[2]) < 1e-5);
  assert.equal(edge.length, 2, "Fixture requires one straight seam at each endpoint");
  const width = Math.hypot(edge[1][0] - edge[0][0], edge[1][1] - edge[0][1]);
  const tangent = edge[1].map((value, axis) => (value - edge[0][axis]) / width);
  const ends = edge.map((point, i) =>
    point.map((value, axis) => value + tangent[axis] * (i ? 20 : -20)),
  );
  const side = Math.sign(
    plane[0] * door.outside[0] + plane[1] * door.outside[1] + plane[2] - door.outside[2],
  );
  const normal = [(plane[0] * side) / length, (plane[1] * side) / length];
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
          polygon: [
            ends[0],
            ends[1],
            ...[ends[1], ends[0]].map((point) => point.map((v, axis) => v + normal[axis] * 80)),
          ],
          height: door.outside[2],
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
      // while keeping the three placements independently editable.
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
    assert.ok(compiled.descriptor.asset_geometry.lifts.every((lift) => lift.physical_navigation));
    const file = `${edit.asset}-${height}-${rotation}.level.json`;
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
    snapshot_notes: { syntheticExternalLandings: true, preserveLandings },
    results,
  }),
);
await fs.writeFile(`${output}/rejected-landings.json`, JSON.stringify(rejected));
console.log(JSON.stringify({ output, placements: results.length * 2, rejected: rejected.length }));

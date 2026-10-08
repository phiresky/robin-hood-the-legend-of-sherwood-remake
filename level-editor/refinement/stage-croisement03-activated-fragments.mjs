import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { compileMap } from "../app/src/map-compile.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// One-time asset authoring: independent controls use each fragment's own solid
// footprint. The assembly's shared exclusion and remote waypoint are not copied.
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const ownerBytes = await fs.readFile(
  "library/3d-assets/croisement03-southwest-state-assembly/asset.json",
);
assert.equal(hash(ownerBytes), "e5973d2ec611c11ebb10ecdc4fe0f385c1196ad9ca682c9e0fd8e3a2a92d2d2a");
const owner = JSON.parse(ownerBytes);
assert.equal(owner.gameplay.movementTransitions.length, 1);
const control = owner.gameplay.movementTransitions[0];
assert.deepEqual(control.initialSight, []);
assert.deepEqual(control.initial, []);
assert.deepEqual(control.appliedSight, [
  "building-102",
  "building-103",
  "building-104",
  "building-105",
]);
const reviewed = [
  [
    "103",
    "d1a40e068f8b33ca4dc7d531bf7920356577e38b83f0aec9feade97313be5c9b",
    "4005309e60214e404d5f873ba2d8e24227d700c185d7bd0483f28060203bed97",
  ],
  [
    "104",
    "f2fb722296a2128754c5d34be0d080565d49acb1e5369f1a7e2deb6718361921",
    "809a2c823133f78a0383914485e8b6f2470e79a69eb7f71dcf6c005ce35a7a28",
  ],
  [
    "105",
    "583daf05cd879b76dc63f8d6ece687f2a7f7e97f74659a8ed12144920079f4ac",
    "dddb7cf19c5d4f48713a7f7683da350003fc1d29411f14420c1060e866134144",
  ],
];
const assets = new Map(),
  references = new Map(),
  edits = [];
for (const [suffix, descriptorSha256, modelSha256] of reviewed) {
  const id = `croisement03-group-${suffix}`;
  const base = `3d-assets/croisement03/${id}`;
  const bytes = await fs.readFile(`library/${base}/asset.json`);
  assert.equal(hash(bytes), descriptorSha256);
  assert.equal(hash(await fs.readFile(`library/${base}/model.glb`)), modelSha256);
  const descriptor = JSON.parse(bytes);
  assert.equal(descriptor.gameplay, undefined);
  assert.equal(descriptor.parts.length, 1);
  const part = descriptor.parts[0];
  assert.equal(part.node, `building-${suffix}`);
  assert.ok(control.appliedSight.includes(part.node));
  assert.ok(
    owner.gameplay.movementBlockers.every((blocker) => blocker.node !== part.node),
    "A fragment with permanent collision needs separate authoring",
  );
  assert.equal(owner.gameplay.surfaces.length, 0);
  assert.equal(owner.gameplay.doors.length, 0);
  assert.equal(owner.gameplay.lifts.length, 0);
  const shape = part.obstacle_local_game;
  assert.ok(shape.solid && shape.points.every((p) => p.z_bottom === 0));
  const center = [0, 1].map(
    (axis) => shape.points.reduce((sum, p) => sum + (axis ? p.y : p.x), 0) / shape.points.length,
  );
  part.appearance = { show: ["activate-fragment"] };
  descriptor.gameplay = {
    version: 1,
    collision: "parts",
    surfaces: [],
    doors: [],
    movementBlockers: [],
    movementTransitions: [
      {
        id: "activate-fragment",
        appearances: ["activate-fragment"],
        node: part.node,
        active: control.active,
        definitive: control.definitive,
        waypoint: [...center, 0],
        applyPolygon: [],
        noApplyPolygon: [],
        initial: [],
        applied: [
          {
            id: "activated-footprint",
            node: part.node,
            polygon: shape.points.map((p) => [p.x, p.y]),
            height: 0,
            preserveMovementPrecision: true,
          },
        ],
        initialSight: [],
        appliedSight: [part.node],
      },
    ],
    draft: {
      issues: [
        "Independent activated-fragment draft; full gameplay parity is not certified.",
        "Ground blocking is derived from this fragment's solid footprint; shared assembly exclusion and remote trigger placement are deliberately not inherited.",
        "The control starts unapplied. Activate it explicitly to enable this fragment; rendered state appearance and mesh contact still require review.",
      ],
    },
  };
  validateAssetGameplay(descriptor.gameplay, descriptor);
  assets.set(id, descriptor);
  references.set(id, {
    id,
    role: "objects",
    model: `${base}/model.glb`,
    model_sha256: modelSha256,
    descriptor: `${base}/asset.json`,
    descriptor_sha256: descriptorSha256,
    model_scene: descriptor.model_scene,
    resources: descriptor.resources,
  });
  edits.push({
    asset: id,
    descriptorSha256,
    modelSha256,
    gameplay: descriptor.gameplay,
    partAppearances: { [part.node]: part.appearance },
  });
}
const output = await fs.mkdtemp("work/map-compile/croisement03-activated-fragments-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      complete,
      results,
    }),
  );
await report(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: "Independent activated fragments",
      size: [1000, 1500],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 1000, 1500], 250, height),
    };
    let placement = 0;
    for (const [id, descriptor] of assets)
      for (const copy of [0, 1]) {
        document = insertProjectionAsset(document, descriptor, references.get(id), [
          250.25 + 450 * copy,
          250.75 + 450 * placement,
          height,
        ]).document;
        document.groups.at(-1).transform.rot_deg = rotation;
        if (copy === 1) placement++;
      }
    const compiled = compileMap(document, [0, 0, 1000, 1500], assets);
    const geometry = compiled.descriptor.asset_geometry;
    assert.equal(geometry.movement_transitions.length, 6);
    assert.equal(geometry.sight_obstacles.length, 7, "Six fragments plus the terrain receiver");
    const probes = geometry.movement_transitions.map((transition) => {
      assert.equal(transition.initial_sight?.length ?? 0, 0);
      assert.equal(transition.applied_sight.length, 1);
      const shape = geometry.sight_obstacles[transition.applied_sight[0]];
      const xs = shape.points.map((p) => p.x),
        ys = shape.points.map((p) => p.y - p.z_bottom);
      const y = ys.reduce((a, b) => a + b) / ys.length;
      return {
        id: transition.id,
        layer: 0,
        start: [Math.min(...xs) - 15, y],
        end: [Math.max(...xs) + 15, y],
        initial: true,
        applied: false,
      };
    });
    const file = `fragments-${height}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({ file, map: file, warnings: compiled.warnings, transition_probes: probes });
    await report(false);
  }
await report(true);

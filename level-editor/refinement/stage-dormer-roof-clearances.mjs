// Author roof access independently of the house's approximate enclosing solids.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const id = "nottingham-north-dormer-house";
const base = `3d-assets/nottingham/${id}`;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/${base}/asset.json`);
const descriptorSha256 = hash(bytes);
assert.equal(descriptorSha256, "600628aa7939be2491be9d0371bbab4e0c6c1c051dfe116762e9e5d928e2de00");
const descriptor = JSON.parse(bytes);
assert.deepEqual(
  descriptor.gameplay.surfaces.map((s) => s.node),
  ["building-123", "building-124"],
);
const original = structuredClone(descriptor);
for (const surface of descriptor.gameplay.surfaces) {
  // These authored roof faces are usable decks. Adjacent enclosing house
  // volumes protrude slightly through them; they are not rooftop obstacles.
  const { projectionMaterials: _materials, navigationRegion: _region, ...clearance } = surface;
  descriptor.gameplay.movementClearances.push({
    ...structuredClone(clearance),
    id: `${surface.id}-support-clearance`,
    preserveMovementPrecision: true,
  });
}
descriptor.gameplay.draft.issues.push(
  "Authored roof clearances exclude this house's enclosing support volumes. Independently placed obstacles still block the roof; roof connections and rendered actor contact require separate verification.",
);
validateAssetGameplay(descriptor.gameplay, descriptor);
const modelSha256 = hash(await fs.readFile(`library/${base}/model.glb`));
const reference = {
  id,
  descriptor: `${base}/asset.json`,
  descriptor_sha256: descriptorSha256,
  model: `${base}/model.glb`,
  model_sha256: modelSha256,
  model_scene: descriptor.model_scene,
  resources: descriptor.resources,
};
const output = await fs.mkdtemp("work/needs-review/map-compile/dormer-roof-clearances-");
console.log(output);
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([{ asset: id, descriptorSha256, modelSha256, gameplay: descriptor.gameplay }]),
);
const results = [];
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: "Independent dormer roof clearance",
      size: [1000, 1000],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 1000, 1000], 500, height),
    };
    for (const x of [250.25, 700.25]) {
      const inserted = insertProjectionAsset(document, descriptor, reference, [x, 500.75, height]);
      document = inserted.document;
      document.groups.find((group) => group.id === inserted.selection.id).transform.rot_deg =
        rotation;
    }
    const compile = (asset) =>
      compileMap(document, [0, 0, 1000, 1000], new Map([[id, asset]]), { bestEffort: true })
        .descriptor;
    const before = compile(original),
      after = compile(descriptor);
    // A roof clearance cannot erase the house or its removable ground prop.
    assert.deepEqual(
      after.asset_geometry.motion_data.layers[0],
      before.asset_geometry.motion_data.layers[0],
    );
    const solids = (level) => level.asset_geometry.sight_obstacles.filter((s) => s.solid);
    assert.deepEqual(solids(after), solids(before));
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(after));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({ file });
  }
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({ scope: "roof-clearance-exports-not-gameplay-parity", complete: true, results }),
);

// Stage the complete keep from its current component models and local gameplay.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { mergeStaticAppearances } from "../pipeline/src/merge-static-appearances.ts";
import { canonical, modelContentSignatures } from "../pipeline/src/bundle-asset-states.ts";
import {
  normalizeStaticAssetModel,
  verifyStaticParts,
  writeStaticAssetModel,
} from "../pipeline/src/static-asset-model.ts";
import { sceneToGame, sceneToGltf } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { serializeStoredMap } from "../shared/src/stored-level.ts";

const require = createRequire(new URL("../pipeline/package.json", import.meta.url));
const { Document, NodeIO } = require("@gltf-transform/core");
const { ALL_EXTENSIONS } = require("@gltf-transform/extensions");
const { mergeDocuments, unpartition } = require("@gltf-transform/functions");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const candidate = process.argv[2];
assert.ok(candidate, "Usage: stage-derby-keep-model.mjs <gameplay-candidate-directory>");
const authoring = JSON.parse(await fs.readFile(path.join(candidate, "authoring.json")));
assert.equal(authoring.id, "derby-great-keep");
const readDescriptor = async (id, expected) => {
  const bytes = await fs.readFile(`library/3d-assets/derby/${id}/asset.json`);
  assert.equal(hash(bytes), expected, `${id}: descriptor changed`);
  return JSON.parse(bytes);
};
const descriptor = await readDescriptor(authoring.id, authoring.descriptorSha256);
descriptor.gameplay = JSON.parse(
  await fs.readFile(path.join(candidate, "candidate.gameplay.json")),
);
const io = new NodeIO().registerExtensions(ALL_EXTENSIONS);
const target = new Document();
const scene = target.createScene("default");
target.getRoot().setDefaultScene(scene);
const assembly = target.createNode(descriptor.id);
scene.addChild(assembly);
const expected = new Map(),
  sources = [],
  owned = new Set();
const bounds = { min: [Infinity, Infinity, Infinity], max: [-Infinity, -Infinity, -Infinity] };
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
for (const component of authoring.components) {
  const source = await readDescriptor(component.id, component.descriptorSha256);
  const offset = source.source_origin_scene.map(
    (v, axis) => v - descriptor.source_origin_scene[axis],
  );
  const gameOffset = sceneToGame(camera, offset);
  for (const part of source.parts) {
    assert.ok(!owned.has(part.node), `Duplicate part ${part.node}`);
    owned.add(part.node);
    component.offsets[part.node].forEach((v, axis) =>
      assert.ok(
        Math.abs(v - gameOffset[axis]) < 0.001,
        `${part.node}: model/gameplay frame mismatch`,
      ),
    );
  }
  for (let axis = 0; axis < 3; axis++) {
    bounds.min[axis] = Math.min(
      bounds.min[axis],
      source.bounds_local_scene.min[axis] + offset[axis],
    );
    bounds.max[axis] = Math.max(
      bounds.max[axis],
      source.bounds_local_scene.max[axis] + offset[axis],
    );
  }
  const bytes = await fs.readFile(`library/3d-assets/derby/${component.id}/model.glb`);
  const model = await io.readBinary(bytes);
  const scenes = model.getRoot().listScenes();
  assert.equal(scenes.length, 1);
  assert.equal(scenes[0].getName(), source.model_scene);
  assert.deepEqual(scenes[0].getExtras(), {});
  assert.deepEqual(model.getRoot().getExtras(), {});
  mergeStaticAppearances(scenes[0], component.id, undefined);
  const wrapper = model.createNode(`component:${component.id}`).setTranslation(sceneToGltf(offset));
  for (const child of scenes[0].listChildren()) wrapper.addChild(child);
  scenes[0].addChild(wrapper);
  const signatures = await modelContentSignatures(model);
  for (const part of source.parts) {
    const nodes = model
      .getRoot()
      .listNodes()
      .filter((node) => node.getName() === part.node);
    assert.equal(nodes.length, 1);
    expected.set(part.node, signatures.nodeData(nodes[0]));
  }
  const mapping = mergeDocuments(target, model);
  const merged = mapping.get(scenes[0]);
  for (const child of merged.listChildren()) assembly.addChild(child);
  merged.dispose();
  sources.push({
    id: component.id,
    descriptorSha256: component.descriptorSha256,
    modelSha256: hash(bytes),
    offset,
  });
}
assert.equal(owned.size, descriptor.parts.length);
for (const part of descriptor.parts) assert.ok(owned.has(part.node));
await target.transform(unpartition());
const names = descriptor.parts.map((part) => part.node);
const expectedParts = names.map((name) => expected.get(name));
await verifyStaticParts(target, names, expectedParts);
await normalizeStaticAssetModel(target, descriptor.id, names);
const bytes = await writeStaticAssetModel(io, target);
const reopened = await io.readBinary(bytes);
await verifyStaticParts(reopened, names, expectedParts);
for (const part of descriptor.parts)
  assert.equal(
    reopened
      .getRoot()
      .listNodes()
      .filter((node) => node.getName() === part.node).length,
    1,
  );
const triggers = new Set();
reopened
  .getRoot()
  .listNodes()
  .forEach((node) => {
    const extras = node.getExtras();
    for (const key of ["reveal_hide_when_applied", "reveal_show_when_applied"])
      for (const id of extras[key] ?? []) triggers.add(id);
    if (extras.reveal_material_patch) triggers.add(extras.reveal_material_patch);
  });
const controls = new Set(
  descriptor.gameplay.movementTransitions.flatMap((entry) => entry.appearances ?? []),
);
assert.deepEqual(triggers, controls, "Model triggers must match independent gameplay controls");
descriptor.bounds_local_scene = bounds;
descriptor.gameplay.draft.issues = descriptor.gameplay.draft.issues.filter(
  (issue) =>
    !issue.startsWith("Combined keep gameplay is staged from its local component definitions."),
);
descriptor.gameplay.draft.issues.push(
  "Combined keep candidate: elevated entrance support, mask boundaries and rendered reveal states remain under review.",
);
validateAssetGameplay(descriptor.gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/derby-keep-model-");
await fs.writeFile(path.join(output, "model.glb"), bytes);
await fs.writeFile(path.join(output, "asset.json"), JSON.stringify(descriptor));
const document = JSON.parse(await fs.readFile(path.join(candidate, "37-40.level.json.scene.json")));
assert.equal(document.assetSources.length, 1);
assert.equal(document.assetSources[0].id, descriptor.id);
document.map = "keep-model";
document.assetSources[0].descriptor_sha256 = hash(JSON.stringify(descriptor));
document.assetSources[0].model_sha256 = hash(bytes);
const assetDirectory = path.join(output, "3d-assets/derby", descriptor.id);
await fs.mkdir(assetDirectory, { recursive: true });
await fs.writeFile(
  path.join(output, "3d-assets/index.json"),
  JSON.stringify({
    version: 1,
    assets: [
      {
        id: descriptor.id,
        name: descriptor.name,
        source_map: descriptor.source_map,
        descriptor: `derby/${descriptor.id}/asset.json`,
        model: `derby/${descriptor.id}/model.glb`,
        model_scene: descriptor.model_scene,
        descriptor_sha256: document.assetSources[0].descriptor_sha256,
        model_sha256: document.assetSources[0].model_sha256,
        editor: descriptor,
      },
    ],
  }),
);
for (const file of ["asset.json", "model.glb"])
  await fs.link(path.resolve(output, file), path.join(assetDirectory, file));
for (const directory of ["game-data", "population"])
  await fs.symlink(path.resolve("library", directory), path.join(output, directory), "dir");
await fs.mkdir(path.join(output, "scenes"));
await fs.writeFile(
  path.join(output, "scenes/keep-model.rhlos-map.json"),
  JSON.stringify(serializeStoredMap(document, new Map([[descriptor.id, descriptor]]))),
);
await fs.writeFile(
  path.join(output, "authoring.json"),
  JSON.stringify({
    candidate,
    sources,
    modelSha256: hash(bytes),
    appearanceTriggers: [...triggers],
    verifiedPartSignatures: Object.fromEntries(
      [...expected].map(([name, data]) => [name, hash(canonical(data))]),
    ),
    publicationReady: false,
    reason: "Authored landing support and rendered appearance validation remain required",
  }),
);
console.log(
  JSON.stringify({
    output,
    bytes: bytes.length,
    triggers: [...triggers],
    verifiedParts: expected.size,
  }),
);

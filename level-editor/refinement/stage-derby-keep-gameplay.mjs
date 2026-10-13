// Reassemble reviewed component gameplay in the complete keep's local frame.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { translateGameplayFrames } from "../pipeline/src/translate-gameplay-frames.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const pins = {
  "derby-great-keep": "09723b8fc9da1464276c6b1ce9678961cf991f4d9508ad4d19c667197b73c393",
  "derby-keep-west-tower": "216dd4ec2d371e383270fedf4d6e840098cbd11ae978febc001f9cb156a3c555",
  "derby-keep-main-hall": "ba8683a8dd02f13522efb419bb1db7d4375659c20aa3d65be4920149bd9c7262",
  "derby-keep-central-gallery": "61fa9c26d22934239ed4d1b8a76340ad4b35dc8e2d7f3b2e3d4c98cb9bb8b559",
  "derby-keep-north-tower": "c5112ce0d48adccfc5215e5fe4b26ece00a08754665dad0fb68ab0f7f8afb8ba",
};
const read = async (id) => {
  const bytes = await fs.readFile(`library/3d-assets/derby/${id}/asset.json`);
  assert.equal(hash(bytes), pins[id], `${id}: descriptor changed`);
  return JSON.parse(bytes);
};
const id = "derby-great-keep",
  descriptor = await read(id);
assert.equal(descriptor.gameplay, undefined);
const owned = new Set(),
  components = [];
const gameplay = {
  version: 1,
  collision: "parts",
  surfaces: [],
  doors: [],
  sightOrder: {},
  draft: { issues: [] },
};
const arrays = new Set([
  "surfaces",
  "doors",
  "sounds",
  "lights",
  "masks",
  "movementTransitions",
  "movementBlockers",
  "lifts",
  "interiors",
  "movementClearances",
]);
for (const componentId of Object.keys(pins).filter((key) => key !== id)) {
  const source = await read(componentId),
    offsets = new Map();
  for (const part of source.parts) {
    assert.ok(!owned.has(part.node), `Duplicate component part ${part.node}`);
    owned.add(part.node);
    const target = descriptor.parts.find((candidate) => candidate.node === part.node);
    assert.ok(target, `Missing combined part ${part.node}`);
    const a = part.obstacle_local_game,
      b = target.obstacle_local_game;
    const delta = [
      b.points[0].x - a.points[0].x,
      b.points[0].y - a.points[0].y,
      b.points[0].z_bottom - a.points[0].z_bottom,
    ];
    assert.equal(a.points.length, b.points.length);
    for (const key of ["solid", "opaque", "mouse"]) assert.equal(a[key], b[key]);
    a.points.forEach((point, i) => {
      for (const [key, axis] of [
        ["x", 0],
        ["y", 1],
        ["z_bottom", 2],
        ["z_top", 2],
      ])
        assert.ok(
          Math.abs(point[key] + delta[axis] - b.points[i][key]) < 0.001,
          `${part.node}: non-translational collision change`,
        );
    });
    offsets.set(part.node, delta);
  }
  const translated = translateGameplayFrames(source.gameplay, offsets);
  // Component controls both used appearance-1; the complete model must give
  // their visual triggers separate names before this draft is published.
  for (const control of translated.movementTransitions ?? [])
    if (control.appearances)
      control.appearances = control.appearances.map((name) => `${componentId}/${name}`);
  for (const [key, value] of Object.entries(translated)) {
    if (arrays.has(key)) (gameplay[key] ??= []).push(...value);
    else if (key === "sightOrder") Object.assign(gameplay.sightOrder, value);
    else if (key === "draft") gameplay.draft.issues.push(...value.issues);
    else
      assert.ok(
        (key === "version" && value === 1) || (key === "collision" && value === "parts"),
        `Unreviewed component field ${key}`,
      );
  }
  components.push({
    id: componentId,
    descriptorSha256: pins[componentId],
    offsets: Object.fromEntries(offsets),
  });
}
assert.equal(owned.size, descriptor.parts.length);
for (const key of arrays) {
  const entries = gameplay[key] ?? [];
  assert.equal(
    new Set(entries.map((entry) => entry.id)).size,
    entries.length,
    `Duplicate ${key} identities`,
  );
}
gameplay.placementGroundHeight = Math.min(
  ...descriptor.parts.flatMap((part) =>
    part.obstacle_local_game.points.map((point) => point.z_bottom),
  ),
);
gameplay.draft.issues = [
  ...new Set([
    ...gameplay.draft.issues,
    "Combined keep gameplay is staged from its local component definitions. The combined model still needs independent appearance metadata migration; this draft is not ready for publication.",
  ]),
];
descriptor.gameplay = gameplay;
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/derby-keep-gameplay-");
console.log(output);
await fs.writeFile(`${output}/candidate.gameplay.json`, JSON.stringify(gameplay));
await fs.writeFile(
  `${output}/authoring.json`,
  JSON.stringify({
    id,
    descriptorSha256: pins[id],
    components,
    publicationReady: false,
    reason: "Combined model appearance migration and native traversal validation remain required",
  }),
);
const reference = {
  id,
  descriptor: `3d-assets/derby/${id}/asset.json`,
  descriptor_sha256: pins[id],
  model: `3d-assets/derby/${id}/model.glb`,
  model_sha256: hash(await fs.readFile(`library/3d-assets/derby/${id}/model.glb`)),
  model_scene: descriptor.model_scene,
  resources: descriptor.resources,
};
const results = [];
await fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({ complete: false, results }));
for (const rotation of [0, 37, 90, 180]) {
  const size = 2400,
    height = 40;
  let document = {
    version: 1,
    map: "Independent complete keep",
    size: [size, size],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    objects: [],
    groups: [],
    sceneAssets: [],
    assetSources: [],
    terrain: createTerrainGrid([0, 0, size, size], size / 2, height),
  };
  const inserted = insertProjectionAsset(document, descriptor, reference, [
    1200.25,
    1500.75,
    height,
  ]);
  document = inserted.document;
  document.groups.find((group) => group.id === inserted.selection.id).transform.rot_deg = rotation;
  const compiled = compileMap(document, [0, 0, size, size], new Map([[id, descriptor]]), {
    bestEffort: true,
  });
  const file = `${rotation}-40.level.json`;
  await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
  await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
  results.push({ file, warnings: compiled.warnings });
  await fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "combined-keep-draft-not-gameplay-or-appearance-parity",
      complete: false,
      results,
    }),
  );
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "combined-keep-draft-not-gameplay-or-appearance-parity",
    complete: true,
    expected_directed_routes: 40,
    results,
  }),
);

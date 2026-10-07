// Asset-only rooted foliage candidate. Rendered mask behavior needs review.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh, maskRecoveryTextures } from "../pipeline/src/mask-recovery-mesh.ts";
import { maskCoverage } from "../pipeline/src/mask-roundtrip.ts";
import { sceneToGame, gltfToScene } from "../shared/src/geometry.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json")).assets;
const output = await fs.mkdtemp("work/map-compile/fern-gameplay-");
const edits = [],
  results = [],
  reviews = [];
console.log(JSON.stringify({ output }));
function hull(points) {
  const sorted = [
    ...new Map(points.map(([x, y]) => [JSON.stringify([x, y]), [x, y]])).values(),
  ].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const cross = (a, b, c) => (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
  const chain = (values) => {
    const result = [];
    for (const p of values) {
      while (result.length > 1 && cross(result.at(-2), result.at(-1), p) <= 0) result.pop();
      result.push(p);
    }
    return result.slice(0, -1);
  };
  return [...chain(sorted), ...chain(sorted.toReversed())];
}
const requested = process.argv.slice(2);
assert.ok(
  requested.every((arg) => arg.startsWith("--asset=")),
  "Use --asset=<library-id>",
);
const selected = requested.length
  ? requested.map((arg) => arg.slice("--asset=".length))
  : ["croisement03-fern-35", "croisement03-fern-76"];
assert.equal(new Set(selected).size, selected.length, "Duplicate selected foliage asset");
for (const id of selected) {
  const entry = index.find((entry) => entry.id === id);
  assert.ok(entry);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(hash(bytes), entry.descriptor_sha256);
  const descriptor = JSON.parse(bytes);
  assert.equal(descriptor.gameplay, undefined);
  assert.equal(descriptor.parts.length, 1);
  const node = descriptor.parts[0].node;
  const reference = {
    id,
    role: "objects",
    model: `3d-assets/${entry.model}`,
    model_sha256: hash(await fs.readFile(`library/3d-assets/${entry.model}`)),
    descriptor: `3d-assets/${entry.descriptor}`,
    descriptor_sha256: hash(bytes),
    model_scene: entry.model_scene,
    resources: descriptor.resources ?? [],
  };
  const model = await loadSceneModel("library", reference);
  assert.ok(
    model
      .getRoot()
      .listMaterials()
      .every(
        (material) =>
          material.getAlphaMode() === "MASK" &&
          material.getExtras().foliage_physical_opacity === true,
      ),
    "Rooted foliage authoring requires physical-alpha materials",
  );
  const textures = await maskRecoveryTextures(model);
  const triangles = maskRecoveryMesh(
    model,
    node,
    (p) => sceneToGame(camera, gltfToScene(p)),
    textures,
    undefined,
    { preserveMaterialSidedness: true },
  );
  assert.ok(triangles.length);
  const points = triangles.flat();
  // These rooted plants were authored on scene Z=0, with their origin just above
  // it. Store that ground contact locally so terrain placement preserves it.
  const ground = sceneToGame(camera, [0, 0, -descriptor.source_origin_scene[2]])[2];
  assert.ok(Math.abs(ground) < 0.1, "Review changed plant ground contact");
  const boundary = hull(points).map(([x, y]) => [x, y, ground]);
  assert.ok(boundary.length >= 3);
  descriptor.gameplay = {
    version: 1,
    collision: "none",
    placementGroundHeight: ground,
    surfaces: [],
    doors: [],
    maskOcclusionNodes: [node],
    masks: [
      {
        id: "foliage-cover",
        node,
        triangles,
        cullBackfaces: true,
        anchor: [0, 0, ground],
        view: true,
        characterBoundary: boundary,
        projectileBoundary: boundary,
        obstacles: [],
      },
    ],
    draft: {
      issues: [
        "Foliage mask uses alpha-covered asset geometry and a closed canopy footprint on its ground plane. Front-envelope behavior is an authored approximation; rendered character/projectile contact still needs review.",
      ],
    },
  };
  validateAssetGameplay(descriptor.gameplay, descriptor);
  edits.push({
    asset: id,
    descriptorSha256: hash(bytes),
    modelSha256: reference.model_sha256,
    gameplay: descriptor.gameplay,
  });
  reviews.push({
    asset: id,
    modelSha256: reference.model_sha256,
    triangles: triangles.length,
    boundary,
    ground,
    scope: "Alpha coverage retained; closed canopy boundary is not a recovered native polyline",
  });
  for (const elevation of [0, 40])
    for (const rotation of [0, 37, 90, 180, 270]) {
      const empty = {
        version: 1,
        map: id,
        camera,
        size: [1000, 1000],
        objects: [],
        groups: [],
        sceneAssets: [],
        assetSources: [],
        terrain: createTerrainGrid([0, 0, 1000, 1000], 250, elevation),
      };
      const { document } = insertProjectionAsset(empty, descriptor, reference, [
        500.25,
        500.75,
        elevation,
      ]);
      document.groups[0].transform.rot_deg = rotation;
      const compiled = compileMap(document, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
        bestEffort: false,
      });
      const masks = compiled.descriptor.asset_geometry.masks;
      assert.ok(masks?.length, "Fern must retain typed mask coverage");
      for (const mask of masks) {
        assert.equal(mask.mask_type, 7);
        assert.ok(mask.mask_data.length);
        assert.ok(mask.character_polyline.length >= 2 && mask.projectile_polyline.length >= 2);
        const xs = [...maskCoverage(mask)].map((pixel) => Number(pixel.split(",")[0]));
        assert.ok(xs.length > 0 && xs.length < mask.box_size[0] * mask.box_size[1]);
        for (const boundary of [mask.character_polyline, mask.projectile_polyline]) {
          assert.ok(boundary[0][0] <= Math.min(...xs) + 0.5);
          assert.ok(boundary.at(-1)[0] >= Math.max(...xs) + 0.5);
        }
      }
      const floating = structuredClone(document);
      floating.terrain = createTerrainGrid([0, 0, 1000, 1000], 250, elevation - 1);
      assert.throws(
        () =>
          compileMap(floating, [0, 0, 1000, 1000], new Map([[id, descriptor]]), {
            bestEffort: false,
          }),
        /receiving anchor/,
      );
      assert.equal(
        compiled.descriptor.asset_geometry.motion_data.layers
          .flatMap((layer) => layer)
          .flatMap((area) => area.obstacles).length,
        0,
        "Fern must not invent movement collision",
      );
      const file = `${id}-${elevation}-${rotation}.level.json`;
      await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
      await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
      results.push({
        file,
        map: file,
        rotation,
        elevation,
        masks: masks.length,
        wrongHeightRejected: true,
      });
      console.log(JSON.stringify(results.at(-1)));
    }
}
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify(reviews));
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    complete: true,
    scope: "static-geometry-only-not-gameplay-parity",
    note: "Candidate construction only; rendered mask behavior remains unverified",
    results,
  }),
);

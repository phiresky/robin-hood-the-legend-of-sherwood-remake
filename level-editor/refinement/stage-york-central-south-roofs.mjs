// Stage physical roof support for independent placement and mesh review.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const mode = process.argv[2] ?? "support";
assert.ok(["support", "edges", "both", "inset", "faces"].includes(mode));
const scene = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", scene.assetSources, scene.sceneAssets);
const edits = [];
const reviews = [];
for (const [name, surfaceId, segmentId] of [
  ["red", "building-323-walk-0", "jump-pair-45-side-0"],
  ["rear", "building-331-walk-0", "jump-pair-45-side-1"],
]) {
  const id = `york-central-south-${name}-timber-house`;
  const asset = assets.get(id);
  const source = scene.assetSources.find((entry) => entry.id === id);
  assert.ok(asset && source);
  const gameplay = structuredClone(asset.gameplay);
  const roof = gameplay.surfaces.find((surface) => surface.id === surfaceId);
  const edge = gameplay.jumpSegments.find((segment) => segment.id === segmentId)?.edge;
  assert.ok(roof && edge);
  const before = { roof: structuredClone(roof), edge: structuredClone(edge) };
  const plane = heightPlane(roof.projectionMaterials.planePoints);
  const footprint = roof.projectionMaterials.footprint;
  assert.ok(footprint.length >= 3);
  assert.ok(footprint.every(([x, y, z]) => Math.abs(z - planeHeight(plane, [x, y])) < 0.001));
  if (mode !== "edges") {
    roof.polygon = footprint.map(([x, y]) => [x, y]);
    roof.height = roof.polygon.map((point) => planeHeight(plane, point));
  }
  if (mode !== "support" && mode !== "faces") {
    if (mode === "inset") {
      const center = [0, 1].map((axis) =>
        footprint.reduce((sum, point) => sum + point[axis] / footprint.length, 0),
      );
      const delta = center.map((value, axis) => value - (edge.a[axis] + edge.b[axis]) / 2);
      const distance = Math.hypot(...delta);
      assert.ok(distance > 10, "Inset must stay between the edge and roof center");
      for (const point of [edge.a, edge.b])
        for (const axis of [0, 1]) point[axis] += (10 * delta[axis]) / distance;
    }
    for (const point of [edge.a, edge.b]) point[2] = planeHeight(plane, point);
  }
  if (mode === "faces") {
    const node = name === "red" ? "building-322" : "building-330";
    const volume = asset.parts.find((part) => part.node === node)?.obstacle_local_game;
    assert.ok(volume && !gameplay.surfaces.some((surface) => surface.node === node));
    const vertices = volume.points.map((point) => [point.x, point.y, point.z_top]);
    const anchors = vertices.slice(0, 3);
    const plane = heightPlane(anchors);
    const footprint = vertices.map(([x, y]) => [x, y, planeHeight(plane, [x, y])]);
    gameplay.surfaces.push({
      id: `${node}-walk-0`,
      node,
      navigationRegion: roof.navigationRegion,
      polygon: footprint.map(([x, y]) => [x, y]),
      height: footprint.map((point) => point[2]),
      holes: [],
      preserveMovementPrecision: true,
      projectionMaterials: {
        defaultMaterial: volume.default_material,
        regions: [],
        planePoints: anchors,
        footprint,
        priority: -Number(node.slice("building-".length)),
        priorityHeight: Math.max(...footprint.map((point) => point[2])),
      },
    });
  }
  validateAssetGameplay(gameplay, asset);
  edits.push({
    asset: id,
    descriptorSha256: source.descriptor_sha256,
    modelSha256: source.model_sha256,
    gameplay,
  });
  reviews.push({ source, before, after: { roof, edge, surfaces: gameplay.surfaces } });
}
const output = await fs.mkdtemp("work/map-compile/york-central-south-roofs-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ scope: "unpublished-roof-support-experiment", mode, reviews }),
);
console.log(output);

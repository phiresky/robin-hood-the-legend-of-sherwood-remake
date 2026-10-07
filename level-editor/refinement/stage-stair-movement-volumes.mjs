import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Explicit unpublished asset authoring. A corrected stair flight must carry
// matching movement collision when placed against a newly authored floor.
const args = process.argv.slice(2);
const standalone = args.includes("--standalone");
const [map, stage, ...ids] = args.filter((argument) => argument !== "--standalone");
assert.ok(map && stage && ids.length, "Provide map, gameplay stage and stair asset IDs");
assert.ok(![map, stage, ...ids].some((argument) => argument.startsWith("--")), "Unknown option");
const document = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const changes = [];
for (const id of ids) {
  const descriptor = assets.get(id);
  const edit = edits.find((edit) => edit.asset === id);
  assert.ok(descriptor && edit, `Missing staged asset: ${id}`);
  assert.equal(
    edit.descriptorSha256,
    document.assetSources.find((source) => source.id === id)?.descriptor_sha256,
    `Staged descriptor changed: ${id}`,
  );
  const gameplay = edit.gameplay;
  const stairs = (gameplay.lifts ?? []).filter((lift) => lift.type === 1);
  assert.ok(stairs.length, `No stair flights in ${id}`);
  if (standalone) {
    // Publishable stair-only reviews must not inherit unrelated solids enabled
    // by a larger terrain experiment. Keep the library's other collision rules.
    assert.ok(
      descriptor.gameplay.movementBlockers !== undefined,
      `Implicit part collision needs separate review: ${id}`,
    );
    gameplay.movementSolids = [
      ...new Set([
        ...(descriptor.gameplay.movementSolids ?? []),
        ...stairs.map(
          (lift) => gameplay.surfaces.find((surface) => surface.id === lift.surface).node,
        ),
      ]),
    ];
  }
  for (const lift of stairs) {
    const floor = gameplay.surfaces.find((surface) => surface.id === lift.surface);
    const part = descriptor.parts.find((part) => part.node === floor?.node);
    assert.ok(part?.obstacle_local_game?.solid && !floor.holes?.length);
    assert.ok(
      gameplay.movementSolids?.includes(part.node),
      `No selected solid: ${id}/${part.node}`,
    );
    const shape = part.obstacle_local_game;
    const bottom = heightPlane(shape.points.map((p) => [p.x, p.y, p.z_bottom]));
    const points = floor.polygon.map(([x, y], index) => [
      x,
      y,
      typeof floor.height === "number" ? floor.height : floor.height[index],
    ]);
    const top = heightPlane(points);
    assert.ok(points.every(([x, y, z]) => Math.abs(planeHeight(top, [x, y]) - z) < 1e-4));
    // Volume consumers fit their planes from the first three vertices.
    // Rotate the ring past any redundant collinear upper-edge samples.
    let rotated;
    for (let i = 0; i < points.length; i++) {
      const ring = [...points.slice(i), ...points.slice(0, i)];
      const [a, b, c] = ring;
      if (Math.abs((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) > 1e-6) {
        rotated = ring;
        break;
      }
    }
    assert.ok(rotated, `Degenerate stair footprint: ${id}/${floor.id}`);
    const volumeId = `${floor.id}-movement-volume`;
    assert.ok(!gameplay.volumes?.some((volume) => volume.id === volumeId));
    const volume = {
      id: volumeId,
      node: floor.node,
      shape: {
        solid: true,
        opaque: false,
        mouse: false,
        show_shadow_polygon: false,
        default_material: shape.default_material,
        points: rotated.map(([x, y, z]) => ({
          x,
          y,
          z_bottom: planeHeight(bottom, [x, y]),
          z_top: z,
        })),
      },
    };
    assert.ok(volume.shape.points.every((p) => p.z_bottom <= p.z_top));
    gameplay.volumes ??= [];
    gameplay.volumes.push(volume);
    gameplay.movementSolids = gameplay.movementSolids.map((node) =>
      node === part.node ? volumeId : node,
    );
    changes.push({ asset: id, node: part.node, before: shape, after: volume });
  }
  gameplay.draft ??= { issues: [] };
  gameplay.draft.issues.push(
    "Unpublished stair movement volume follows the authored flight; moved contact and mesh review remain required.",
  );
  validateAssetGameplay(gameplay, descriptor);
}
const output = await fs.mkdtemp("work/map-compile/stair-movement-volumes-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify(standalone ? edits.filter((edit) => ids.includes(edit.asset)) : edits),
);
await fs.writeFile(`${output}/review.json`, JSON.stringify({ map, stage, standalone, changes }));
console.log(JSON.stringify({ output, changedFlights: changes.length }));

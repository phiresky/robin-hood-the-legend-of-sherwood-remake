import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { compileMap } from "../app/src/map-compile.ts";
import { compoundLiftCompilerFixture } from "../shared/test-fixtures/asset-gameplay.ts";
import { parseLevel3D, parseProjectionAssetDescriptor } from "../shared/src/validation.ts";
import nearEdgeOnStair from "../shared/test-fixtures/near-edge-on-placed-stair.json" with { type: "json" };

// Synthetic editor documents exercise reusable joined assets, without level input.
const output = await fs.mkdtemp("work/map-compile/compound-climb-placements-");
const results = [];
for (const type of [2, 3]) {
  for (const height of [0, 40]) {
    for (const rotation of [0, 37, 90, 180]) {
      const { document, assets } = compoundLiftCompilerFixture();
      for (const asset of assets.values())
        for (const lift of asset.gameplay?.lifts ?? []) lift.type = type;
      const groups = [...document.groups];
      const objects = [...document.objects];
      for (const group of groups) {
        group.transform = { dx: 800, dy: 800, dz: height, rot_deg: rotation };
        document.groups.push({
          id: `${group.id}-copy`,
          transform: { ...group.transform, dx: 1800 },
        });
      }
      for (const object of objects.filter((object) => object.group))
        document.objects.push({
          ...structuredClone(object),
          id: `${object.id}-copy`,
          group: `${object.group}-copy`,
        });
      const compiled = compileMap(document, [0, 0, 4000, 4000], assets);
      const lifts = compiled.descriptor.asset_geometry.lifts;
      assert.equal(lifts.length, 2, JSON.stringify(compiled.warnings));
      assert.notEqual(lifts[0].motion_area_index, lifts[1].motion_area_index);
      for (const lift of lifts) {
        assert.equal(lift.lift_type, type);
        assert.equal(lift.physical_navigation.floor_patches.length, 2);
        assert.equal(lift.doors.length, 2);
      }
      const file = `climb-${type}-${height}-${rotation}.level.json`;
      await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
      await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
      results.push({ file, type, height, rotation, warnings: compiled.warnings });
    }
  }
}
for (const type of [2, 3]) {
  const document = parseLevel3D(nearEdgeOnStair.document);
  const asset = parseProjectionAssetDescriptor(nearEdgeOnStair.asset);
  for (const lift of asset.gameplay.lifts) lift.type = type;
  const compiled = compileMap(document, [0, 0, ...document.size], new Map([[asset.id, asset]]));
  const lifts = compiled.descriptor.asset_geometry.lifts;
  assert.equal(lifts.length, 2, JSON.stringify(compiled.warnings));
  assert.ok(lifts.every((lift) => lift.lift_type === type && lift.physical_navigation));
  const file = `pinched-climb-${type}.level.json`;
  await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
  await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
  results.push({ file, type, pinchedProjection: true, warnings: compiled.warnings });
}
await fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({
  scope: "compiled-placement-fixtures-not-native-traversal-certification",
  complete: true,
  results,
}, null, 2));
console.log(JSON.stringify({ output, maps: results.length, expectedDirectedRoutes: results.length * 4 }));

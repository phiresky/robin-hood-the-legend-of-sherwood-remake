import assert from "node:assert/strict";
import test from "node:test";
import { assetCompilerFixture } from "../test-fixtures/asset-gameplay.ts";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";

test("exported precise floors remove numeric seam slits before graph preparation", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.collision = "none";
  hut.gameplay!.doors = [];
  hut.gameplay!.surfaces = [
    {
      id: "roof",
      node: "building-999",
      preserveMovementPrecision: true,
      height: 20,
      polygon: [
        [0, 0],
        [50, 0],
        [50, 90],
        [50.000008, 0],
        [100, 0],
        [100, 100],
        [0, 100],
      ],
    },
  ];
  document.groups[0]!.transform = { dx: 900.125, dy: 900.125, dz: 0, rot_deg: 0 };
  const compiled = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]);
  const areas = compiled.motion_data.layers.flat();
  assert.equal(areas.length, 1);
  const area = areas[0]!;
  const floor = area.precise_polygon ?? area.polygon.points;
  const minX = Math.min(...floor.map((point) => point[0]));
  const maxX = Math.max(...floor.map((point) => point[0]));
  const minY = Math.min(...floor.map((point) => point[1]));
  const maxY = Math.max(...floor.map((point) => point[1]));
  const y = (minY + maxY) / 2;
  assert.deepEqual(
    prepareCorridorStates(area.obstacles, floor)(
      [minX + (maxX - minX) / 4, y],
      [maxX - (maxX - minX) / 4, y],
    ),
    [0],
  );
  assert.ok(compiled.motion_data.graph_bytes.length);
});

import assert from "node:assert/strict";
import test from "node:test";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import { ordinaryPassageBarrierCompilerFixture } from "../test-fixtures/asset-gameplay.ts";
import { quantizePassageReceivers } from "./quantize-passage-receivers.ts";

function fixture() {
  const { document, assets } = ordinaryPassageBarrierCompilerFixture();
  const geometry = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]);
  const door = geometry.doors[0]!;
  door.point_in = [11, 20];
  door.world_endpoints = {
    inside: [10.55, 120.2, 100],
    outside: [0, 0, 0],
    middle: [0, 0, 0],
  };
  geometry.doors = [door];
  geometry.sight_obstacles = [
    {
      points: [
        [10, 20],
        [10.6, 20],
        [10.6, 21],
        [10, 21],
      ].map(([x, y]) => ({
        x: x!,
        y: y! + 100,
        z_bottom: 100,
        z_top: 100,
      })),
      projection_area: [door.sector_in, door.layer_in],
      opaque: false,
      solid: false,
      mouse: true,
      show_shadow_polygon: false,
      default_material: 0,
      material_indices: [],
    },
  ];
  return { geometry, door };
}

for (const type of [0, 3])
  test(`passage type ${type} quantization retains receiving support and world elevation`, () => {
    const { geometry, door } = fixture();
    door.door_type = type;
    quantizePassageReceivers(geometry);
    assert.deepEqual(door.point_in, [10, 20]);
    assert.deepEqual(door.world_endpoints!.inside, [10, 120, 100]);
    const once = structuredClone(geometry);
    quantizePassageReceivers(geometry);
    assert.deepEqual(geometry, once);
  });

test("unsupported anchors and receivers belonging to other sectors cannot attract a passage", () => {
  for (const mismatch of ["anchor", "sector"] as const) {
    const { geometry, door } = fixture();
    if (mismatch === "anchor") door.world_endpoints!.inside[0] = 11.1;
    else geometry.sight_obstacles[0]!.projection_area = [65535, door.layer_in];
    const before = structuredClone(geometry);
    quantizePassageReceivers(geometry);
    assert.deepEqual(geometry, before);
  }
});

test("a narrow rotated receiver can require the next integer beyond floor and ceil", () => {
  const { geometry, door } = fixture();
  door.point_in = [1453, 1260];
  door.world_endpoints!.inside = [1453.4455302684394, 1686.2341655275748, 426.7025468448412];
  const receiver = geometry.sight_obstacles[0]!;
  receiver.points = [
    [1508.327525138855, 1711.476430417446, 472.54079103603726],
    [1508.318127632141, 1711.4680956005468, 472.5289104580297],
    [1466.4494895935059, 1674.6955618408392, 419.96069474489195],
    [1423.3492498397827, 1690.548951128556, 419.8092498574988],
    [1527.145990371704, 1781.5823139408021, 550.0011467197328],
    [1570.0218343734741, 1765.661461585958, 550.0010878982139],
  ].map(([x, y, z]) => ({ x: x!, y: y!, z_bottom: z!, z_top: z! }));
  quantizePassageReceivers(geometry);
  assert.deepEqual(door.point_in, [1452, 1260]);
  assert.ok(
    Math.abs(door.world_endpoints!.inside[1] - door.world_endpoints!.inside[2] - 1260) < 1e-8,
  );
});

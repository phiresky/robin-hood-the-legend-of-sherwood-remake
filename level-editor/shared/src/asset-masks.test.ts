import test from "node:test";
import assert from "node:assert/strict";
import { maskAssetCompilerFixture } from "../test-fixtures/asset-gameplay.ts";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import { validateAssetGameplay } from "./asset-gameplay.ts";
import { IDENTITY_TRANSFORM } from "./level3d.ts";
import type { AssetGameplay } from "./asset-gameplay.ts";

const bounds: [number, number, number, number] = [0, 0, 2000, 2000];
test("mask ground contact points survive an unsupported origin and reject wrong or competing layers", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  gameplay.movementTransitions = [];
  const mask = gameplay.masks![0]!;
  const baseline = compileAssetGameplay(document, assets, bounds).masks;
  mask.anchor = [150, 150, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /found 0/);
  mask.receiverPoints = [
    [150, 150, 0],
    [45, 80, 0],
  ];
  assert.deepEqual(compileAssetGameplay(document, assets, bounds).masks, baseline);
  mask.receiverPoints = [[45, 80, 1]];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /found 0/);
  gameplay.surfaces.push({ ...gameplay.surfaces[0]!, id: "second-contact-floor", height: 5 });
  mask.receiverPoints = [
    [45, 80, 0],
    [45, 80, 5],
  ];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /exactly one authored receiving layer/,
  );
  mask.receiverPoints = [];
  assert.throws(() => validateAssetGameplay(gameplay, hut), /invalid mask receiving points/);
});

test("mask receiving polylines resolve all bends and reject competing layers", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  gameplay.movementTransitions = [];
  gameplay.collision = "none";
  const mask = gameplay.masks![0]!;
  for (const item of gameplay.masks!) item.obstacles = [];
  const before = compileAssetGameplay(document, assets, bounds).masks;
  mask.receiverPolyline = [
    [40, 80, -10],
    [45, 80, 10],
    [50, 80, -10],
  ];
  assert.deepEqual(compileAssetGameplay(document, assets, bounds).masks, before);
  gameplay.surfaces.push({ ...gameplay.surfaces[0]!, id: "upper-probe-floor", height: 5 });
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /exactly one authored receiving layer/,
  );
  gameplay.surfaces.pop();
  mask.receiverPolyline = [
    [40, 80, 10],
    [45, 80, 20],
    [50, 80, 10],
  ];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /found 0/);
  mask.receiverPolyline = [
    [40, 80, -10],
    [40, 80, -10],
  ];
  assert.throws(() => validateAssetGameplay(gameplay, hut), /invalid mask receiving polyline/);
});

test("sloped mask receivers evaluate elevation at the fractional authored point", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  gameplay.movementTransitions = [];
  gameplay.collision = "none";
  gameplay.surfaces = [{ ...gameplay.surfaces[0]!, height: [0, 90, 90, 0] }];
  for (const mask of gameplay.masks!) {
    mask.anchor = [45.25, 80.5, 45.25];
    mask.obstacles = [];
  }
  assert.equal(compileAssetGameplay(document, assets, bounds).masks!.length, 2);
  gameplay.masks![0]!.anchor[2] += 1;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiving anchor/);
});

test("mask receivers survive movement exclusions but require authored surface support", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  gameplay.movementTransitions = [];
  const before = compileAssetGameplay(document, assets, bounds).masks;
  gameplay.movementBlockers = [
    {
      id: "covered-ground",
      node: "building-999",
      height: 0,
      polygon: [
        [0, 0],
        [55, 0],
        [55, 100],
        [0, 100],
      ],
    },
  ];
  assert.deepEqual(compileAssetGameplay(document, assets, bounds).masks, before);
  gameplay.surfaces[0]!.polygon = [
    [55, 0],
    [90, 0],
    [90, 100],
    [55, 100],
  ];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /authored receiving layer/);
});

test("open asset boundaries compile separate character and projectile rules", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  const mask = hut.gameplay!.masks![0]!;
  mask.characterBoundary = [
    [40, 50, 5],
    [45, 40, 5],
    [50, 50, 5],
  ];
  mask.characterBoundaryClosed = false;
  mask.projectileBoundary = [
    [40, 55, 5],
    [50, 60, 5],
  ];
  mask.projectileBoundaryClosed = false;
  validateAssetGameplay(hut.gameplay, hut);
  const compiled = compileAssetGameplay(document, assets, bounds).masks![0]!;
  assert.deepEqual(compiled.character_polyline, [
    [340, 345],
    [345, 335],
    [350, 345],
  ]);
  assert.deepEqual(compiled.projectile_polyline, [
    [340, 355],
    [350, 360],
  ]);
  mask.projectileBoundaryClosed = true;
  assert.throws(() => validateAssetGameplay(hut.gameplay, hut), /mask boundary/);
  delete mask.projectileBoundary;
  assert.throws(() => validateAssetGameplay(hut.gameplay, hut), /mask boundary closure/);
});

test("asset mask geometry, rules and transitions compile entirely from local definitions", () => {
  const { document, assets } = maskAssetCompilerFixture();
  const first = compileAssetGameplay(document, assets, bounds);
  assert.equal(first.masks?.length, 2);
  const mask = first.masks![0]!;
  assert.equal(mask.mask_type, 23);
  assert.deepEqual(mask.box_top_left, [340, 310]);
  assert.deepEqual(mask.box_size, [10, 40]);
  assert.deepEqual(mask.character_polyline, [
    [340, 350],
    [350, 350],
  ]);
  assert.deepEqual(mask.projectile_polyline, []);
  assert.deepEqual(mask.obstacle_indices, [0]);
  assert.deepEqual(first.movement_transitions![0]!.initial_masks, [0]);
  assert.deepEqual(first.movement_transitions![0]!.applied_masks, [1]);
  document.groups[0]!.transform.dx = 100;
  document.groups[0]!.transform.dy = -10;
  document.groups[0]!.transform.dz = 10;
  const moved = compileAssetGameplay(document, assets, bounds).masks![0]!;
  assert.deepEqual(moved.box_top_left, [440, 290]);
  assert.deepEqual(moved.character_polyline, [
    [440, 330],
    [450, 330],
  ]);
  assert.deepEqual(moved.mask_data, mask.mask_data);
});

test("rotated duplicates rebuild independent mask and obstacle state references", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  for (const mask of hut.gameplay!.masks!) {
    mask.receiverPoints = [mask.anchor];
    mask.anchor = [150, 150, 0];
  }
  const part = document.objects.find((p) => p.group)!;
  document.groups.push({
    id: "mask-copy",
    transform: { ...IDENTITY_TRANSFORM, dx: 1000, rot_deg: 90 },
  });
  document.objects.push({ ...structuredClone(part), id: "mask-copy-part", group: "mask-copy" });
  const geometry = compileAssetGameplay(document, assets, bounds);
  assert.equal(geometry.masks?.length, 4);
  assert.deepEqual(
    geometry.movement_transitions!.map((t) => [t.initial_masks, t.applied_masks]),
    [
      [[0], [1]],
      [[2], [3]],
    ],
  );
  assert.deepEqual(
    geometry.masks!.map((mask) => mask.obstacle_indices),
    [[0], [0], [1], [1]],
  );
  assert.notDeepEqual(geometry.masks![0]!.box_top_left, geometry.masks![2]!.box_top_left);
  assert.ok(
    geometry.masks!.every(
      (mask) => mask.character_polyline![0]![0] < mask.character_polyline!.at(-1)![0],
    ),
  );
});

test("one local mask state controls every generated bitmap tile", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  hut.gameplay!.masks![0]!.triangles = [
    [
      [0, 0, 0],
      [1500, 0, 0],
      [1500, 1, 0],
    ],
    [
      [0, 0, 0],
      [1500, 1, 0],
      [0, 1, 0],
    ],
  ];
  const geometry = compileAssetGameplay(document, assets, bounds);
  assert.equal(geometry.masks!.length, 3);
  assert.deepEqual(geometry.movement_transitions![0]!.initial_masks, [0, 1]);
  assert.deepEqual(geometry.movement_transitions![0]!.applied_masks, [2]);
});

test("mask authoring rejects missing rules, references, geometry and competing state ownership", () => {
  for (const mutate of [
    (g: AssetGameplay) => {
      g.masks![0]!.obstacles = ["missing"];
    },
    (g: AssetGameplay) => {
      g.masks![0]!.triangles = [];
    },
    (g: AssetGameplay) => {
      g.masks![0]!.view = false;
      g.masks![0]!.obstacles = [];
      delete g.masks![0]!.characterBoundary;
    },
    (g: AssetGameplay) => {
      g.movementTransitions![0]!.appliedMasks = ["covered"];
    },
    (g: AssetGameplay) => {
      g.movementTransitions![0]!.appliedMasks = ["missing"];
    },
  ]) {
    const { hut } = maskAssetCompilerFixture();
    mutate(hut.gameplay!);
    assert.throws(() => validateAssetGameplay(hut.gameplay, hut), /mask/i);
  }
  const { document, assets, hut } = maskAssetCompilerFixture();
  hut.gameplay!.masks![0]!.anchor = [500, 500, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiving anchor/);
});

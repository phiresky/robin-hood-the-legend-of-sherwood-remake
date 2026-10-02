import test from "node:test";
import assert from "node:assert/strict";
import { maskAssetCompilerFixture } from "../../shared/test-fixtures/asset-gameplay.ts";
import { compileAssetGameplay } from "../../shared/src/compile-asset-gameplay.ts";
import { validateAssetGameplay } from "../../shared/src/asset-gameplay.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import type { MaskTriangle } from "../../shared/src/compile-mask-geometry.ts";
import { recoverOcclusionMask, type MaskRecoveryDefinition } from "./recover-occlusion-mask.ts";

function fixture() {
  const data = maskAssetCompilerFixture();
  const authored = data.hut.gameplay!.masks![0]!;
  const bounds: [number, number, number, number] = [0, 0, 2000, 2000];
  const source = compileAssetGameplay(data.document, data.assets, bounds).masks![0]!;
  const definition: MaskRecoveryDefinition = {
    id: authored.id,
    node: authored.node,
    anchor: [345, 345, 0],
    surfaces: authored.triangles.map(
      (t) => t.map(([x, y, z]): Vec3 => [x + 300, y + 300, z]) as MaskTriangle,
    ),
    characterHeights: source.character_polyline!.map(() => 0),
    obstacles: new Map([[0, authored.obstacles[0]!]]),
    localize: ([x, y, z]) => [x - 300, y - 300, z],
  };
  return { ...data, bounds, source, definition };
}

test("complete recovered mask rules compile identically without source data", () => {
  const { document, assets, hut, bounds, source, definition } = fixture();
  hut.gameplay!.masks![0] = recoverOcclusionMask(source, definition);
  validateAssetGameplay(hut.gameplay, hut);
  assert.deepEqual(compileAssetGameplay(document, assets, bounds).masks![0], source);
  document.groups[0]!.transform.dx = 25;
  document.groups[0]!.transform.dz = 10;
  const moved = compileAssetGameplay(document, assets, bounds).masks![0]!;
  assert.deepEqual(moved.box_top_left, [source.box_top_left[0] + 25, source.box_top_left[1] - 10]);
  assert.deepEqual(
    moved.character_polyline,
    source.character_polyline!.map(([x, y]) => [x + 25, y - 10]),
  );
  assert.deepEqual(moved.mask_data, source.mask_data);
});

test("mask recovery localizes authored receiving segments independently of coverage", () => {
  const { source, definition } = fixture();
  const baseline = recoverOcclusionMask(source, definition);
  definition.receiverSegment = [
    [345, 345, -8],
    [345, 345, 8],
  ];
  const recovered = recoverOcclusionMask(source, definition);
  assert.deepEqual(recovered.receiverSegment, [
    [45, 45, -8],
    [45, 45, 8],
  ]);
  const { receiverSegment: _segment, ...coverage } = recovered;
  assert.deepEqual(coverage, baseline);
});

test("character elevation and projectile world XY recover independently", () => {
  const { source, definition, document, assets, hut, bounds } = fixture();
  source.projectile_polyline = [
    [340, 355],
    [350, 360],
  ];
  definition.characterHeights = [5, 8];
  definition.projectileHeights = [20, 30];
  const mask = recoverOcclusionMask(source, definition);
  assert.deepEqual(mask.characterBoundary, [
    [40, 55, 5],
    [50, 58, 8],
  ]);
  assert.deepEqual(mask.projectileBoundary, [
    [40, 55, 20],
    [50, 60, 30],
  ]);
  assert.equal(mask.characterBoundaryClosed, false);
  assert.equal(mask.projectileBoundaryClosed, false);
  assert.equal("layer" in mask, false);
  assert.equal("obstacle_indices" in mask, false);
  assert.equal("mask_data" in mask, false);
  hut.gameplay!.masks![0] = mask;
  assert.deepEqual(compileAssetGameplay(document, assets, bounds).masks![0], source);
});

test("recovery preserves every supported combination of masking rules", () => {
  for (const flags of [1, 2, 3, 4, 5, 6, 7, 18, 19, 22, 23]) {
    const { source, definition, document, assets, hut, bounds } = fixture();
    source.mask_type = flags;
    if (!(flags & 1)) {
      source.character_polyline = null;
      delete definition.characterHeights;
    }
    if (!(flags & 16)) source.obstacle_indices = [];
    if (!(flags & 2)) source.projectile_polyline = null;
    else if (!(flags & 16)) {
      source.projectile_polyline = [
        [340, 350],
        [350, 350],
      ];
      definition.projectileHeights = [0, 0];
    }
    hut.gameplay!.masks![0] = recoverOcclusionMask(source, definition);
    assert.deepEqual(compileAssetGameplay(document, assets, bounds).masks![0], source);
  }
});

test("recovered geometry does not retain mutable authoring inputs", () => {
  const { source, definition } = fixture();
  definition.localize = (p) => p;
  const recovered = recoverOcclusionMask(source, definition);
  const snapshot = structuredClone(recovered);
  definition.anchor[0] = -1;
  definition.surfaces[0]![0][0] = -1;
  definition.characterHeights![0] = -1;
  source.character_polyline![0]![0] = -1;
  assert.deepEqual(recovered, snapshot);
});

test("mask recovery refuses missing elevation, local ownership and incompatible flags", () => {
  for (const mutate of [
    ({ definition }: ReturnType<typeof fixture>) => {
      delete definition.characterHeights;
    },
    ({ definition }: ReturnType<typeof fixture>) => {
      definition.characterHeights = [NaN, 0];
    },
    ({ definition }: ReturnType<typeof fixture>) => {
      definition.obstacles = new Map();
    },
    ({ source }: ReturnType<typeof fixture>) => {
      source.mask_type = 8;
    },
    ({ source }: ReturnType<typeof fixture>) => {
      source.mask_type = 2 ** 32 + 23;
    },
    ({ source }: ReturnType<typeof fixture>) => {
      source.mask_type = 7;
    },
    ({ source }: ReturnType<typeof fixture>) => {
      source.character_polyline = null;
    },
    ({ source }: ReturnType<typeof fixture>) => {
      source.projectile_polyline = null;
    },
    ({ definition }: ReturnType<typeof fixture>) => {
      definition.anchor = [NaN, 0, 0];
    },
  ]) {
    const input = fixture();
    mutate(input);
    assert.throws(() => recoverOcclusionMask(input.source, input.definition));
  }
});

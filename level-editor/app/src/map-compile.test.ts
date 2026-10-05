import test from "node:test";
import assert from "node:assert/strict";
import { decode } from "fast-png";
import { unzipSync, strFromU8 } from "fflate";
import * as THREE from "three";
import { type Level3D, gameToScene, parseStoredMap } from "@rle/shared";
import { compileMap, packageCompiledMap, validateBakeBounds } from "./map-compile.ts";
import {
  bakeScene,
  contentBakeBounds,
  maskOcclusionObjects,
  withDepthOcclusion,
} from "./map-bake-render.ts";
import {
  assetCompilerFixture,
  rotatedSceneryCompilerFixture,
  anchoredReceiverCompilerFixture,
  preservedBoundaryCompilerFixture,
  preservedStateBoundaryCompilerFixture,
  preservedContoursCompilerFixture,
  maskAssetCompilerFixture,
  unavailableMaskControlCompilerFixture,
  slopedAssetCompilerFixture,
  liftAssetCompilerFixture,
  changingLiftCompilerFixture,
  changingClimbCompilerFixture,
  copiedChangingLiftCompilerFixture,
  disconnectedLiftCompilerFixture,
  liftLightCompilerFixture,
  interiorAssetCompilerFixture,
  joinedInteriorCompilerFixture,
  connectedInteriorCompilerFixture,
  clearanceAssetCompilerFixture,
  materialAssetCompilerFixture,
  projectionMaterialCompilerFixture,
  projectionVolumeCompilerFixture,
  unavailableProjectionControlCompilerFixture,
  receivingGapCompilerFixture,
  receivingIslandCompilerFixture,
  soundAssetCompilerFixture,
  movementTransitionCompilerFixture,
  terrainTransitionCompilerFixture,
  unavailableTerrainControlCompilerFixture,
  appearanceOnlyCompilerFixture,
  joinedTransitionCompilerFixture,
  sightTransitionCompilerFixture,
  inactiveJumpObstacleCompilerFixture,
  swordJumpObstacleCompilerFixture,
  lightAssetCompilerFixture,
  jumpAssetCompilerFixture,
  navigationRegionCompilerFixture,
  compoundLiftCompilerFixture,
  multiPlaneRegionCompilerFixture,
  slopedTerrainSocketCompilerFixture,
  joinedNavigationCompilerFixture,
  nonrenderingVolumeCompilerFixture,
  crossAssetJumpCompilerFixture,
  geometricJumpCompilerFixture,
  levelContourJumpCompilerFixture,
  movementBlockedJumpCompilerFixture,
  changingJumpApproachCompilerFixture,
  obstructedJumpCompilerFixture,
  multiDestinationJumpCompilerFixture,
  detachedJumpCompilerFixture,
  doorTransitionCompilerFixture,
  doorAnchorCompilerFixture,
} from "../../shared/test-fixtures/asset-gameplay.ts";
import { readFile } from "node:fs/promises";

test("rotated scenery exports the boundary checked by native display ordering", async () => {
  const { document, assets } = rotatedSceneryCompilerFixture();
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-scenery-rotated.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, expected);
});

test("joined asset switches match the native multi-part apply/reset fixture", async () => {
  const { document, assets } = joinedTransitionCompilerFixture();
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-joined-transition.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, expected);
});

test("preserved state boundary export matches native apply/reset geometry", async () => {
  const { document, assets } = preservedStateBoundaryCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-preserved-state-boundary.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("anchored receiver export preserves native shared ground navigation", async () => {
  const { document, assets } = anchoredReceiverCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-anchored-receiver.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("receiving island export preserves native area and material ownership", async () => {
  const { document, assets } = receivingIslandCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-receiving-island.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("merged platform export preserves the opening checked by native receiving queries", async () => {
  const { document, assets } = receivingGapCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-receiving-gap.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("disconnected lift retains collision, landings and another lift in the same asset", async () => {
  const { document, assets } = disconnectedLiftCompilerFixture();
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets, { bestEffort: true });
  assert.equal(compiled.descriptor.asset_geometry!.lifts!.length, 1);
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-disconnected-lift.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compiled.descriptor, fixture);
});

test("permanently inactive obstacles retain full jump spans", async () => {
  const { document, assets } = inactiveJumpObstacleCompilerFixture();
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets);
  const clear = geometricJumpCompilerFixture();
  const expected = compileMap(clear.document, [0, 0, 2000, 2000], clear.assets);
  assert.deepEqual(
    compiled.descriptor.asset_geometry!.jump_line_pairs,
    expected.descriptor.asset_geometry!.jump_line_pairs,
  );
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-jump-inactive.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compiled.descriptor, fixture);
});

test("switchable inactive obstacles still constrain jumps that cannot change with the switch", () => {
  const { document, assets } = inactiveJumpObstacleCompilerFixture();
  assets.get("jump-wall")!.gameplay!.movementTransitions = [
    {
      id: "jump-wall-state",
      node: "building-999",
      waypoint: [20, 50, 0],
      active: true,
      definitive: false,
      initial: [],
      applied: [],
      initialSight: [],
      appliedSight: ["jump-obstruction"],
      applyPolygon: [],
      noApplyPolygon: [],
    },
  ];
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets);
  const obstructed = obstructedJumpCompilerFixture();
  const expected = compileMap(obstructed.document, [0, 0, 2000, 2000], obstructed.assets);
  assert.deepEqual(
    compiled.descriptor.asset_geometry!.jump_line_pairs,
    expected.descriptor.asset_geometry!.jump_line_pairs,
  );
  assets.get("jump-wall")!.gameplay!.movementTransitions![0]!.waypoint[2] = 100;
  const frozen = compileMap(document, [0, 0, 2000, 2000], assets, { bestEffort: true });
  const clear = geometricJumpCompilerFixture();
  assert.deepEqual(
    frozen.descriptor.asset_geometry!.jump_line_pairs,
    compileMap(clear.document, [0, 0, 2000, 2000], clear.assets).descriptor.asset_geometry!
      .jump_line_pairs,
  );
  assert.equal(frozen.descriptor.asset_geometry!.movement_transitions?.length ?? 0, 0);
});

test("unavailable sight control preserves inactive receiving geometry", async () => {
  const { document, assets } = unavailableProjectionControlCompilerFixture();
  assert.throws(() => compileMap(document, [0, 0, 2000, 2000], assets), /waypoint must resolve/);
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets, { bestEffort: true });
  const geometry = compiled.descriptor.asset_geometry!;
  assert.equal(geometry.movement_transitions?.length ?? 0, 0);
  const receiver = geometry.sight_obstacles.find((obstacle) => obstacle.initial_active === false)!;
  assert.ok(receiver.projection_area);
  assert.ok(receiver.material_indices.length);
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-unavailable-projection-control.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compiled.descriptor, fixture);
});

test("unavailable sight and movement control retains the closed obstacle only", () => {
  const { document, assets, hut } = sightTransitionCompilerFixture();
  hut.gameplay!.movementTransitions![0]!.waypoint[2] = 100;
  const geometry = compileMap(document, [0, 0, 2000, 2000], assets, { bestEffort: true }).descriptor
    .asset_geometry!;
  assert.equal(geometry.movement_transitions?.length ?? 0, 0);
  assert.deepEqual(
    geometry.sight_obstacles.map((obstacle) => obstacle.initial_active ?? true),
    [true, false],
  );
});

test("unavailable mask control keeps the initial mask and reindexes the remaining switch", async () => {
  const { document, assets } = unavailableMaskControlCompilerFixture();
  assert.throws(() => compileMap(document, [0, 0, 2000, 2000], assets), /waypoint must resolve/);
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets, { bestEffort: true });
  const geometry = compiled.descriptor.asset_geometry!;
  assert.equal(geometry.masks!.length, 3);
  assert.equal(geometry.movement_transitions!.length, 1);
  assert.deepEqual(geometry.movement_transitions![0]!.initial_masks, [1]);
  assert.deepEqual(geometry.movement_transitions![0]!.applied_masks, [2]);
  assert.equal(geometry.doors!.length, 2);
  assert.deepEqual(geometry.movement_transitions![0]!.door_links, {
    mode: "trigger-transition",
    indices: [1],
  });
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-unavailable-mask-control.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compiled.descriptor, fixture);
});

test("asset mask geometry exports the native state fixture and survives ZIP packaging", async () => {
  const { document, assets } = maskAssetCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL("../../../crates/robin_engine/tests/fixtures/asset-mask.level.json", import.meta.url),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
  const compiled = compileMap(document, [0, 0, 512, 512], assets);
  const bytes = await packageCompiledMap(compiled, {
    color: new Uint8Array(512 * 512 * 4),
    depth: new Uint16Array(512 * 512),
  });
  const files = unzipSync(bytes);
  const packaged = JSON.parse(strFromU8(files[`Data/Levels/${compiled.name}.level.json`]!));
  assert.deepEqual(packaged.asset_geometry.masks, compiled.descriptor.asset_geometry!.masks);
  assert.deepEqual(
    packaged.asset_geometry.movement_transitions,
    compiled.descriptor.asset_geometry!.movement_transitions,
  );
});

test("door receiving anchors export the native endpoint fixture", async () => {
  const { document, assets } = doorAnchorCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-door-anchor.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("door transition export matches the native door wiring fixture", async () => {
  const { document, assets } = doorTransitionCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-door-transition.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("cross-asset jumps export the geometry verified by the native traversal fixture", async () => {
  const { document, assets } = crossAssetJumpCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL("../../../crates/robin_engine/tests/fixtures/asset-jump.level.json", import.meta.url),
      "utf8",
    ),
  );
  assert.deepEqual(
    compileMap(document, [0, 0, 2000, 2000], assets).descriptor.asset_geometry,
    fixture.asset_geometry,
  );
});

test("rearranged geometric jumps export the native traversal fixture", async () => {
  const { document, assets } = geometricJumpCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-jump-geometric.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("obstructed jumps export the clearance checked by native animation", async () => {
  const { document, assets } = obstructedJumpCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-jump-obstructed.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("movement-only assets trim generated approaches and reconnect when moved away", async () => {
  const { document, assets } = movementBlockedJumpCompilerFixture();
  const compile = () => compileMap(document, [0, 0, 2000, 2000], assets).descriptor;
  const blocked = compile();
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-jump-movement-blocked.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(blocked, expected);
  assert.equal(blocked.asset_geometry!.jump_line_pairs!.length, 2);
  document.groups.find((g) => g.id === "roof-exclusion")!.transform.dx += 500;
  const clear = compile();
  assert.equal(clear.asset_geometry!.jump_line_pairs!.length, 1);
  assert.deepEqual(clear.asset_geometry!.sight_obstacles, blocked.asset_geometry!.sight_obstacles);
});

test("switchable movement exclusions constrain permanent jump connections in both states", async () => {
  const fixed = movementBlockedJumpCompilerFixture();
  const permanent = compileMap(fixed.document, [0, 0, 2000, 2000], fixed.assets).descriptor
    .asset_geometry!.jump_line_pairs;
  const { document, assets } = changingJumpApproachCompilerFixture();
  const compile = () => compileMap(document, [0, 0, 2000, 2000], assets).descriptor;
  const compiled = compile();
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-jump-changing-approach.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compiled, expected);
  const gameplay = assets.get("roof-exclusion")!.gameplay!;
  const transition = gameplay.movementTransitions![0]!;
  assert.deepEqual(compiled.asset_geometry!.jump_line_pairs, permanent);
  transition.initial = transition.applied;
  transition.applied = [];
  assert.deepEqual(compile().asset_geometry!.jump_line_pairs, permanent);
});

test("adjusted roof contours export level edges checked by native flight", async () => {
  const { document, assets } = levelContourJumpCompilerFixture();
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets).descriptor;
  assert.equal(compiled.asset_geometry!.jump_line_pairs!.length, 1);
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-jump-level-contours.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compiled, fixture);
  for (const asset of assets.values())
    for (const surface of asset.gameplay?.surfaces ?? [])
      if (surface.jump) delete surface.jump.maxLevelAdjustment;
  assert.equal(
    compileMap(document, [0, 0, 2000, 2000], assets).descriptor.asset_geometry!.jump_line_pairs,
    undefined,
  );
});

test("low jump obstacles export spans cleared by sword-fighting flight too", async () => {
  const { document, assets } = swordJumpObstacleCompilerFixture();
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets).descriptor;
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-jump-sword.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compiled, fixture);
  assert.ok(
    compiled.asset_geometry!.warnings!.some((warning) => warning.includes("obstruct the flight")),
  );
});

test("new surface-derived courtyard exports native jump destinations", async () => {
  const { document, assets } = multiDestinationJumpCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-jump-courtyard.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("detached jump export keeps the complete copy with compact native zone references", async () => {
  const { document, assets } = detachedJumpCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-jump-detached.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("non-rendering gameplay export matches the native collision fixture", async () => {
  const { document, assets } = nonrenderingVolumeCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-nonrendering-volume.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("preserved boundary export matches the native thin corridor fixture", async () => {
  const { document, assets } = preservedBoundaryCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-preserved-boundary.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("preserved contour export matches the native overlap fixture", async () => {
  const { document, assets } = preservedContoursCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-preserved-contours.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("sloped terrain socket exports match the native actor traversal cases", async () => {
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-sloped-terrain-sockets.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const actual = [0, 45, 90, 180, 270].map((rotation) => {
    const { document, assets, route } = slopedTerrainSocketCompilerFixture(false, rotation);
    return {
      rotation,
      route,
      descriptor: compileMap(document, [0, 0, 2000, 2000], assets).descriptor,
    };
  });
  assert.deepEqual(actual, expected);
});

test("ordinary multi-plane export matches the native traversal fixture", async () => {
  const { document, assets } = multiPlaneRegionCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-multi-plane-region.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
  const joined = joinedNavigationCompilerFixture();
  assert.deepEqual(
    compileMap(joined.document, [0, 0, 2000, 2000], joined.assets).descriptor,
    fixture,
  );
});

test("compound lift export matches the native multi-plane traversal fixture", async () => {
  const { document, assets } = compoundLiftCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-compound-lift.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("navigation partition export matches the native gate fixture", async () => {
  const { document, assets } = navigationRegionCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-navigation-region.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("jump export matches the native traversal fixture", async () => {
  const { document, assets } = jumpAssetCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL("../../../crates/robin_engine/tests/fixtures/asset-jump.level.json", import.meta.url),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("traversal light export matches the native ambience fixture", async () => {
  const { document, assets } = liftLightCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-lift-light.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("light region export matches the native ambience and interior fixture", async () => {
  const { document, assets } = lightAssetCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-light.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("sight transition export matches native apply/reset fixture", async () => {
  const { document, assets } = sightTransitionCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-sight-transition.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("appearance-only export matches native apply/reset without fabricated gameplay changes", async () => {
  const { document, assets, hut } = appearanceOnlyCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-appearance-only.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets);
  assert.deepEqual(compiled.descriptor, fixture);
  const transition = compiled.descriptor.asset_geometry!.movement_transitions![0]!;
  assert.equal(transition.has_appearance, true);
  assert.deepEqual(transition.motion_changes, []);
  assert.equal(transition.door_links, undefined);
  assert.equal(transition.initial_sight, undefined);
  assert.equal(transition.initial_masks, undefined);
  delete hut.gameplay!.movementTransitions![0]!.appearances;
  assert.throws(
    () => compileMap(document, [0, 0, 2000, 2000], assets),
    /invalid movement transition/,
  );
});

test("terrain transition export matches native slope and upper-floor fixture", async () => {
  const { document, assets } = terrainTransitionCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-terrain-transition.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("unavailable terrain control exports loadable initial barriers without orphan state bits", async () => {
  const { document, assets } = unavailableTerrainControlCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-unavailable-terrain-control.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(
    compileMap(document, [0, 0, 2000, 2000], assets, { bestEffort: true }).descriptor,
    fixture,
  );
});

test("movement transition export matches native apply/reset fixture", async () => {
  const { document, assets } = movementTransitionCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-movement-transition.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("map ZIP includes paired appearance resources bound to compiled patch indices", async () => {
  const { document, assets } = appearanceOnlyCompilerFixture();
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets);
  const transition = compiled.descriptor.asset_geometry!.movement_transitions![0]!;
  assert.equal(transition.has_appearance, true);
  const pixels = {
    color: new Uint8Array(2000 * 2000 * 4).fill(255),
    depth: new Uint16Array(2000 * 2000).fill(10),
  };
  const files = unzipSync(
    await packageCompiledMap(compiled, pixels, [
      {
        bounds: [1, 1, 1, 1],
        patches: [transition.id],
        states: [
          { color: Uint8Array.of(255, 255, 255, 255), depth: Uint16Array.of(10) },
          { color: Uint8Array.of(255, 0, 0, 255), depth: Uint16Array.of(40000) },
        ],
      },
    ]),
  );
  const prefix = `Data/Levels/Day/${compiled.name}`;
  const manifest = JSON.parse(strFromU8(files[`${prefix}.appearance.json`]!));
  assert.deepEqual(manifest.regions[0].patches, [0]);
  assert.equal(manifest.regions[0].states[0], null);
  const state = manifest.regions[0].states[1];
  assert.deepEqual([...decode(files[state.color]!).data], [255, 0, 0, 255]);
  assert.deepEqual([...decode(files[state.depth]!).data], [40000]);
});

test("asset environmental sound export matches the native source fixture", async () => {
  const { document, assets } = soundAssetCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-sound.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("receiving volume export matches the native state fixture", async () => {
  const { document, assets } = projectionVolumeCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-projection-volume.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("receiving material export matches the native lookup fixture", async () => {
  const { document, assets } = projectionMaterialCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-projection-material.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("asset material export matches the native lookup fixture", async () => {
  const { document, assets } = materialAssetCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-material.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("asset-local movement clearance export matches the native navigation fixture", async () => {
  const { document, assets } = clearanceAssetCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-clearance.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

export function bakeFixture(): Level3D {
  return {
    version: 1,
    map: "Bake Contract",
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    sceneAssets: [],
    size: [128, 128],
    groups: [{ id: "house", transform: { dx: 5, dy: 10, dz: 0, rot_deg: 0 } }],
    objects: [
      {
        id: "wall",
        node: "wall",
        kind: "building",
        source: { map: "Bake Contract", obstacle: 0 },
        group: "house",
        transform: { dx: 3, dy: 2, dz: 0, rot_deg: 0 },
        obstacle: {
          points: [
            [0, 0],
            [20, 0],
            [20, 20],
            [0, 20],
          ].map(([x, y]) => ({ x: x!, y: y!, z_bottom: 0, z_top: 30 })),
          opaque: true,
          solid: true,
          mouse: true,
          projection_area: null,
          show_shadow_polygon: false,
          default_material: 0,
          material_indices: [],
        },
      },
    ],
  };
}

test("compilation rebases transformed volumes, namespaces output and preserves the document", () => {
  const document = bakeFixture(),
    before = structuredClone(document);
  const result = compileMap(document, [-20, -10, 128, 128]);
  assert.equal(result.name, "editor-bake-contract");
  assert.deepEqual(result.descriptor.volumes[0]!.footprint, [
    [28, 22],
    [48, 22],
    [48, 42],
    [28, 42],
  ]);
  assert.equal(result.descriptor.volumes[0]!.motion_blocking, true);
  assert.deepEqual(document, before);
  document.groups[0]!.hidden = true;
  assert.deepEqual(compileMap(document, [0, 0, 128, 128]).descriptor.volumes, []);
});

test("map export does not invent mission spawns, even for a tiny frame", () => {
  const result = compileMap(bakeFixture(), [0, 0, 8, 8]);
  assert.equal("spawn_player" in result.descriptor, false);
  assert.deepEqual(result.descriptor.spawn_points, []);
  assert.equal("spawn" in result.descriptor, false);
  assert.equal("reveal_all" in result.descriptor, false);
});

test("bounds round outward, reject unsafe sizes, and fit visible geometry only", () => {
  assert.deepEqual(validateBakeBounds([-0.5, -1.5, 100, 100]), [-1, -2, 101, 101]);
  for (const bounds of [
    [0, 0, 0, 1],
    [0, 0, 17000, 1],
    [0, 0, 16384, 16384],
    [NaN, 0, 1, 1],
  ])
    assert.throws(() => validateBakeBounds(bounds as [number, number, number, number]));
  const camera = bakeFixture().camera;
  const root = new THREE.Group();
  const geometry = new THREE.BufferGeometry().setFromPoints(
    [
      [0, 0, 0],
      [100, 0, 0],
      [0, 100, 0],
    ].map((p) => new THREE.Vector3(...gameToScene(camera, p[0]!, p[1]!, p[2]!))),
  );
  root.add(new THREE.Mesh(geometry, new THREE.MeshBasicMaterial()));
  const hidden = new THREE.Mesh(new THREE.BoxGeometry(10000, 10000, 10000));
  hidden.visible = false;
  root.add(hidden);
  const bounds = contentBakeBounds(root, camera);
  assert.ok(bounds[2] <= 101 && bounds[3] <= 101);
  geometry.dispose();
  hidden.geometry.dispose();
});

test("bake snapshot resets patch previews without changing editor objects", () => {
  const root = new THREE.Group(),
    node = new THREE.Group();
  node.userData = { reveal_material_patch: "p", reveal_material_state: "covered" };
  node.visible = false;
  root.add(node);
  const snapshot = bakeScene([root]);
  assert.equal(snapshot.children[0]!.children[0]!.visible, true);
  assert.equal(node.visible, false);
});

test("bake snapshots select combined appearance states without leaking previews or depth exclusions", () => {
  const root = new THREE.Group();
  const covered = new THREE.Group();
  covered.userData = { reveal_material_patch: "roof", reveal_material_state: "covered" };
  const revealed = new THREE.Group();
  revealed.userData = { reveal_material_patch: "roof", reveal_material_state: "revealed" };
  revealed.visible = false;
  const receiver = new THREE.Group();
  receiver.userData = {
    reveal_show_when_applied: ["roof"],
    reveal_hide_when_applied: ["gate"],
    map_bake_object_id: "receiver",
  };
  receiver.visible = false;
  const hiddenParent = new THREE.Group();
  hiddenParent.visible = false;
  hiddenParent.add(revealed.clone());
  root.add(covered, revealed, receiver, hiddenParent);
  const peer = new THREE.Group();
  peer.userData = { reveal_hide_when_applied: ["roof"] };
  const snapshot = bakeScene([root, peer], new Set(["roof"]));
  const visibility = () => snapshot.children[0]!.children.map((node) => node.visible);
  assert.deepEqual(visibility(), [false, true, true, false]);
  assert.equal(snapshot.children[1]!.visible, false);
  withDepthOcclusion(snapshot, new Set(["receiver"]), () => {
    assert.deepEqual(visibility(), [false, true, false, false]);
  });
  assert.deepEqual(visibility(), [false, true, true, false]);
  const combined = bakeScene([root, peer], new Set(["roof", "gate"]));
  assert.deepEqual(
    combined.children[0]!.children.map((node) => node.visible),
    [false, true, false, false],
  );
  assert.deepEqual(
    root.children.map((node) => node.visible),
    [true, false, false, false],
  );
  assert.equal(peer.visible, true);
  assert.deepEqual(
    bakeScene([root]).children[0]!.children.map((node) => node.visible),
    [true, false, false, false],
  );
});

test("mask-owned parts retain color visibility but reveal underlying depth geometry", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  hut.gameplay!.maskOcclusionNodes = ["building-999"];
  const owner = document.objects.find((p) => p.node.endsWith(":building-999"))!;
  const excluded = maskOcclusionObjects(document, assets);
  assert.deepEqual([...excluded], [owner.id]);
  const root = new THREE.Group(),
    foreground = new THREE.Group(),
    background = new THREE.Group();
  foreground.userData.map_bake_object_id = owner.id;
  foreground.add(new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial()));
  background.add(new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial()));
  root.add(foreground, background);
  const snapshot = bakeScene([root]);
  const visibleMeshes = () => {
    let count = 0;
    snapshot.traverseVisible((n) => {
      if (n instanceof THREE.Mesh) count++;
    });
    return count;
  };
  assert.equal(visibleMeshes(), 2);
  withDepthOcclusion(snapshot, excluded, () => assert.equal(visibleMeshes(), 1));
  assert.equal(visibleMeshes(), 2);
  assert.equal(foreground.visible, true);
  assert.throws(
    () =>
      withDepthOcclusion(snapshot, excluded, () => {
        throw new Error("render failed");
      }),
    /render failed/,
  );
  assert.equal(visibleMeshes(), 2);
  compileMap(document, [0, 0, 2000, 2000], assets);
  hut.gameplay!.maskOcclusionNodes = ["missing"];
  assert.throws(() => compileMap(document, [0, 0, 2000, 2000], assets), /mask occlusion nodes/);
  hut.gameplay!.maskOcclusionNodes = ["building-999", "building-999"];
  assert.throws(() => compileMap(document, [0, 0, 2000, 2000], assets), /mask occlusion nodes/);
  hut.gameplay!.maskOcclusionNodes = ["building-999"];
  delete hut.gameplay!.masks;
  assert.throws(() => compileMap(document, [0, 0, 2000, 2000], assets), /mask occlusion nodes/);
  root.traverse((n) => {
    if (n instanceof THREE.Mesh) {
      n.geometry.dispose();
      (n.material as THREE.Material).dispose();
    }
  });
});

test("mod ZIP has root metadata, a playable descriptor and lossless 16-bit depth", async () => {
  const document = bakeFixture();
  document.notes = "Unsaved authoring notes";
  document.exportBounds = [-10, -20, 128, 128];
  const expectedDocument = structuredClone(document);
  const compiled = compileMap(document, [0, 0, 128, 128]);
  document.notes = "Edited after compilation";
  const color = new Uint8Array(128 * 128 * 4).fill(255);
  const depth = Uint16Array.from({ length: 128 * 128 }, (_, i) => i * 4);
  const bytes = await packageCompiledMap(compiled, { color, depth });
  const files = unzipSync(bytes),
    prefix = "Data/Levels/Day/editor-bake-contract";
  assert.deepEqual(JSON.parse(strFromU8(files["details.json"]!)).hackable_missions, [
    compiled.name,
  ]);
  assert.deepEqual(
    JSON.parse(strFromU8(files[`Data/Levels/${compiled.name}.level.json`]!)),
    compiled.descriptor,
  );
  const editable = JSON.parse(strFromU8(files[`editor/${compiled.name}.rhlos-map.json`]!));
  assert.deepEqual(parseStoredMap(editable, new Map()), expectedDocument);
  const decoded = decode(files[`${prefix}.occlusion-depth.png`]!);
  assert.equal(decoded.depth, 16);
  assert.equal(decoded.channels, 1);
  assert.deepEqual(decoded.data, depth);
  assert.equal(decode(files[`${prefix}.map.png`]!).width, 128);
  assert.equal(decode(files[`${prefix}.min.png`]!).width, 9);
  await assert.rejects(
    packageCompiledMap(compiled, { color: new Uint8Array(), depth }),
    /dimensions/,
  );
});

test("asset export retains a reopenable pinned scene and matches the Rust runtime fixture", async () => {
  const { document, assets } = assetCompilerFixture();
  const runtimeFixture = compileMap(document, [0, 0, 2000, 2000], assets);
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-compiled.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(runtimeFixture.descriptor, fixture);
  const expected = structuredClone(document);
  const compiled = compileMap(document, [0, 0, 512, 512], assets);
  const bytes = await packageCompiledMap(compiled, {
    color: new Uint8Array(512 * 512 * 4),
    depth: new Uint16Array(512 * 512),
  });
  const files = unzipSync(bytes);
  const reopened = parseStoredMap(
    JSON.parse(strFromU8(files[`editor/${compiled.name}.rhlos-map.json`]!)),
    assets,
  );
  assert.deepEqual(reopened, expected);
});

test("best effort ZIP retains omitted mission and wall authoring and reports missing gameplay", async () => {
  const { document, assets, hut } = assetCompilerFixture();
  delete hut.gameplay;
  document.splines = [
    {
      id: "wall",
      name: "Wall",
      kind: "wall",
      asset: "hut",
      axis: "x",
      points: [
        [0, 0, 0],
        [100, 0, 0],
      ],
      closed: false,
      width: 10,
      repeatLength: 20,
    },
  ];
  document.population = {
    version: 1,
    spriteCatalog: "catalog.json",
    actors: [],
    routes: [],
    items: [
      {
        id: "item",
        name: "Item",
        sprite: "apple",
        position: [10, 10, 0],
        quantity: 1,
        purpose: "Preview",
      },
    ],
  };
  const expected = structuredClone(document);
  const compiled = compileMap(document, [0, 0, 512, 512], assets, { bestEffort: true });
  assert.ok(compiled.warnings.some((message) => message.includes("Mission population omitted")));
  const files = unzipSync(
    await packageCompiledMap(compiled, {
      color: new Uint8Array(512 * 512 * 4),
      depth: new Uint16Array(512 * 512),
    }),
  );
  const reopened = parseStoredMap(
    JSON.parse(strFromU8(files[`editor/${compiled.name}.rhlos-map.json`]!)),
    assets,
  );
  assert.deepEqual(reopened, expected);
  assert.deepEqual(document, expected);
  assert.deepEqual(
    JSON.parse(strFromU8(files["compile-report.json"]!)).warnings,
    compiled.warnings,
  );
  assert.equal("spawn_player" in compiled.descriptor, false);
});

test("sloped asset export matches the native elevation/navigation fixture", async () => {
  const { document, assets } = slopedAssetCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-sloped.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("changing lift exports match native placed traversal fixtures", async () => {
  const fixtures = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-changing-lifts.levels.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  for (const [index, rotation] of [0, 90, 180, 270].entries()) {
    const { document, assets } = changingLiftCompilerFixture();
    document.groups[0]!.transform = { dx: 900, dy: 900, dz: 20, rot_deg: rotation };
    assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixtures[index]);
  }
});

test("lift asset export matches the native traversal fixture", async () => {
  const { document, assets } = liftAssetCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL("../../../crates/robin_engine/tests/fixtures/asset-lift.level.json", import.meta.url),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("physical stair landings retain fractional receiving boundaries at zero height", () => {
  const { document, assets, hut } = liftAssetCompilerFixture();
  for (const surface of hut.gameplay!.surfaces) surface.preserveMovementPrecision = true;
  document.groups[0]!.transform.dx += 0.25;
  const geometry = compileMap(document, [0, 0, 2000, 2000], assets).descriptor.asset_geometry!;
  const lift = geometry.lifts![0]!;
  assert.ok(lift.physical_navigation);
  const door = lift.doors[0]!;
  const receivers = geometry.sight_obstacles!.filter(
    (obstacle) =>
      obstacle.projection_area?.[0] === door.sector_out &&
      obstacle.projection_area[1] === door.layer_out,
  );
  assert.equal(receivers.length, 1);
  const receiver = receivers[0]!;
  assert.ok(receiver.points.every((point) => point.z_top === 0));
  assert.ok(
    receiver.points.some((point) => point.x === lift.physical_navigation!.doors[0]!.middle[0]),
  );
  assert.ok(receiver.points.some((point) => point.x % 1 === 0.25));
  assert.notDeepEqual(receiver.points[0], receiver.points.at(-1));
});

test("changing climb exports match native placed traversal fixtures", async () => {
  const fixtures = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-changing-climbs.levels.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  let index = 0;
  for (const [type, high] of [
    [2, 4],
    [3, 4],
    [3, 6],
  ] as const) {
    for (const rotation of [0, 90, 180, 270]) {
      const { document, assets } = changingClimbCompilerFixture(type, high);
      document.groups[0]!.transform = { dx: 900, dy: 900, dz: 20, rot_deg: rotation };
      assert.deepEqual(
        compileMap(document, [0, 0, 2000, 2000], assets).descriptor,
        fixtures[index++],
      );
    }
  }
});

test("copied changing stairs export matches the native isolation fixture", async () => {
  const { document, assets } = copiedChangingLiftCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-changing-lifts-copied.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("copied changing climbs export independent native state bindings", async () => {
  const fixtures = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-changing-climbs-copied.levels.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  for (const [index, [type, high]] of (
    [
      [2, 4],
      [3, 4],
      [3, 6],
    ] as const
  ).entries()) {
    const { document, assets } = changingClimbCompilerFixture(type, high, true);
    const compiled = compileMap(document, [0, 0, 2000, 2000], assets).descriptor;
    assert.deepEqual(compiled, fixtures[index]);
    assert.equal(compiled.asset_geometry!.movement_transitions!.length, 2);
    assert.equal(compiled.asset_geometry!.lifts!.length, 2);
  }
});

test("interior asset export matches the native building fixture", async () => {
  const { document, assets } = interiorAssetCompilerFixture();
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-interior.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
});

test("joined and separated asset interiors match native fixtures", async () => {
  const { document, assets } = joinedInteriorCompilerFixture();
  for (const state of ["joined", "separated"]) {
    if (state === "separated")
      document.objects.find((p) => p.id === "connector-body")!.transform.dx += 1;
    const fixture = JSON.parse(
      await readFile(
        new URL(
          `../../../crates/robin_engine/tests/fixtures/asset-interior-${state}.level.json`,
          import.meta.url,
        ),
        "utf8",
      ),
    );
    assert.deepEqual(compileMap(document, [0, 0, 2000, 2000], assets).descriptor, fixture);
  }
});

test("editor-connected rooms match the moved native fixture and remain in the editable zip", async () => {
  const { document, assets } = connectedInteriorCompilerFixture();
  document.objects.find((part) => part.group === "annex")!.transform.dx += 100;
  const compiled = compileMap(document, [0, 0, 1000, 1000], assets);
  const expected = JSON.parse(
    await readFile(
      new URL(
        "../../../crates/robin_engine/tests/fixtures/asset-interior-editor-linked.level.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(compiled.descriptor, expected);
  const files = unzipSync(
    await packageCompiledMap(compiled, {
      color: new Uint8Array(1000 * 1000 * 4),
      depth: new Uint16Array(1000 * 1000),
    }),
  );
  const restored = parseStoredMap(
    JSON.parse(strFromU8(files[`editor/${compiled.name}.rhlos-map.json`]!)),
    assets,
  );
  assert.deepEqual(restored.interiorConnections, document.interiorConnections);
});

import test from "node:test";

import assert from "node:assert/strict";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import { validateAssetGameplay } from "./asset-gameplay.ts";
import { IDENTITY_TRANSFORM } from "./level3d.ts";
import {
  assetCompilerFixture,
  maskAssetCompilerFixture,
  anchoredReceiverCompilerFixture,
  slopedAssetCompilerFixture,
  liftAssetCompilerFixture,
  liftLightCompilerFixture,
  interiorAssetCompilerFixture,
  terrainInteriorCompilerFixture,
  terrainPassageCompilerFixture,
  joinedInteriorCompilerFixture,
  soundAssetCompilerFixture,
  movementTransitionCompilerFixture,
  sightTransitionCompilerFixture,
  lightAssetCompilerFixture,
  jumpAssetCompilerFixture,
  compoundLiftCompilerFixture,
  multiPlaneRegionCompilerFixture,
  joinedNavigationCompilerFixture,
  crossAssetJumpCompilerFixture,
  obstructedJumpCompilerFixture,
  surfaceJumpCompilerFixture,
  multiDestinationJumpCompilerFixture,
  doorTransitionCompilerFixture,
  doorAnchorCompilerFixture,
  projectionMaterialCompilerFixture,
  projectionVolumeCompilerFixture,
  receivingIslandCompilerFixture,
  preservedBoundaryCompilerFixture,
} from "../test-fixtures/asset-gameplay.ts";

import { heightPlane, planeHeight } from "./gameplay-plane.ts";

test("mask receiving segments bind slopes without moving their pixels or boundary rules", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  gameplay.movementTransitions = [];
  const baseline = compileAssetGameplay(document, assets, bounds).masks;
  for (const surface of gameplay.surfaces) surface.height = [0, 9, 9, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiving anchor/);
  for (const mask of gameplay.masks!)
    mask.receiverSegment = [
      [45, 45, -20],
      [45, 45, 20],
    ];
  const result = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    result.masks?.map((mask) => ({ ...mask, layer: 0 })),
    baseline,
  );
  assert.ok(result.masks?.every((mask) => result.motion_data.layers[mask.layer]?.length));
  // An authored finite reach cannot bind a far-away floor.
  for (const surface of gameplay.surfaces) surface.height = 30;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiving anchor/);
  const partial = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(partial.masks, undefined);
  assert.ok(
    partial.warnings?.some((warning) => warning.includes("sprite occlusion is incomplete")),
  );
});

test("bounded physical receivers bind terrain without flattening their geometry", () => {
  const { document, assets, hut } = anchoredReceiverCompilerFixture();
  const baseline = compileAssetGameplay(document, assets, bounds);
  const ground = assets.get("marker")!.gameplay!.surfaces[0]!;
  ground.height = [0, 7, 7, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /navigation anchor/);
  const receiver = hut.gameplay!.projectionReceivers![0]!;
  receiver.receiverSegment = [
    [50, 50, -8],
    [50, 50, 8],
  ];
  const result = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(result.sight_obstacles[0], baseline.sight_obstacles[0]);
  assert.equal(result.motion_data.layers.flat().length, 1);
  document.groups[0]!.transform.dx += 40;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.equal(moved.sight_obstacles[0]!.points.length, result.sight_obstacles[0]!.points.length);
  for (const [index, point] of moved.sight_obstacles[0]!.points.entries()) {
    const old = result.sight_obstacles[0]!.points[index]!;
    assert.ok(Math.abs(point.x - old.x - 40) < 1e-8);
    for (const key of ["y", "z_bottom", "z_top"] as const)
      assert.ok(Math.abs(point[key] - old[key]) < 1e-8);
  }
  assert.deepEqual(
    moved.sight_obstacles[0]!.projection_area,
    result.sight_obstacles[0]!.projection_area,
  );
  document.groups[0]!.transform.dx -= 40;
  ground.height = 20;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiving segment/);
  const partial = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(partial.sight_obstacles[0]!.projection_area, null);
  assert.ok(
    partial.warnings?.some((w) => w.startsWith("Receiver ") && w.includes("receiving segment")),
  );
  receiver.receiverSegment = [
    [50, 50, 0],
    [50, 50, 0],
  ];
  assert.throws(
    () => validateAssetGameplay(hut.gameplay, hut),
    /invalid projection receiving segment/,
  );
});

test("interior receiving segments move approach points onto sloped terrain", () => {
  const { document, assets, hut } = interiorAssetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  for (const surface of gameplay.surfaces) surface.height = [0, 9, 9, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /outside must resolve/);
  for (const door of gameplay.interiors![0]!.doors) {
    const [x, y, z] = door.outside;
    door.outsideReceiverSegment = [
      [x, y, z - 10],
      [x, y, z + 10],
    ];
  }
  const geometry = compileAssetGameplay(document, assets, bounds);
  const building = geometry.buildings!.find((b) => "Building" in b)!;
  assert.ok("Building" in building);
  assert.deepEqual(
    building.Building.doors.map((d) => d.point_out),
    [
      [320, 378],
      [380, 372],
    ],
  );
  assert.deepEqual(
    building.Building.doors.map((d) => d.point_in),
    [
      [320, 420],
      [380, 420],
    ],
  );
  for (const surface of gameplay.surfaces) surface.height = 30;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiving segment/);
  const partial = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.ok(partial.warnings?.some((w) => w.includes("Door") && w.includes("receiving segment")));
  assert.ok(!partial.buildings?.some((b) => "Building" in b));
});

test("ordinary passages bind both receiving endpoints and preserve lock transitions", () => {
  const { document, assets, hut } = terrainPassageCompilerFixture();
  const before = structuredClone(document);
  const result = compileAssetGameplay(document, assets, bounds);
  const door = result.doors[0]!;
  assert.deepEqual(door.point_out, [380, 348]);
  assert.deepEqual(door.point_in, [420, 344]);
  assert.equal(door.locked_pc, true);
  assert.equal(door.locked_pc_after_patch, false);
  assert.notEqual(door.sector_in, door.sector_out);
  document.groups[0]!.transform = { dx: 600, dy: 200, dz: 30, rot_deg: 90 };
  const moved = compileAssetGameplay(document, assets, bounds).doors[0]!;
  assert.notDeepEqual(moved.point_out, door.point_out);
  assert.notEqual(moved.sector_in, moved.sector_out);
  document.groups = before.groups;
  hut.gameplay!.surfaces[1]!.height = 30;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /inside: receiving segment/);
  const partial = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(partial.doors.length, 0);
  assert.match(partial.warnings!.join("\n"), /inside: receiving segment/);
});

test("passage receivers reject ambiguous layers and cannot override interior or lift destinations", () => {
  const { document, assets, hut } = terrainPassageCompilerFixture();
  const door = hut.gameplay!.doors[0]!;
  door.insideAnchor = door.inside;
  assert.throws(() => validateAssetGameplay(hut.gameplay, hut), /inside receiving segment/);
  delete door.insideAnchor;
  hut.gameplay!.surfaces.push({
    ...structuredClone(hut.gameplay!.surfaces[1]!),
    id: "stacked",
    height: 7,
  });
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /exactly one unblocked surface/,
  );
  const interior = interiorAssetCompilerFixture();
  interior.hut.gameplay!.interiors![0]!.doors[0]!.insideReceiverSegment =
    door.insideReceiverSegment;
  assert.throws(
    () => validateAssetGameplay(interior.hut.gameplay, interior.hut),
    /interior door inside receiving segment/,
  );
  const lift = liftAssetCompilerFixture();
  lift.hut.gameplay!.lifts![0]!.doors[0]!.insideReceiverSegment = door.insideReceiverSegment;
  assert.throws(
    () => validateAssetGameplay(lift.hut.gameplay, lift.hut),
    /lift door inside receiving segment/,
  );
});

test("cropping a passage's destination omits its connection even during strict export", () => {
  const { document, assets } = terrainPassageCompilerFixture();
  const result = compileAssetGameplay(document, assets, [0, 0, 400, 400]);
  assert.equal(result.doors.length, 0);
  assert.match(
    result.warnings!.join("\n"),
    /inside: receiving segment has no surface inside the export frame/,
  );
});

test("interior receiving segments reject blocked, stacked and incompatible attachments", () => {
  const { document, assets, hut } = terrainInteriorCompilerFixture();
  const gameplay = hut.gameplay!;
  const door = gameplay.interiors![0]!.doors[0]!;
  const segment = structuredClone(door.outsideReceiverSegment!);
  door.outsideReceiverSegment = [
    [45, 45, -10],
    [45, 45, 10],
  ];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /exactly one unblocked surface/,
  );
  door.outsideReceiverSegment = segment;
  gameplay.surfaces.push({ ...structuredClone(gameplay.surfaces[0]!), id: "upper", height: 9 });
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /exactly one unblocked surface/,
  );
  door.outsideAnchor = [...door.outside];
  assert.throws(
    () => validateAssetGameplay(gameplay, hut),
    /invalid interior door receiving segment/,
  );
  delete door.outsideAnchor;
  door.outsideReceiverSegment = [
    [20, 80, 0],
    [20, 80, 0],
  ];
  assert.throws(
    () => validateAssetGameplay(gameplay, hut),
    /invalid interior door receiving segment/,
  );
  door.outsideReceiverSegment = [
    [20, 80, NaN],
    [20, 80, 10],
  ];
  assert.throws(
    () => validateAssetGameplay(gameplay, hut),
    /invalid interior door receiving segment/,
  );
});

test("mask receiving segments reject stacked layers and malformed endpoints", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  gameplay.movementTransitions = [];
  for (const mask of gameplay.masks!)
    mask.receiverSegment = [
      [45, 45, -20],
      [45, 45, 20],
    ];
  gameplay.surfaces.push({ ...structuredClone(gameplay.surfaces[0]!), id: "upper", height: 10 });
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiving anchor.*found 2/);
  for (const value of [NaN, Infinity]) {
    gameplay.masks![0]!.receiverSegment![0][2] = value;
    assert.throws(() => validateAssetGameplay(gameplay, hut), /invalid mask receiving segment/);
  }
  gameplay.masks![0]!.receiverSegment = [
    [45, 45, 0],
    [45, 45, 0],
  ];
  assert.throws(() => validateAssetGameplay(gameplay, hut), /invalid mask receiving segment/);
  gameplay.masks![0]!.receiverSegment = [
    [45, 45, 0],
    [46, 45, 0],
  ];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /segment lies in a receiving plane/,
  );
  const partial = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.ok(
    partial.warnings?.some((warning) => warning.includes("segment lies in a receiving plane")),
  );
});

test("preserved boundaries retain crossing obstacle contours without rounding their intersections", () => {
  const { document, assets, hut } = preservedBoundaryCompilerFixture();
  const geometry = compileAssetGameplay(document, assets, bounds);
  assert.equal(geometry.motion_data.layers[0]!.length, 1);
  const area = geometry.motion_data.layers[0]![0]!;
  assert.deepEqual(area.polygon.points, [
    [300, 300],
    [400, 300],
    [400, 370],
  ]);
  assert.equal(area.obstacles.length, 1);
  assert.equal(area.obstacles[0]!.polygon.points.length, 4);
  assert.ok(area.obstacles[0]!.polygon.points.some(([x, y]) => x === 410 && y === 376));
  // Ordinary free-space clipping rounds the crossing at x=301.428... to 301.
  // Keep a control so this fixture continues to exercise a real difference.
  hut.gameplay!.surfaces[0]!.preserveMovementBoundary = false;
  const clipped = compileAssetGameplay(document, assets, bounds);
  assert.notDeepEqual(clipped.motion_data.layers[0], geometry.motion_data.layers[0]);
  hut.gameplay!.surfaces[0]!.preserveMovementBoundary = true;
  delete hut.gameplay!.surfaces[0]!.navigationRegion;
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /preserved movement boundaries/,
  );
});

test("asset contour labels preserve separate overlapping exclusions through placement", () => {
  const { document, assets, hut } = preservedBoundaryCompilerFixture();
  const surface = hut.gameplay!.surfaces[0]!;
  surface.polygon = [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  surface.holes = [
    [
      [0, 0],
      [100, 0],
      [0, 71],
    ],
  ];
  surface.holeContours = ["assembly/slope"];
  const wall = hut.gameplay!.movementBlockers![0]!;
  wall.polygon = [
    [40, -10],
    [60, -10],
    [60, 100],
    [40, 100],
  ];
  wall.movementContour = "assembly/wall";
  const area = compileAssetGameplay(document, assets, bounds).motion_data.layers[0]![0]!;
  assert.equal(area.obstacles.length, 2);
  assert.deepEqual(area.obstacles[0]!.polygon.points, [
    [300, 300],
    [400, 300],
    [300, 371],
  ]);
  assert.equal(area.obstacles[1]!.polygon.points.length, 4);
  surface.holeContours = [];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /hole contour labels/);
  surface.holeContours = ["assembly/slope"];
  wall.movementContour = "";
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /movement contour labels/);
});

test("visual component bounds can opt out of physical collision while retaining their frame", () => {
  const { document, assets, hut } = assetCompilerFixture();
  const part = hut.parts[0]!;
  const before = structuredClone(part.obstacle_local_game);
  part.collision = "none";
  hut.gameplay!.doors = [];
  const compiled = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]);
  assert.equal(compiled.sight_obstacles.length, 0);
  assert.deepEqual(part.obstacle_local_game, before);
  assert(document.objects.some((o) => o.obstacle));
  hut.gameplay!.movementSolids = [part.node];
  assert.throws(
    () => compileAssetGameplay(document, assets, [0, 0, 2000, 2000]),
    /permanent movement solid/,
  );
  delete hut.gameplay!.movementSolids;
  hut.gameplay!.surfaces[0]!.projectionVolume = part.node;
  assert.throws(
    () => compileAssetGameplay(document, assets, [0, 0, 2000, 2000]),
    /projection volume/,
  );
});

const bounds: [number, number, number, number] = [0, 0, 2000, 2000];
test("physical receiver anchors share ground navigation without cutting a separate walking area", () => {
  const { document, assets, hut } = anchoredReceiverCompilerFixture();
  const baseline = compileAssetGameplay(document, assets, bounds);
  assert.equal(baseline.motion_data.layers.flat().length, 1);
  assert.equal(baseline.motion_data.layers[0]![0]!.obstacles.length, 0);
  assert.deepEqual(baseline.sight_obstacles[0]!.projection_area, [0, 0]);
  document.objects[0]!.transform.dx += 50;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(moved.motion_data, baseline.motion_data);
  assert.deepEqual(moved.sight_obstacles[0]!.projection_area, [0, 0]);
  assert.equal(
    moved.sight_obstacles[0]!.points[0]!.x,
    baseline.sight_obstacles[0]!.points[0]!.x + 50,
  );
  const copy = structuredClone(document.objects[0]!);
  copy.id = "hut-copy";
  delete copy.group;
  copy.transform.dx += 50;
  document.objects.push(copy);
  const duplicated = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(duplicated.motion_data, baseline.motion_data);
  assert.equal(duplicated.sight_obstacles.length, 2);
  assert(duplicated.sight_obstacles.every((s) => JSON.stringify(s.projection_area) === "[0,0]"));
  document.objects.pop();
  document.objects[0]!.transform.rot_deg = 90;
  const rotated = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(rotated.motion_data, baseline.motion_data);
  assert.deepEqual(rotated.sight_obstacles[0]!.projection_area, [0, 0]);
  hut.gameplay!.projectionReceivers![0]!.anchor = [900, 900, 0];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /navigation anchor must resolve/,
  );
});

test("physical receiver anchors reject dangling and conflicting ownership", () => {
  const { document, assets, hut } = anchoredReceiverCompilerFixture();
  const receiver = hut.gameplay!.projectionReceivers![0]!;
  receiver.volume = "missing";
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /invalid projection receiver/,
  );
  receiver.volume = hut.parts[0]!.node;
  hut.gameplay!.projectionReceivers!.push({ ...receiver, id: "duplicate" });
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /invalid projection receiver/,
  );
});

test("feature anchors use the physical receiver's elevation within shared ground navigation", () => {
  const { document, assets, hut } = anchoredReceiverCompilerFixture();
  hut.gameplay!.doors = [
    {
      id: "slope-passage",
      node: hut.parts[0]!.node,
      polygon: [],
      outside: [10, 25, 5],
      inside: [90, 50, 45],
      middle: [50, 50, 25],
      type: 0,
      locked: false,
      unlockable: false,
      allowContinuous: true,
    },
  ];
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.equal(compiled.motion_data.layers.flat().length, 1);
  assert.equal(compiled.doors.length, 0);
  hut.gameplay!.doors[0]!.outside = [150, 25, 75];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /outside must resolve/);
});

test("receiving volumes retain thickness, materials and state links after placement", () => {
  const { document, assets } = projectionVolumeCompilerFixture();
  const compile = () => compileAssetGameplay(document, assets, bounds);
  const baseline = compile();
  const receiver = baseline.sight_obstacles[0]!;
  assert.deepEqual(receiver.projection_area, [1, 1]);
  assert.equal(receiver.solid, true);
  assert.equal(receiver.opaque, true);
  assert.equal(receiver.show_shadow_polygon, true);
  assert.equal(receiver.default_material, 2);
  assert.deepEqual(receiver.material_indices, [0]);
  assert.ok(
    receiver.points.every((p) => Math.fround(p.z_bottom) === 15 && Math.fround(p.z_top) === 20),
  );
  assert.equal(baseline.sight_obstacles.length, 2);
  assert.deepEqual(baseline.movement_transitions![0]!.applied_sight, [0]);
  const part = document.objects.find((p) => p.node.endsWith(":building-999"))!;
  part.transform.dx += 100;
  const moved = compile();
  const coordinates = (points: typeof receiver.points) =>
    points.map((p) => [p.x, p.y, p.z_bottom, p.z_top].map(Math.fround));
  assert.deepEqual(
    coordinates(moved.sight_obstacles[0]!.points),
    coordinates(receiver.points.map((p) => ({ ...p, x: p.x + 100 }))),
  );
  assert.deepEqual(moved.movement_transitions![0]!.applied_sight, [0]);
  part.transform.rot_deg = 90;
  const rotated = compile();
  assert.ok(
    rotated.sight_obstacles[0]!.points.every(
      (p) => Math.fround(p.z_bottom) === 15 && Math.fround(p.z_top) === 20,
    ),
  );
  assert.deepEqual(rotated.movement_transitions![0]!.applied_sight, [0]);
  const copy = structuredClone(part);
  copy.id = "projection-copy";
  copy.group = "projection-copy";
  copy.transform.dx += 600;
  document.groups.push({ id: "projection-copy", transform: { ...IDENTITY_TRANSFORM } });
  document.objects.push(copy);
  const duplicated = compile();
  const transitions = duplicated.movement_transitions!;
  assert.deepEqual(
    transitions.map((t) => t.applied_sight),
    [[0], [1]],
  );
  const receivers = transitions.map((t) => duplicated.sight_obstacles[t.applied_sight![0]!]!);
  assert.notDeepEqual(receivers[0]!.projection_area, receivers[1]!.projection_area);
  assert.notDeepEqual(receivers[0]!.material_indices, receivers[1]!.material_indices);
});

test("receiving ownership excludes a separate island inside a navigation hole", () => {
  const { document, assets } = receivingIslandCompilerFixture();
  const geometry = compileAssetGameplay(document, assets, bounds);
  const physical = geometry.sight_obstacles[0]!;
  const islandReceiver = geometry.sight_obstacles.find((o) => o.default_material === 4)!;
  assert.notDeepEqual(physical.projection_area, islandReceiver.projection_area);
  assert.equal(geometry.sight_obstacles.length, 2);
  assert.deepEqual(geometry.movement_transitions![0]!.applied_sight, [0]);
});

test("physical receiving planes use the first three vertices without flattening later heights", () => {
  const { document, assets, hut } = projectionVolumeCompilerFixture();
  const points = hut.gameplay!.volumes![0]!.shape.points;
  points[3]!.z_top = 22;
  const geometry = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    geometry.sight_obstacles[0]!.points.map((p) => Math.fround(p.z_top)),
    [20, 20, 20, 22],
  );
  hut.gameplay!.surfaces[0]!.height = 22;
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /top must lie on the surface/,
  );
  hut.gameplay!.surfaces[0]!.height = 20;
  points[2] = { ...points[1]! };
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /no nondegenerate height plane/,
  );
});

test("receiving part links reuse physical geometry and preserve state references", () => {
  const { document, assets, hut } = projectionVolumeCompilerFixture();
  const gameplay = hut.gameplay!;
  const volume = gameplay.volumes![0]!;
  const part = hut.parts.find((p) => p.node === volume.node)!;
  part.obstacle_local_game = { ...volume.shape, projection_area: null, material_indices: [] };
  gameplay.collision = "parts";
  gameplay.surfaces[0]!.projectionVolume = part.node;
  gameplay.materials![0]!.obstacles = [part.node];
  gameplay.movementTransitions![0]!.appliedSight = [part.node];
  delete gameplay.volumes;
  // Navigation and receiving footprints are independent: an uncovered navigation
  // margin must not invent additional receiving geometry.
  gameplay.surfaces[0]!.polygon[0]![0] -= 1;
  const result = compileAssetGameplay(document, assets, bounds);
  assert.equal(result.sight_obstacles.length, 2);
  assert.deepEqual(result.movement_transitions![0]!.applied_sight, [0]);
  assert.deepEqual(result.sight_obstacles[0]!.projection_area, [1, 1]);
  assert.equal(Math.min(...result.sight_obstacles[0]!.points.map((p) => p.x)), 300);
  delete gameplay.movementTransitions;
  const instance = document.objects.find((p) => p.node.endsWith(":building-999"))!;
  const visible = structuredClone(instance);
  visible.id = "receiver-frame";
  visible.node = "asset:hut:visible-frame";
  hut.parts.push({ node: "visible-frame", name: "Visible frame", scenery: true });
  document.objects.push(visible);
  instance.hidden = true;
  assert.deepEqual(
    compileAssetGameplay(document, assets, bounds).sight_obstacles[0]!.projection_area,
    [1, 1],
  );
  gameplay.collision = "none";
  gameplay.materials![0]!.obstacles = [];
  gameplay.materials![0]!.ground = true;
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /sight obstacle|projection volume/,
  );
});

test("receiving volume links reject missing, conflicting and disjoint definitions", () => {
  const { document, assets, hut } = projectionVolumeCompilerFixture();
  const surface = hut.gameplay!.surfaces[0]!;
  surface.projectionVolume = "missing";
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid projection volume/);
  surface.projectionVolume = "platform-volume";
  surface.projectionMaterials = { defaultMaterial: 2, regions: [] };
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid projection volume/);
  delete surface.projectionMaterials;
  surface.height = 21;
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /top must lie on the surface/,
  );
  surface.height = 20;
  surface.polygon = surface.polygon.map(([x, y]) => [x + 200, y]);
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /does not overlap/);
  surface.polygon = surface.polygon.map(([x, y]) => [x - 200, y]);
  const adjacent = hut.gameplay!.surfaces[1]!;
  const saved = structuredClone(adjacent.polygon);
  adjacent.polygon = structuredClone(surface.polygon);
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /physical and generated receivers/,
  );
  adjacent.polygon = saved;
  const other = structuredClone(surface);
  other.id = "other-area";
  surface.navigationRegion = "one";
  other.navigationRegion = "two";
  hut.gameplay!.surfaces.push(other);
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /multiple receiving areas/);
});

test("independent interiors join through a placed passage and separate when it moves", () => {
  const { document, assets, passage } = joinedInteriorCompilerFixture();
  const joined = compileAssetGameplay(document, assets, bounds);
  assert.equal(joined.buildings!.length, 1);
  const doors = joined.buildings![0]!.Building.doors;
  assert.equal(doors.length, 2);
  assert.equal(doors[0]!.sector_in, doors[1]!.sector_in);
  assert.notEqual(doors[0]!.locked_pc, doors[1]!.locked_pc);
  const part = document.objects.find((p) => p.id === "connector-body")!;
  part.transform.dx += 1;
  const separated = compileAssetGameplay(document, assets, bounds);
  assert.equal(separated.buildings!.length, 2);
  assert.ok(separated.buildings!.every((b) => b.Building.doors.length === 1));
  assert.notEqual(
    separated.buildings![0]!.Building.doors[0]!.sector_in,
    separated.buildings![1]!.Building.doors[0]!.sector_in,
  );
  part.transform.dx -= 1;
  part.transform.rot_deg = 90;
  assert.equal(compileAssetGameplay(document, assets, bounds).buildings!.length, 2);
  part.transform.rot_deg = 0;
  document.objects.find((p) => p.id === "annex-body")!.transform.dx += 100;
  const relocated = compileAssetGameplay(document, assets, bounds);
  assert.equal(relocated.buildings!.length, 2);
  assert.ok(
    relocated.buildings!.some((building) =>
      building.Building.doors.some((door) => door.point_out[0] === 780 && door.point_in[0] === 780),
    ),
  );
  passage.gameplay!.interiors![0]!.joins![0]!.direction = [0, 0];
  assert.throws(() => validateAssetGameplay(passage.gameplay, passage), /invalid interior joins/);
});

test("joined interior assemblies rotate and duplicate without sharing rooms between copies", () => {
  const { document, assets } = joinedInteriorCompilerFixture();
  const copies = document.objects
    .filter((p) => p.kind === "building")
    .map((part) => {
      const copy = structuredClone(part);
      copy.id += "-copy";
      copy.group = "assembly-copy";
      return copy;
    });
  document.groups.push({
    id: "assembly-copy",
    transform: {
      dx: 1000,
      dy: 0,
      dz: 40,
      rot_deg: 90,
    },
  });
  document.objects.push(...copies);
  const result = compileAssetGameplay(document, assets, bounds);
  assert.equal(result.buildings!.length, 2);
  assert.ok(result.buildings!.every((b) => b.Building.doors.length === 2));
  const first = result.buildings![0]!.Building.doors;
  const second = result.buildings![1]!.Building.doors;
  assert.notEqual(first[0]!.sector_in, second[0]!.sector_in);
  assert.equal(second[0]!.sector_in, second[1]!.sector_in);
});

test("joined rooms retain each asset's door-transition binding", () => {
  const { document, assets, annex } = joinedInteriorCompilerFixture();
  const transition = structuredClone(
    doorTransitionCompilerFixture().hut.gameplay!.movementTransitions![1]!,
  );
  transition.doorLinks!.ids = [annex.gameplay!.interiors![0]!.doors[0]!.id];
  annex.gameplay!.movementTransitions = [transition];
  const joined = compileAssetGameplay(document, assets, bounds);
  const linkedDoor = (result: typeof joined) => {
    const link = result.movement_transitions![0]!.door_links!;
    assert.equal(link.mode, "swap-rights");
    assert.equal(link.indices.length, 1);
    return result.buildings!.flatMap((b) => b.Building.doors)[link.indices[0]!]!;
  };
  assert.deepEqual(linkedDoor(joined).point_out, [680, 380]);
  document.objects.find((p) => p.id === "connector-body")!.transform.dx += 1;
  const separated = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(linkedDoor(separated).point_out, [680, 380]);
  assert.equal(separated.buildings!.length, 2);
});

test("transition receiving anchors preserve reference points outside their linked surfaces", () => {
  const { document, assets, hut } = doorTransitionCompilerFixture();
  const transition = hut.gameplay!.movementTransitions![0]!;
  const before = compileAssetGameplay(document, assets, bounds).movement_transitions![0]!;
  transition.waypointAnchor = [...transition.waypoint];
  transition.waypoint = [95, 50, 0];
  const result = compileAssetGameplay(document, assets, bounds).movement_transitions![0]!;
  assert.deepEqual(result.waypoint, [395, 350]);
  assert.equal(result.sector, before.sector);
  assert.equal(result.layer, before.layer);
  transition.waypointAnchor = [500, 500, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /waypoint must resolve/);
  transition.waypointAnchor = [NaN, 0, 0];
  assert.throws(() => validateAssetGameplay(hut.gameplay, hut), /invalid movement transition/);
});

test("receiving anchors preserve door coordinates outside the receiving polygons", () => {
  const { document, assets, hut } = doorAnchorCompilerFixture();
  const compiled = compileAssetGameplay(document, assets, bounds);
  const door = compiled.doors[0]!;
  assert.deepEqual(door.point_out, [395, 350]);
  assert.deepEqual(door.point_in, [405, 350]);
  assert.notEqual(door.sector_out, door.sector_in);
  const copy = structuredClone(document.objects[0]!);
  copy.id = "copy";
  copy.group = "copy";
  copy.transform = { ...IDENTITY_TRANSFORM };
  document.objects.push(copy);
  document.groups.push({ id: "copy", transform: { dx: 1000, dy: 700, dz: 30, rot_deg: 90 } });
  const duplicated = compileAssetGameplay(document, assets, bounds);
  assert.equal(duplicated.doors.length, 2);
  assert.notEqual(duplicated.doors[1]!.sector_out, duplicated.doors[0]!.sector_out);
  assert.notEqual(duplicated.doors[1]!.sector_in, duplicated.doors[0]!.sector_in);
  const rotated = duplicated.doors.find((door) => door.point_out[0] > 500)!;
  assert.equal(rotated.point_in[0], rotated.point_out[0]);
  assert.equal(rotated.point_in[1] - rotated.point_out[1], 5);
  document.objects.pop();
  document.groups.pop();
  hut.gameplay!.doors[0]!.insideAnchor = [500, 500, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /inside must resolve/);
  hut.gameplay!.doors[0]!.insideAnchor = [45, 45, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /inside must resolve/);
});

test("receiving anchors validate coordinates and cannot replace a virtual interior", () => {
  const { hut } = doorAnchorCompilerFixture();
  hut.gameplay!.doors[0]!.insideAnchor = [NaN, 0, 0];
  assert.throws(() => validateAssetGameplay(hut.gameplay, hut), /insideAnchor/);
  const interior = interiorAssetCompilerFixture();
  interior.hut.gameplay!.interiors![0]!.doors[0]!.insideAnchor = [0, 0, 0];
  assert.throws(() => validateAssetGameplay(interior.hut.gameplay, interior.hut), /shared room/);
});

test("optional unrestricted passages disappear only after their walkable areas join", () => {
  const { document, assets, hut } = assetCompilerFixture();
  const door = hut.gameplay!.doors[0]!;
  door.polygon = [];
  door.allowContinuous = true;
  const separated = compileAssetGameplay(document, assets, bounds);
  assert.equal(separated.doors.length, 1);
  hut.gameplay!.surfaces.push({
    id: "connector",
    node: door.node,
    height: 0,
    polygon: [
      [80, 0],
      [120, 0],
      [120, 100],
      [80, 100],
    ],
  });
  const joined = compileAssetGameplay(document, assets, bounds);
  assert.equal(joined.doors.length, 0);
  assert.ok(joined.warnings?.some((w) => w.includes("omitted unrestricted passage")));
  door.allowContinuous = false;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /distinct motion areas/);
  door.allowContinuous = true;
  for (const key of ["locked", "unlockable", "lockedVillains", "lockedCivilians"] as const) {
    door[key] = true;
    assert.throws(() => compileAssetGameplay(document, assets, bounds), /cannot allow continuous/);
    door[key] = false;
  }
  door.active = false;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /cannot allow continuous/);
  door.active = true;
  door.afterTransition = {
    locked: true,
    unlockable: false,
    lockedVillains: false,
    lockedCivilians: false,
  };
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /cannot allow continuous/);
  delete door.afterTransition;
  door.polygon = [
    [90, 40],
    [110, 40],
    [110, 60],
    [90, 60],
  ];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /cannot allow continuous/);
});

test("omitting a redundant passage preserves remaining native door bindings", () => {
  const { document, assets, hut } = doorTransitionCompilerFixture();
  const original = compileAssetGameplay(document, assets, bounds);
  hut.gameplay!.doors.unshift({
    id: "redundant",
    node: "building-999",
    type: 0,
    polygon: [],
    outside: [10, 10, 0],
    inside: [20, 20, 0],
    middle: [15, 15, 0],
    locked: false,
    unlockable: false,
    allowContinuous: true,
  });
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(compiled.doors, original.doors);
  assert.deepEqual(compiled.buildings, original.buildings);
  assert.deepEqual(compiled.movement_transitions, original.movement_transitions);
  hut.gameplay!.movementTransitions![0]!.doorLinks!.ids = ["redundant"];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /distinct motion areas/);
});

test("door-only transitions resolve native interior-first indices independently for each placement", () => {
  const { document, assets, hut } = doorTransitionCompilerFixture();
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    compiled.movement_transitions!.map((t) => t.door_links),
    [
      { mode: "trigger-transition", indices: [2] },
      { mode: "swap-rights", indices: [0, 1] },
    ],
  );
  const part = structuredClone(document.objects.find((p) => p.group === "hut-a")!);
  part.id = "copy";
  part.group = "hut-b";
  part.transform.dx += 300;
  document.objects.push(part);
  document.groups.push({
    ...structuredClone(document.groups.find((g) => g.id === "hut-a")!),
    id: "hut-b",
  });
  const duplicate = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    duplicate.movement_transitions!.map((t) => t.door_links),
    [
      { mode: "trigger-transition", indices: [4] },
      { mode: "swap-rights", indices: [0, 1] },
      { mode: "trigger-transition", indices: [5] },
      { mode: "swap-rights", indices: [2, 3] },
    ],
  );
  hut.gameplay!.movementTransitions![0]!.doorLinks!.ids = ["missing"];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /missing ordinary\/interior door/,
  );
  hut.gameplay!.movementTransitions![0]!.doorLinks!.ids = ["passage"];
  hut.gameplay!.movementTransitions![1]!.doorLinks = {
    mode: "trigger-transition",
    ids: ["passage"],
  };
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /triggers multiple transitions/,
  );
});
test("sight transitions rebuild local references and reject ambiguous obstacle control", () => {
  const { document, assets, hut } = sightTransitionCompilerFixture();
  const geometry = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(geometry.movement_transitions![0]!.initial_sight, [0]);
  assert.deepEqual(geometry.movement_transitions![0]!.applied_sight, [1]);
  const transition = hut.gameplay!.movementTransitions![0]!;
  transition.initial = [];
  transition.applied = [];
  assert.deepEqual(
    compileAssetGameplay(document, assets, bounds).movement_transitions![0]!.motion_changes,
    [],
  );
  transition.appliedSight = ["building-999"];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /multiply controlled/);
  transition.appliedSight = ["missing"];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /sight obstacle/);
  transition.appliedSight = ["open-barrier"];
  delete hut.gameplay!.movementBlockers;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /explicit movement blockers/);
});
test("hidden mesh frames retain explicit gameplay and only referenced sight geometry", () => {
  const { document, assets, hut } = sightTransitionCompilerFixture();
  const part = document.objects.find((p) => p.node.endsWith(":building-999"))!;
  const visible = structuredClone(part);
  visible.id = "visible-anchor";
  visible.node = visible.node.replace("building-999", "anchor");
  hut.parts.push({ node: "anchor", name: "Visible frame", scenery: true });
  document.objects.push(visible);
  part.hidden = true;
  let geometry = compileAssetGameplay(document, assets, bounds);
  assert.equal(geometry.doors.length, 1);
  assert.equal(geometry.sight_obstacles.length, 2);
  delete hut.gameplay!.movementTransitions![0]!.initialSight;
  geometry = compileAssetGameplay(document, assets, bounds);
  assert.equal(geometry.sight_obstacles.length, 1);
  assert.deepEqual(geometry.movement_transitions![0]!.applied_sight, [0]);
});
for (const kind of ["part", "volume"] as const)
  test(`selected permanent ${kind} collision retains its clearances independently of changing sight`, () => {
    const { document, assets, hut } = sightTransitionCompilerFixture();
    const gameplay = hut.gameplay!;
    delete gameplay.movementBlockers;
    const fixed = structuredClone(hut.parts[0]!.obstacle_local_game!);
    fixed.points = fixed.points.map((p) => ({ ...p, y: p.y + 30 }));
    let ref: string;
    if (kind === "part") {
      ref = "building-998";
      hut.parts.push({
        node: ref,
        name: "Permanent wall",
        source_obstacle: 998,
        obstacle_local_game: fixed,
      });
      const placed = structuredClone(document.objects.find((p) => p.group === "hut-a")!);
      placed.id = "fixed-wall";
      placed.node = `asset:hut:${ref}`;
      placed.obstacle = fixed;
      document.objects.push(placed);
    } else {
      ref = "fixed-wall";
      const { projection_area: _projection, material_indices: _materials, ...shape } = fixed;
      gameplay.volumes!.push({ id: ref, node: "building-999", shape });
    }
    gameplay.movementSolids = [ref];
    gameplay.movementClearances = [
      {
        id: "opening",
        node: "building-999",
        height: 0,
        polygon: [
          [40, 70],
          [45, 70],
          [45, 80],
          [40, 80],
        ],
      },
    ];
    const compiled = compileAssetGameplay(document, assets, bounds);
    const permanent = compiled.motion_data.layers.flatMap((layer) =>
      layer.flatMap((area) => area.obstacles.filter((o) => o.state_id === 0)),
    );
    assert.equal(permanent.length, 1);
    assert.deepEqual(permanent[0]!.polygon.points.map((p) => p.join(",")).sort(), [
      "345,370",
      "345,380",
      "350,370",
      "350,380",
    ]);
    assert.equal(compiled.sight_obstacles.length, 3);
    assert.equal(compiled.movement_transitions!.length, 1);
    for (const part of document.objects) part.transform.dx += 100;
    const moved = compileAssetGameplay(document, assets, bounds);
    const after = moved.motion_data.layers.flatMap((layer) =>
      layer.flatMap((area) => area.obstacles.filter((o) => o.state_id === 0)),
    );
    assert.deepEqual(
      after[0]!.polygon.points,
      permanent[0]!.polygon.points.map(([x, y]) => [x + 100, y]),
    );
    gameplay.movementSolids = ["absent"];
    assert.throws(
      () => compileAssetGameplay(document, assets, bounds),
      /invalid permanent movement solid/,
    );
    gameplay.movementSolids = [ref, ref];
    assert.throws(
      () => compileAssetGameplay(document, assets, bounds),
      /invalid permanent movement solids/,
    );
  });
test("duplicated sight transitions control only their own transformed obstacles", () => {
  const { document, assets } = sightTransitionCompilerFixture();
  const part = document.objects.find((p) => p.group)!;
  document.groups.push({
    id: "state-copy",
    transform: { ...IDENTITY_TRANSFORM, dx: 1000, rot_deg: 90 },
  });
  document.objects.push({ ...structuredClone(part), id: "state-copy-part", group: "state-copy" });
  const geometry = compileAssetGameplay(document, assets, bounds);
  const [first, second] = geometry.movement_transitions!;
  assert.equal(geometry.movement_transitions!.length, 2);
  assert.deepEqual(first!.initial_sight, [0]);
  assert.deepEqual(second!.initial_sight, [2]);
  assert.deepEqual(second!.applied_sight, [3]);
  assert.notDeepEqual(geometry.sight_obstacles[0]!.points, geometry.sight_obstacles[2]!.points);
});
test("cross-asset jumps detach and reconnect with independently placed assets", () => {
  const { document, assets } = crossAssetJumpCompilerFixture();
  const whole = jumpAssetCompilerFixture();
  assert.deepEqual(
    compileAssetGameplay(document, assets, bounds),
    compileAssetGameplay(whole.document, whole.assets, bounds),
  );
  document.groups.find((g) => g.id === "jump-upper")!.transform.dx = 20;
  const detached = compileAssetGameplay(document, assets, bounds);
  assert.equal(detached.jump_line_pairs, undefined);
  assert.equal(detached.jump_zones, undefined);
  assert.equal(
    detached.warnings!.filter((warning) => warning.includes("connection is unavailable")).length,
    2,
  );
  assert.ok(detached.motion_data.layers.length > 0);
  document.groups.find((g) => g.id === "jump-upper")!.transform.dx = 0;
  assert.deepEqual(
    compileAssetGameplay(document, assets, bounds),
    compileAssetGameplay(whole.document, whole.assets, bounds),
  );
  for (const group of document.groups.slice())
    document.groups.push({
      id: `${group.id}-copy`,
      transform: { ...IDENTITY_TRANSFORM, dx: 1000, dy: 100, rot_deg: 90 },
    });
  for (const part of [...document.objects].filter((p) => p.group))
    document.objects.push({
      ...structuredClone(part),
      id: `${part.id}-copy`,
      group: `${part.group}-copy`,
    });
  const copies = compileAssetGameplay(document, assets, bounds);
  assert.equal(copies.jump_line_pairs!.length, 2);
  assert.equal(copies.jump_zones!.length, 4);
  assert.notEqual(
    copies.jump_line_pairs![0]!.line1.jump_zone_index,
    copies.jump_line_pairs![1]!.line1.jump_zone_index,
  );
  document.groups.find((g) => g.id === "jump-upper")!.transform.dx = 20;
  const partial = compileAssetGameplay(document, assets, bounds);
  assert.equal(partial.jump_line_pairs!.length, 1);
  assert.equal(partial.jump_zones!.length, 2);
  const pair = partial.jump_line_pairs![0]!;
  assert.deepEqual(
    [pair.line1.jump_zone_index, pair.line2.jump_zone_index].sort((a, b) => a - b),
    [0, 1],
  );
  assert.notDeepEqual(
    pair.line1.point_a,
    compileAssetGameplay(whole.document, whole.assets, bounds).jump_line_pairs![0]!.line1.point_a,
  );
  assert.ok(
    copies.jump_line_pairs!.some((copy) =>
      copy.line1.point_a.every((value, i) => value === pair.line1.point_a[i]),
    ),
  );
});

test("walkways and roof jumps reconnect to replacement assets without original neighbor identities", () => {
  for (const kind of ["walkway", "jump"] as const) {
    const fixture =
      kind === "walkway" ? joinedNavigationCompilerFixture() : crossAssetJumpCompilerFixture();
    const { document, assets, upper } = fixture;
    const original = compileAssetGameplay(document, assets, bounds);
    const part = document.objects.find((p) => p.node.startsWith(`asset:${upper.id}:`))!;
    const group = document.groups.find((g) => g.id === part.group)!;
    group.transform.dx += 30;
    const detached = compileAssetGameplay(document, assets, bounds);
    if (kind === "jump") assert.equal(detached.jump_line_pairs, undefined);
    else assert.equal(detached.motion_data.layers.flat().length, 2);
    const replacement = structuredClone(upper);
    replacement.id = "newly-authored-replacement";
    replacement.source_map = "unrelated-authoring-provenance";
    assets.set(replacement.id, replacement);
    document.assetSources!.push({
      ...document.assetSources!.find((ref) => ref.id === upper.id)!,
      id: replacement.id,
    });
    const replacementPart = structuredClone(part);
    replacementPart.id = "new-neighbor-body";
    replacementPart.group = "new-neighbor";
    replacementPart.node = part.node.replace(`asset:${upper.id}:`, `asset:${replacement.id}:`);
    document.objects.push(replacementPart);
    document.groups.push({ id: "new-neighbor", transform: { ...IDENTITY_TRANSFORM } });
    const rebuilt = compileAssetGameplay(document, assets, bounds);
    if (kind === "jump") {
      assert.deepEqual(rebuilt.jump_line_pairs, original.jump_line_pairs);
      assert.equal(rebuilt.jump_zones!.length, 2);
    } else {
      // The detached old neighbor remains separate; the replacement joins the walkway.
      assert.equal(rebuilt.motion_data.layers.flat().length, 2);
      assert.equal(rebuilt.warnings!.filter((w) => w.includes("no matching boundary")).length, 1);
    }
  }
});
test("surface rules construct jump edges and landing zones without recovered jump metadata", () => {
  const { document, assets, hut, upper } = surfaceJumpCompilerFixture();
  assert.equal(hut.gameplay!.jumpSegments!.length, 0);
  assert.equal(upper.gameplay!.jumpZones!.length, 0);
  const generated = compileAssetGameplay(document, assets, bounds);
  assert.equal(generated.jump_line_pairs!.length, 1);
  assert.equal(generated.jump_zones!.length, 2);
  const west = hut.gameplay!.surfaces.find((surface) => surface.id === "west")!;
  west.jump!.maxGap = 1;
  assert.equal(compileAssetGameplay(document, assets, bounds).jump_line_pairs, undefined);
  west.jump!.edges = [999];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid surface jump rules/);
});

test("one generated ledge connects to multiple separately placed roofs", () => {
  const { document, assets } = multiDestinationJumpCompilerFixture();
  const geometry = compileAssetGameplay(document, assets, bounds);
  assert.equal(geometry.jump_line_pairs!.length, 2);
  assert.equal(geometry.jump_zones!.length, 4);
  const spans = geometry
    .jump_line_pairs!.map(({ line1 }) => [
      Math.min(line1.point_a[1], line1.point_b[1]),
      Math.max(line1.point_a[1], line1.point_b[1]),
    ])
    .sort((a, b) => a[0]! - b[0]!);
  assert.ok(spans[0]![1]! < spans[1]![0]!);
  document.groups.find((group) => group.id === "jump-upper-second")!.transform.dx = 100;
  assert.equal(compileAssetGameplay(document, assets, bounds).jump_line_pairs!.length, 1);
});

test("surface-generated courtyard connections survive a rotated, elevated duplicate", () => {
  const { document, assets, hut, upper } = multiDestinationJumpCompilerFixture();
  hut.gameplay!.surfaces.find((surface) => surface.id === "west")!.height = 40;
  upper.gameplay!.surfaces.find((surface) => surface.id === "east")!.height = 40;
  for (const group of document.groups.filter((group) => group.id.startsWith("jump-upper")))
    group.transform.dy -= 100;
  const angle = (37 * Math.PI) / 180;
  const sinView = Math.sin((document.camera.elevation_deg * Math.PI) / 180);
  const originals = [...document.groups];
  for (const group of originals) {
    const t = group.transform;
    document.groups.push({
      ...structuredClone(group),
      id: `${group.id}-copy`,
      transform: {
        dx: 1000 + t.dx * Math.cos(angle) - (t.dy / sinView) * Math.sin(angle),
        dy: 300 + (t.dx * Math.sin(angle) + (t.dy / sinView) * Math.cos(angle)) * sinView,
        dz: t.dz + 30,
        rot_deg: 37,
      },
    });
  }
  for (const part of [...document.objects].filter((part) => part.group))
    document.objects.push({
      ...structuredClone(part),
      id: `${part.id}-copy`,
      group: `${part.group}-copy`,
    });
  const geometry = compileAssetGameplay(document, assets, bounds);
  assert.equal(geometry.jump_line_pairs!.length, 4, JSON.stringify(geometry.warnings ?? []));
  for (const { line1, line2 } of geometry.jump_line_pairs!) {
    assert.deepEqual(
      [line1.point_b[0] - line1.point_a[0], line1.point_b[1] - line1.point_a[1]],
      [line2.point_a[0] - line2.point_b[0], line2.point_a[1] - line2.point_b[1]],
    );
    assert.equal(line1.point_a[2], line2.point_a[2]);
    assert.ok(Math.abs(line1.point_a[0] - line2.point_b[0]) < 100);
  }
});

test("moving a separate wall rebuilds the usable jump span", () => {
  const { document, assets } = obstructedJumpCompilerFixture();
  const blocked = compileAssetGameplay(document, assets, bounds);
  assert.equal(blocked.jump_line_pairs!.length, 1);
  const span = (geometry: typeof blocked) => {
    const line = geometry.jump_line_pairs![0]!.line1;
    return Math.hypot(line.point_b[0] - line.point_a[0], line.point_b[1] - line.point_a[1]);
  };
  assert.equal(span(blocked), 18);
  assert.match(blocked.warnings!.join("\n"), /solid obstacles obstruct/);
  document.groups.find((group) => group.id === "jump-wall")!.transform.dx = 100;
  const clear = compileAssetGameplay(document, assets, bounds);
  assert.equal(span(clear), 30);
  assert.ok(!clear.warnings?.some((warning) => warning.includes("solid obstacles obstruct")));
});

test("geometric jump rules connect rearranged assets and preserve receiving zones", () => {
  const { document, assets, hut, upper } = crossAssetJumpCompilerFixture();
  const low = hut.gameplay!.jumpSegments![0]!,
    high = upper.gameplay!.jumpSegments![0]!;
  low.attachment = high.attachment = { maxGap: 35, maxRise: 110, maxDrop: 110, minOverlap: 10 };
  delete low.join;
  delete high.join;
  for (const segment of [low, high])
    [segment.edge.a, segment.edge.b] = [segment.edge.b, segment.edge.a];
  const group = document.groups.find((g) => g.id === "jump-upper")!;
  group.transform.dy = 100;
  group.transform.dx = 1;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.equal(moved.jump_line_pairs!.length, 1);
  assert.equal(moved.jump_zones!.length, 2);
  assert.ok(!moved.warnings?.some((w) => w.includes("connection is unavailable")));
  group.transform.dx = 10;
  assert.equal(compileAssetGameplay(document, assets, bounds).jump_line_pairs, undefined);
  low.attachment.maxGap = -1;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid jump attachment/);
});

test("detached edges do not remove landing zones used by another complete jump", () => {
  const { document, assets, hut } = jumpAssetCompilerFixture();
  const expected = compileAssetGameplay(document, assets, bounds);
  const pair = hut.gameplay!.jumpPairs![0]!;
  hut.gameplay!.jumpSegments = [
    {
      id: "other-edge",
      node: pair.node,
      long: pair.long,
      join: [999, 999, 999],
      edge: structuredClone(pair.edges[0]),
    },
  ];
  const actual = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(actual.jump_zones, expected.jump_zones);
  assert.deepEqual(actual.jump_line_pairs, expected.jump_line_pairs);
  assert.equal(
    actual.warnings!.filter((warning) => warning.includes("connection is unavailable")).length,
    1,
  );
});
test("preview bounds are not collision, while separately authored gameplay remains usable", () => {
  const { document, assets, hut } = assetCompilerFixture();
  const part = hut.parts[0]!;
  const obstacle = part.obstacle_local_game!;
  hut.parts = [
    {
      node: part.node,
      name: part.name,
      mission_profile: "preview-only",
      obstacle_local_game: obstacle,
    },
  ];
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.equal(compiled.sight_obstacles.filter((o) => o.projection_area === null).length, 0);
  assert.equal(compiled.doors.length, 1, "authored passage remains available");
  hut.gameplay!.movementSolids = [part.node];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /preview bounds require/);
  delete hut.gameplay!.movementSolids;
  const { projection_area: _projection, material_indices: _materials, ...shape } = obstacle;
  hut.gameplay!.volumes = [{ id: "authored-wall", node: part.node, shape }];
  const authored = compileAssetGameplay(document, assets, bounds);
  assert.equal(authored.sight_obstacles.filter((o) => o.projection_area === null).length, 1);
});

test("non-rendering asset volumes preserve collision and sight without a mesh part", () => {
  const { hut, document, assets } = assetCompilerFixture();
  const expected = compileAssetGameplay(document, assets, bounds);
  const {
    projection_area: _projection,
    material_indices: _materials,
    ...shape
  } = hut.parts[0]!.obstacle_local_game!;
  hut.gameplay!.collision = "none";
  hut.gameplay!.volumes = [
    { id: "invisible-wall", node: "building-999", shape: structuredClone(shape) },
  ];
  const normalized = (value: unknown) =>
    JSON.parse(
      JSON.stringify(value, (_, v) => (typeof v === "number" ? Math.round(v * 1e8) / 1e8 : v)),
    );
  assert.deepEqual(
    normalized(compileAssetGameplay(document, assets, bounds)),
    normalized(expected),
  );
  document.groups[0]!.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.equal(
    moved.sight_obstacles[0]!.points[0]!.x,
    expected.sight_obstacles[0]!.points[0]!.x + 100,
  );
  hut.gameplay!.materials = [
    {
      id: "wall-material",
      node: "building-999",
      material: 2,
      ground: false,
      obstacles: ["invisible-wall"],
      polygon: [
        [40, 40, 0],
        [50, 40, 0],
        [50, 50, 0],
        [40, 50, 0],
      ],
    },
  ];
  assert.deepEqual(
    compileAssetGameplay(document, assets, bounds).sight_obstacles[0]!.material_indices,
    [0],
  );
  hut.gameplay!.volumes[0]!.node = "missing";
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /unknown gameplay node/);
  hut.gameplay!.volumes[0]!.node = "building-999";
  Object.assign(hut.gameplay!.volumes[0]!.shape, { projection_area: [123, 1] });
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid gameplay volume/);
});
test("separate navigation assets reproduce one continuous region and detach after movement", () => {
  const { document, assets, hut } = joinedNavigationCompilerFixture();
  const local = multiPlaneRegionCompilerFixture();
  const joined = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(joined, compileAssetGameplay(local.document, local.assets, bounds));
  document.groups.find((g) => g.id === "upper")!.transform.dx = 20;
  const detached = compileAssetGameplay(document, assets, bounds);
  assert.equal(detached.motion_data.layers.flat().length, 2);
  assert.equal(detached.warnings!.filter((w) => w.includes("no matching boundary")).length, 2);
  document.groups.find((g) => g.id === "upper")!.transform.dx = 0;
  hut.gameplay!.surfaces[0]!.navigationJoins![0]![0][2] += 1;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /outer surface edge/);
  delete hut.gameplay!.surfaces[0]!.navigationRegion;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /navigation joins/);
});

test("explicit height steps join projected navigation boundaries and reject invalid tolerances", () => {
  const { document, assets, hut, upper } = joinedNavigationCompilerFixture();
  const surface = upper.gameplay!.surfaces[0]!;
  surface.polygon = surface.polygon.map(([x, y]) => [x, y + 1]);
  surface.height =
    typeof surface.height === "number" ? surface.height + 1 : surface.height.map((z) => z + 1);
  for (const edge of surface.navigationJoins!)
    for (const point of edge) {
      point[1] += 1;
      point[2] += 1;
    }
  assert.equal(compileAssetGameplay(document, assets, bounds).motion_data.layers.flat().length, 2);
  surface.navigationJoinHeightTolerance = 1.01;
  hut.gameplay!.surfaces[0]!.navigationJoinHeightTolerance = 1.01;
  assert.equal(compileAssetGameplay(document, assets, bounds).motion_data.layers.flat().length, 1);
  document.groups.find((g) => g.id === "upper")!.transform.dx = 10;
  assert.equal(compileAssetGameplay(document, assets, bounds).motion_data.layers.flat().length, 2);
  for (const invalid of [-1, Infinity, NaN]) {
    surface.navigationJoinHeightTolerance = invalid;
    assert.throws(() => compileAssetGameplay(document, assets, bounds), /height tolerance/);
  }
});

test("separate navigation assemblies rotate and duplicate without joining unrelated copies", () => {
  const { document, assets } = joinedNavigationCompilerFixture();
  const groups = [...document.groups],
    parts = [...document.objects];
  for (const group of groups)
    document.groups.push({
      id: `${group.id}-copy`,
      transform: { ...IDENTITY_TRANSFORM, dx: 1000, dy: 100, rot_deg: 90 },
    });
  for (const part of parts.filter((p) => p.group))
    document.objects.push({
      ...structuredClone(part),
      id: `${part.id}-copy`,
      group: `${part.group}-copy`,
    });
  const result = compileAssetGameplay(document, assets, bounds);
  assert.equal(result.motion_data.layers.flat().length, 2);
  assert.equal(result.sight_obstacles.length, 4);
  assert.equal((result.warnings ?? []).filter((w) => w.includes("no matching boundary")).length, 0);
  const bindings = result.sight_obstacles.map((s) => JSON.stringify(s.projection_area));
  assert.equal(new Set(bindings).size, 2);
});

test("ordinary local navigation regions join height planes without lift behavior", () => {
  const { document, assets, hut } = multiPlaneRegionCompilerFixture();
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.equal(compiled.motion_data.layers.flat().length, 1);
  assert.equal(compiled.motion_data.layers.length, 2);
  const projections = compiled.sight_obstacles.filter((s) => Array.isArray(s.projection_area));
  assert.equal(projections.length, 2);
  assert.deepEqual(projections[0]!.projection_area, projections[1]!.projection_area);
  assert.equal(compiled.lifts?.length ?? 0, 0);
  hut.gameplay!.surfaces[1]!.navigationRegion = "separate";
  assert.equal(compileAssetGameplay(document, assets, bounds).motion_data.layers.flat().length, 2);
});

test("multi-plane regions retain independent sectors after rotation and duplication", () => {
  const { document, assets } = multiPlaneRegionCompilerFixture();
  const part = document.objects.find((p) => p.group)!;
  document.groups.push({
    id: "roof-copy",
    transform: { ...IDENTITY_TRANSFORM, dx: 1000, rot_deg: 90 },
  });
  document.objects.push({ ...structuredClone(part), id: "roof-copy-part", group: "roof-copy" });
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.equal(compiled.motion_data.layers.flat().length, 2);
  const regions = new Map<string, number>();
  for (const surface of compiled.sight_obstacles) {
    const key = JSON.stringify(surface.projection_area);
    regions.set(key, (regions.get(key) ?? 0) + 1);
  }
  assert.deepEqual([...regions.values()], [2, 2]);
});

test("joined lift assets retain multiple height planes in one traversal sector", () => {
  const { document, assets, upper } = compoundLiftCompilerFixture();
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.equal(compiled.lifts!.length, 1);
  const lift = compiled.lifts![0]!;
  assert.equal(lift.doors.length, 2);
  assert.equal(compiled.motion_data.layers.at(-1)!.length, 1);
  const projections = compiled.sight_obstacles.filter(
    (s) => Array.isArray(s.projection_area) && s.projection_area[0] === lift.motion_area_index,
  );
  assert.equal(projections.length, 2);
  assert.deepEqual(
    lift.doors.map((d) => d.sector_in),
    [lift.motion_area_index, lift.motion_area_index],
  );
  document.groups.find((g) => g.id === "upper")!.transform.dx = 10;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /join must match/);
  document.groups.find((g) => g.id === "upper")!.transform.dx = 0;
  upper.gameplay!.lifts![0]!.type = 2;
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /type and direction disagree/,
  );
});
test("compound lifts rotate and duplicate with independent geometric joins", () => {
  const { document, assets } = compoundLiftCompilerFixture();
  const groups = [...document.groups];
  const parts = [...document.objects];
  for (const group of groups) {
    document.groups.push({
      id: `${group.id}-copy`,
      transform: { ...IDENTITY_TRANSFORM, dx: 1000, dy: 100, rot_deg: 90 },
    });
  }
  for (const part of parts.filter((p) => p.group)) {
    document.objects.push({
      ...structuredClone(part),
      id: `${part.id}-copy`,
      group: `${part.group}-copy`,
    });
  }
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.equal(compiled.lifts!.length, 2);
  assert.notEqual(compiled.lifts![0]!.motion_area_index, compiled.lifts![1]!.motion_area_index);
  assert.equal(compiled.lifts![1]!.doors.length, 2);
  assert.notEqual(compiled.lifts![0]!.direction, compiled.lifts![1]!.direction);
});

test("asset-local navigation regions preserve gates between touching coplanar rooms", () => {
  const { hut, document, assets } = assetCompilerFixture();
  const [west, east] = hut.gameplay!.surfaces;
  west!.polygon = [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  east!.polygon = [
    [100, 0],
    [200, 0],
    [200, 100],
    [100, 100],
  ];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /distinct motion areas/);
  west!.navigationRegion = "west";
  east!.navigationRegion = "east";
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.equal(compiled.motion_data.layers[0]!.length, 2);
  assert.notEqual(compiled.doors[0]!.sector_in, compiled.doors[0]!.sector_out);
  east!.navigationRegion = "west";
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /distinct motion areas/);
  east!.navigationRegion = "";
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /navigation regions/);
});

test("navigation region labels belong to each placement independently", () => {
  const { hut, document, assets } = assetCompilerFixture();
  hut.gameplay!.doors = [];
  hut.gameplay!.surfaces = [hut.gameplay!.surfaces[0]!];
  const surface = hut.gameplay!.surfaces[0]!;
  surface.navigationRegion = "room";
  const copy = structuredClone(document.objects[0]!);
  copy.id = "copy";
  copy.group = "copy";
  document.objects.push(copy);
  document.groups.push({ id: "copy", transform: { ...IDENTITY_TRANSFORM, dx: 90 } });
  assert.equal(compileAssetGameplay(document, assets, bounds).motion_data.layers[0]!.length, 2);
  delete surface.navigationRegion;
  assert.equal(compileAssetGameplay(document, assets, bounds).motion_data.layers[0]!.length, 1);
});

test("jump pairs rebuild crossed destination links and preserve height after duplication", () => {
  const { hut, document, assets } = jumpAssetCompilerFixture();
  const first = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    first.jump_zones!.map((z) => [z.sector, z.layer]),
    [
      [0, 0],
      [2, 1],
    ],
  );
  assert.deepEqual(first.jump_line_pairs![0], {
    line1: { point_a: [385, 330, 0], point_b: [385, 370, 0], jump_zone_index: 1 },
    line2: { point_a: [415, 270, 100], point_b: [415, 230, 100], jump_zone_index: 0 },
    jump_long: true,
  });
  const body = structuredClone(document.objects.find((p) => p.group === "hut-a")!);
  body.id = "jump-copy";
  body.group = "jump-copy";
  document.objects.push(body);
  document.groups.push({
    id: "jump-copy",
    transform: { ...IDENTITY_TRANSFORM, dx: 700, dy: 500, rot_deg: 90 },
  });
  const copied = compileAssetGameplay(document, assets, bounds);
  assert.equal(copied.jump_zones!.length, 4);
  assert.equal(copied.jump_line_pairs![1]!.line1.jump_zone_index, 3);
  assert.equal(copied.jump_line_pairs![1]!.line2.jump_zone_index, 2);
  assert.notDeepEqual(
    copied.jump_line_pairs![1]!.line1.point_a,
    first.jump_line_pairs![0]!.line1.point_a,
  );
  hut.gameplay!.jumpPairs![0]!.edges[0].a[2] = 20;
  assert.equal(
    compileAssetGameplay(document, assets, bounds).jump_line_pairs![0]!.line1.point_a[2],
    20,
  );
});
test("jump metadata rejects orphan zones, missing links and collapsed edges", () => {
  const { hut, document, assets } = jumpAssetCompilerFixture();
  const pair = hut.gameplay!.jumpPairs![0]!;
  pair.edges[0].zone = "missing";
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /missing zone/);
  pair.edges[0].zone = "low-zone";
  pair.edges[0].b = [...pair.edges[0].a];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /collapses/);
  hut.gameplay!.jumpPairs = [];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /no paired edge/);
});
test("light regions follow placement and preserve ambience without shifting interior links", () => {
  const { hut, document, assets } = lightAssetCompilerFixture();
  const first = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    first.light_sectors?.map((l) => [l.layer, l.ambience]),
    [
      [0, 1],
      [0, 2],
    ],
  );
  assert.deepEqual(first.light_sectors![0]!.polygon.points[0], [310, 310]);
  document.groups[0]!.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(moved.light_sectors![0]!.polygon.points[0], [410, 310]);
  assert.equal(
    moved.buildings![0]!.Building.doors[0]!.sector_in,
    first.buildings![0]!.Building.doors[0]!.sector_in,
  );
  hut.gameplay!.lights![0]!.ambiences = -1;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid light region/);
});

test("light receiver anchors preserve one contour across elevations and move with the asset", () => {
  const { hut, document, assets } = lightAssetCompilerFixture();
  hut.gameplay!.surfaces.push({
    id: "upper-light-receiver",
    node: "building-999",
    height: 40,
    polygon: [
      [0, 40],
      [90, 40],
      [90, 140],
      [0, 140],
    ],
  });
  const light = hut.gameplay!.lights![0]!;
  light.receivers = [
    [20, 20, 0],
    [20, 60, 40],
    [25, 65, 40],
  ];
  const first = compileAssetGameplay(document, assets, bounds).light_sectors!.filter(
    (l) => l.ambience === 1,
  );
  assert.equal(first.length, 2);
  assert.notEqual(first[0]!.layer, first[1]!.layer);
  assert.deepEqual(first[0]!.polygon, first[1]!.polygon);
  document.groups[0]!.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds).light_sectors!.filter(
    (l) => l.ambience === 1,
  );
  assert.deepEqual(
    moved.map((l) => l.polygon.points),
    first.map((l) => l.polygon.points.map(([x, y]) => [x + 100, y])),
  );
  light.receivers = [[20, 70, 50]];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /walkable surface/);
  light.receivers = [[1000, 1000, 0]];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /outside the light contour/);
  light.receivers = [];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid light receivers/);
});

test("light layer anchors retain fractional positions inside narrow contours and surfaces", () => {
  const { hut, document, assets } = lightAssetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  gameplay.interiors = [];
  gameplay.surfaces = [
    {
      ...gameplay.surfaces[0]!,
      polygon: [
        [10, 10],
        [20, 12],
        [20, 13],
      ],
    },
  ];
  gameplay.lights = [
    {
      id: "narrow",
      node: "building-999",
      ambiences: 1,
      polygon: [
        [10, 10, 0],
        [20, 12, 0],
        [20, 13, 0],
      ],
      receivers: [[15.51, 11.11, 0]],
    },
  ];
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.equal(compiled.light_sectors!.length, 1);
  assert.deepEqual(compiled.light_sectors![0]!.polygon.points, [
    [310, 310],
    [320, 312],
    [320, 313],
  ]);
  gameplay.lights[0]!.receivers = [[16, 11, 0]];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /outside the light contour/);
});

test("anchored lights cannot spill onto a separate coplanar navigation region", () => {
  const { hut, document, assets } = lightAssetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  gameplay.interiors = [];
  gameplay.surfaces = [10, 40].map((x) => ({
    id: `receiver-${x}`,
    node: "building-999",
    height: 0,
    polygon: [
      [x, 10],
      [x + 20, 10],
      [x + 20, 30],
      [x, 30],
    ],
  }));
  const polygon: [number, number, number][] = [
    [0, 0, 0],
    [70, 0, 0],
    [70, 40, 0],
    [0, 40, 0],
  ];
  gameplay.lights = [
    { id: "left-only", node: "building-999", ambiences: 1, polygon, receivers: [[20, 20, 0]] },
    { id: "both", node: "building-999", ambiences: 2, polygon },
  ];
  for (const dx of [0, 100]) {
    document.groups[0]!.transform.dx += dx;
    const compiled = compileAssetGameplay(document, assets, bounds);
    const ordinary = compiled.motion_data.layers.slice(0, -1);
    assert.deepEqual(
      ordinary.map((layer) => layer.length),
      [1, 1],
    );
    const left = ordinary.findIndex((layer) =>
      layer[0]!.polygon.points.some(([x]) => x === 310 + dx),
    );
    assert.ok(left >= 0);
    const isolated = compiled.light_sectors!.filter((light) => light.ambience === 1);
    assert.deepEqual(
      isolated.map((light) => light.layer),
      [left],
    );
    assert.deepEqual(
      compiled.light_sectors!.filter((light) => light.ambience === 2).map((light) => light.layer),
      [0, 1],
    );
    assert.deepEqual(
      isolated[0]!.polygon.points,
      polygon.map(([x, y, z]) => [x + 300 + dx, y - z + 300]),
    );
  }
});

test("light regions resolve on sloped traversal areas and follow their asset", () => {
  const { document, assets } = liftLightCompilerFixture();
  const geometry = compileAssetGameplay(document, assets, bounds);
  const light = geometry.light_sectors![0]!;
  assert.equal(light.layer, geometry.motion_data.layers.length - 1);
  assert.equal(light.ambience, 2);
  for (const part of document.objects) part.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    moved.light_sectors![0]!.polygon.points,
    light.polygon.points.map(([x, y]) => [x + 100, y]),
  );
  assert.deepEqual(
    moved.lifts!.map((lift) => lift.motion_area_index),
    geometry.lifts!.map((lift) => lift.motion_area_index),
  );
});

test("light segments resolve after placement and refuse missing or ambiguous receivers", () => {
  const { hut, document, assets } = lightAssetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  gameplay.interiors = [];
  gameplay.lights = [
    {
      id: "segment",
      node: "building-999",
      ambiences: 1,
      polygon: [
        [10, 10, 0],
        [40, 10, 0],
        [40, 40, 0],
        [10, 40, 0],
      ],
      receiverSegments: [
        [
          [20, 10, -10],
          [20, 30, 10],
        ],
      ],
    },
  ];
  const first = compileAssetGameplay(document, assets, bounds);
  assert.equal(first.light_sectors!.length, 1);
  document.groups[0]!.transform.dx += 1;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    moved.light_sectors![0]!.polygon.points,
    first.light_sectors![0]!.polygon.points.map(([x, y]) => [x + 1, y]),
  );
  gameplay.surfaces.push({ ...gameplay.surfaces[0]!, id: "upper", height: 5 });
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /exactly one walkable surface/,
  );
  gameplay.surfaces.pop();
  gameplay.lights[0]!.receiverSegments = [
    [
      [20, 40, 20],
      [20, 50, 30],
    ],
  ];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /exactly one walkable surface/,
  );
  gameplay.lights[0]!.receiverSegments = [
    [
      [20, 40, 20],
      [20, 40, 20],
    ],
  ];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /invalid light receiving segments/,
  );
});

test("light receiving planes resolve after elevation and reject absent or nonplanar surfaces", () => {
  const { hut, document, assets } = slopedAssetCompilerFixture();
  hut.gameplay!.lights = [
    {
      id: "ramp-shadow",
      node: "building-999",
      ambiences: 0xffffffff,
      polygon: [
        [10, 10, 5],
        [30, 10, 15],
        [30, 30, 15],
        [10, 30, 5],
      ],
    },
  ];
  const first = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(first.light_sectors![0]!.polygon.points[0], [310, 305]);
  const light = hut.gameplay!.lights[0]!;
  light.polygon = light.polygon.map(([x, y, z]) => [x, y, z + 100]);
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiving layer/);
  light.polygon[0]![2] += 1;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /must be planar/);
});

test("movement transitions receive fresh bindings across separate areas and duplicated assets", () => {
  const { document, assets } = movementTransitionCompilerFixture();
  const first = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(first.movement_transitions![0]!.motion_changes, [
    { layer: 0, sector: 0, changing_obstacle: 0 },
    { layer: 0, sector: 2, changing_obstacle: 0 },
  ]);
  assert.deepEqual(
    first.motion_data.layers[0]!.map((a) => a.obstacles.map((o) => o.state_id)),
    [[1], [2]],
  );
  const body = structuredClone(document.objects.find((p) => p.group === "hut-a")!);
  body.id = "hut-b-body";
  body.group = "hut-b";
  document.objects.push(body);
  document.groups.push({
    ...structuredClone(document.groups.find((g) => g.id === "hut-a")!),
    id: "hut-b",
  });
  const doubled = compileAssetGameplay(document, assets, bounds);
  assert.equal(doubled.movement_transitions!.length, 2);
  assert.deepEqual(
    doubled.motion_data.layers[0]!.map((a) => a.obstacles.map((o) => o.state_id)),
    [
      [1, 4],
      [2, 8],
    ],
  );
  for (const p of document.objects) p.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(moved.movement_transitions![0]!.waypoint, [420, 320]);
  assert.equal(moved.motion_data.layers[0]![0]!.obstacles[0]!.polygon.points[0]![0], 445);
});
test("transition reference points may be blocked but must still resolve to a surface", () => {
  const { document, assets, hut } = movementTransitionCompilerFixture();
  hut.gameplay!.movementBlockers = [
    {
      id: "fixed-wall",
      node: "building-999",
      height: 0,
      polygon: [
        [10, 10],
        [30, 10],
        [30, 30],
        [10, 30],
      ],
    },
  ];
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(compiled.movement_transitions![0]!.waypoint, [320, 320]);
  const transition = hut.gameplay!.movementTransitions![0]!;
  const door = hut.gameplay!.doors[0]!;
  const outside = door.outside;
  door.outside = [...transition.waypoint];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /unblocked walkable surface/);
  door.outside = outside;
  transition.waypoint = [20, 20, 100];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /waypoint must resolve/);
});
test("sound geometry follows placement while acoustic categories and falloff remain intact", () => {
  const { document, assets, hut } = soundAssetCompilerFixture();
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(compiled.sound_sources![0]!.polyline, [
    [310, 315],
    [330, 335],
  ]);
  assert.equal(compiled.sound_sources![0]!.altitude, 1);
  assert.deepEqual(compiled.sound_sources![0]!.delayed_params, [100, 200, 4]);
  assert.equal(compiled.sound_sources![1]!.global, true);
  assert.equal(compiled.sound_sources![1]!.polyline, null);
  for (const p of document.objects) p.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(moved.sound_sources![0]!.polyline, [
    [410, 315],
    [430, 335],
  ]);
  assert.equal(moved.sound_sources![0]!.noise_covering_distance, 60);
  hut.gameplay!.sounds![0]!.delay![2] = 65535;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid sound delay/);
  hut.gameplay!.sounds![0]!.delay![2] = 4;
  hut.gameplay!.sounds![0]!.spatial!.innerVolume = 101;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid sound geometry/);
  hut.gameplay!.sounds![0]!.spatial!.innerVolume = 70;
  hut.parts.push({ node: "absent", name: "Absent emitter", scenery: true });
  hut.gameplay!.sounds![1]!.node = "absent";
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /absent is hidden or missing/,
  );
});
test("terrain owns map defaults and conflicting terrain definitions fail", () => {
  const { document, assets, hut } = assetCompilerFixture();
  const terrain = {
    ...structuredClone(hut),
    id: "terrain",
    editor_usage: "map-background" as const,
    parts: [],
    gameplay: {
      version: 1 as const,
      collision: "none" as const,
      surfaces: [],
      doors: [],
      environment: { forest: true, defaultMaterial: 4 },
    },
  };
  assets.set(terrain.id, terrain);
  const source = {
    id: terrain.id,
    role: "ground" as const,
    model: "terrain.glb",
    model_sha256: "0".repeat(64),
    resources: [],
  };
  document.sceneAssets.push(source);
  assert.deepEqual(compileAssetGameplay(document, assets, bounds).map_settings, {
    forest_level: true,
    default_material: 4,
  });
  const other = structuredClone(terrain);
  other.id = "other-terrain";
  other.gameplay.environment.forest = false;
  assets.set(other.id, other);
  document.sceneAssets.push({ ...source, id: other.id });
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /Terrain assets disagree/);
  hut.gameplay!.environment = { forest: false, defaultMaterial: 0 };
  assert.throws(() => validateAssetGameplay(hut.gameplay, hut), /invalid terrain environment/);
});
test("material regions follow placement and preserve separate ground and obstacle lookups", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.materials = [
    {
      id: "stone-inlay",
      node: "building-999",
      material: 2,
      ground: false,
      obstacles: ["building-999"],
      polygon: [
        [40, 40, 10],
        [50, 40, 10],
        [50, 50, 10],
        [40, 50, 10],
      ],
    },
    {
      id: "water",
      node: "building-999",
      material: 5,
      ground: true,
      obstacles: [],
      polygon: [
        [10, 10, 0],
        [20, 10, 0],
        [20, 20, 0],
        [10, 20, 0],
      ],
    },
  ];
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(compiled.sight_material_indices, [1]);
  assert.deepEqual(compiled.sight_obstacles[0]!.material_indices, [0]);
  assert.deepEqual(compiled.material_sectors![0]!.polygon.points[0], [340, 330]);
  for (const p of document.objects) p.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(moved.material_sectors![0]!.polygon.points[0], [440, 330]);
  hut.gameplay!.materials[0]!.obstacles = ["missing"];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid material region/);
});

test("receiving materials preserve a joined walking area and follow asset placement", () => {
  const { document, assets, hut } = projectionMaterialCompilerFixture();
  const gameplay = hut.gameplay!;
  const compiled = compileAssetGameplay(document, assets, bounds);
  const receivers = compiled.sight_obstacles.filter(
    (obstacle) => obstacle.projection_area !== null,
  );
  assert.equal(receivers.length, 2);
  assert.deepEqual(
    receivers.map((obstacle) => obstacle.default_material),
    [2, 4],
  );
  assert.deepEqual(
    receivers.map((obstacle) => obstacle.material_indices),
    [[0], []],
  );
  assert.deepEqual(receivers[0]!.projection_area, receivers[1]!.projection_area);
  assert.deepEqual(compiled.sight_material_indices, []);
  for (const part of document.objects) part.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(moved.material_sectors![0]!.polygon.points[0], [410, 290]);
  assert.deepEqual(
    moved.sight_obstacles[0]!.points.map((point) => point.x),
    receivers[0]!.points.map((point) => point.x + 100),
  );
  gameplay.surfaces[0]!.projectionMaterials!.regions = ["missing"];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /invalid material region|invalid projection materials/,
  );
});

test("receiving plane anchors survive clipping and follow asset placement", () => {
  const { document, assets, hut } = projectionMaterialCompilerFixture();
  const surface = hut.gameplay!.surfaces[0]!;
  surface.projectionMaterials!.planePoints = [
    [100, 0, 20],
    [100, 100, 20],
    [0, 0, 20],
  ];
  const inset = hut.gameplay!.surfaces[1]!;
  inset.polygon = [
    [40, 40],
    [60, 40],
    [60, 60],
    [40, 60],
  ];
  inset.projectionMaterials!.priority = 1;
  const compiled = compileAssetGameplay(document, assets, bounds);
  const receivers = compiled.sight_obstacles.filter((o) => o.projection_plane);
  assert.ok(receivers.length > 1, "hole should subdivide the receiver");
  const expected = [
    [400, 300, 20],
    [400, 400, 20],
    [300, 300, 20],
  ];
  for (const receiver of receivers) {
    assert.deepEqual(receiver.projection_plane, receivers[0]!.projection_plane);
    assert.deepEqual(
      receiver.projection_plane!.map((p) => p.map(Math.fround)),
      expected,
    );
  }
  for (const part of document.objects) part.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds);
  for (const receiver of moved.sight_obstacles.filter((o) => o.projection_plane))
    assert.deepEqual(
      receiver.projection_plane!.map((p) => p.map(Math.fround)),
      expected.map(([x, y, z]) => [x! + 100, y, z]),
    );
  surface.projectionMaterials!.planePoints[0][2] = 21;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /anchors must lie/);
  surface.projectionMaterials!.planePoints = [
    [0, 0, 20],
    [0, 0, 20],
    [0, 0, 20],
  ];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /nondegenerate/);
});

test("rotated copies keep receiving material references local to each placement", () => {
  const { document, assets, hut } = projectionMaterialCompilerFixture();
  hut.gameplay!.surfaces[0]!.projectionMaterials!.planePoints = [
    [100, 0, 20],
    [100, 100, 20],
    [0, 0, 20],
  ];
  const group = document.groups[0]!;
  group.transform.rot_deg = 90;
  group.transform.dx = 800;
  group.transform.dy = 600;
  document.groups.push({
    ...structuredClone(group),
    id: "hut-copy",
    transform: { ...group.transform, dx: 1400 },
  });
  const part = document.objects.find((object) => object.group === group.id)!;
  document.objects.push({ ...structuredClone(part), id: "hut-copy-body", group: "hut-copy" });
  const compiled = compileAssetGameplay(document, assets, bounds);
  const linked = compiled.sight_obstacles.filter((obstacle) => obstacle.material_indices.length);
  assert.equal(linked.length, 2);
  assert.deepEqual(
    linked.map((obstacle) => obstacle.material_indices),
    [[0], [1]],
  );
  assert.notDeepEqual(linked[0]!.projection_area, linked[1]!.projection_area);
  const anchors = linked.map((receiver) =>
    receiver.projection_plane!.map((p) => p.map(Math.fround)),
  );
  assert.deepEqual(
    anchors[1],
    linked[0]!.projection_plane!.map(([x, y, z]) => [x + 600, y, z].map(Math.fround)),
  );
  assert.notEqual(anchors[0]![0]![0], anchors[0]![1]![0], "rotation must affect the anchors");
  const [first, second] = compiled.material_sectors!;
  assert.deepEqual(
    second!.polygon.points,
    first!.polygon.points.map(([x, y]) => [x + 600, y]),
  );
  assert.notDeepEqual(first!.polygon.points, [
    [310, 290],
    [330, 290],
    [330, 310],
    [310, 310],
  ]);
});

test("receiving footprints retain material across portions omitted from walking contours", () => {
  const { document, assets, hut } = projectionMaterialCompilerFixture();
  const surface = hut.gameplay!.surfaces[0]!;
  surface.projectionMaterials!.footprint = surface.polygon.map(([x, y]) => [x, y, 20]);
  surface.polygon = [
    [0, 0],
    [100, 0],
    [100, 100],
    [50, 100],
    [50, 50],
    [0, 50],
  ];
  hut.gameplay!.surfaces.push({
    id: "adjoining-contour",
    node: surface.node,
    height: 20,
    polygon: [
      [0, 50],
      [50, 50],
      [50, 100],
      [0, 100],
    ],
  });
  const geometry = compileAssetGameplay(document, assets, bounds);
  const receivers = geometry.sight_obstacles.filter(
    (obstacle) => obstacle.projection_area !== null,
  );
  assert.equal(receivers.length, 2);
  assert.deepEqual(
    new Set(receivers[0]!.points.map((point) => `${point.x},${point.y}`)),
    new Set(["300,300", "400,300", "400,400", "300,400"]),
  );
  assert.equal(receivers[0]!.default_material, 2);
});

test("material constructors are included in regenerated interior identities", () => {
  const { document, assets } = interiorAssetCompilerFixture();
  const descriptor = [...assets.values()].find((a) => a.gameplay?.interiors?.length)!;
  const before = compileAssetGameplay(document, assets, bounds);
  descriptor.gameplay!.materials = [
    {
      id: "floor",
      node: descriptor.parts[0]!.node,
      material: 2,
      ground: true,
      obstacles: [],
      polygon: [
        [0, 0, 0],
        [10, 0, 0],
        [10, 10, 0],
        [0, 10, 0],
      ],
    },
  ];
  const after = compileAssetGameplay(document, assets, bounds);
  assert.equal(
    after.buildings![0]!.Building.doors[0]!.sector_in,
    before.buildings![0]!.Building.doors[0]!.sector_in + 1,
  );
});
test("movement clearances follow their owner and cannot erase another asset's collision", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.movementClearances = [
    {
      id: "opening",
      node: "building-999",
      polygon: [
        [39, 39],
        [51, 39],
        [51, 51],
        [39, 51],
      ],
      height: 0,
    },
  ];
  const clear = compileAssetGameplay(document, assets, bounds);
  assert.equal(clear.motion_data.layers[0]![0]!.obstacles.length, 0);
  assert.equal(clear.sight_obstacles[0]!.solid, true);
  for (const p of document.objects) p.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.equal(moved.motion_data.layers[0]![0]!.obstacles.length, 0);
  assert.equal(moved.sight_obstacles[0]!.points[0]!.x, 440);
  const other = assets.get("marker")!;
  other.gameplay!.collision = "parts";
  other.parts[0]!.obstacle_local_game = structuredClone(hut.parts[0]!.obstacle_local_game!);
  assert.equal(
    compileAssetGameplay(document, assets, bounds).motion_data.layers[0]![0]!.obstacles.length,
    1,
  );
  other.gameplay!.collision = "none";
  hut.gameplay!.movementClearances[0]!.height = 1;
  assert.equal(
    compileAssetGameplay(document, assets, bounds).motion_data.layers[0]![0]!.obstacles.length,
    1,
  );
});

test("fractional clearance intersections preserve an integer sloping movement boundary", () => {
  const { document, assets, hut } = assetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.doors = [];
  gameplay.surfaces = [
    {
      id: "slope-edge",
      node: "building-999",
      height: 0,
      polygon: [
        [0, 0],
        [100, 90],
        [100, 0],
      ],
    },
  ];
  gameplay.collision = "none";
  const expected = compileAssetGameplay(document, assets, bounds).motion_data;
  gameplay.collision = "parts";
  gameplay.movementClearances = [
    {
      id: "clipped-opening",
      node: "building-999",
      height: 0,
      polygon: [
        [130 / 3, 39],
        [51, 39],
        [51, 45.9],
      ],
    },
  ];
  assert.deepEqual(compileAssetGameplay(document, assets, bounds).motion_data, expected);
});

test("continuous movement cutouts join before their shared fractional edge is rounded", () => {
  const { document, assets, hut } = assetCompilerFixture();
  const gameplay = hut.gameplay!;
  gameplay.collision = "none";
  gameplay.doors = [];
  gameplay.surfaces = [
    {
      id: "floor",
      node: "building-999",
      height: 0,
      polygon: [
        [0, 0],
        [100, 0],
        [100, 100],
        [0, 100],
      ],
    },
  ];
  const common = { node: "building-999", height: 0, preserveMovementPrecision: true };
  gameplay.movementBlockers = [
    {
      ...common,
      id: "whole",
      polygon: [
        [0, 0],
        [100, 90],
        [100, 100],
        [0, 100],
      ],
    },
  ];
  const expected = compileAssetGameplay(document, assets, bounds).motion_data;
  gameplay.movementBlockers = [
    {
      ...common,
      id: "left",
      polygon: [
        [0, 0],
        [45.3, 40.77],
        [45.3, 100],
        [0, 100],
      ],
    },
    {
      ...common,
      id: "right",
      polygon: [
        [45.3, 40.77],
        [100, 90],
        [100, 100],
        [45.3, 100],
      ],
    },
  ];
  assert.deepEqual(compileAssetGameplay(document, assets, bounds).motion_data, expected);
  Object.assign(gameplay.movementBlockers[0]!, { preserveMovementPrecision: "yes" });
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /movement precision/);
});

test("an enclosed clearance retains a walkable island inside derived collision", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.movementClearances = [
    {
      id: "island",
      node: "building-999",
      polygon: [
        [42, 42],
        [48, 42],
        [48, 48],
        [42, 48],
      ],
      height: 0,
    },
  ];
  const result = compileAssetGameplay(document, assets, bounds);
  assert.equal(result.motion_data.layers[0]!.length, 3);
  assert.ok(
    result.motion_data.layers[0]!.some((a) =>
      a.polygon.points.every(([x, y]) => x >= 342 && x <= 348 && y >= 342 && y <= 348),
    ),
  );
});
test("map assets reject mission spawns rather than silently dropping them", () => {
  const { document, assets, hut } = assetCompilerFixture();
  Object.assign(hut.gameplay!, {
    spawns: [{ id: "player", node: "building-999", position: [20, 20, 0] }],
  });
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /spawns belong to missions/);
});
test("authored subpixel surfaces remain errors rather than being silently omitted", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.surfaces[0]!.polygon = [
    [0, 0],
    [0.1, 0],
    [0.1, 100],
    [0, 100],
  ];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /collapses after coordinate quantization/,
  );
});

test("authored movement contours follow an asset independently of sight and terrain", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.doors = [];
  hut.gameplay!.surfaces = [];
  hut.gameplay!.movementBlockers = [
    {
      id: "clearance",
      node: "building-999",
      polygon: [
        [35, 35],
        [55, 35],
        [55, 55],
        [35, 55],
      ],
      height: 0,
    },
  ];
  const marker = assets.get("marker")!.gameplay!;
  marker.surfaces = [
    {
      id: "terrain",
      node: "scenery-marker",
      polygon: [
        [0, 0],
        [300, 0],
        [300, 200],
        [0, 200],
      ],
      height: 0,
    },
  ];
  const before = compileAssetGameplay(document, assets, bounds);
  const sortedPoints = (points: [number, number][]) =>
    [...points].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  assert.deepEqual(sortedPoints(before.motion_data.layers[0]![0]!.obstacles[0]!.polygon.points), [
    [335, 335],
    [335, 355],
    [355, 335],
    [355, 355],
  ]);
  assert.equal(before.sight_obstacles[0]!.points[0]!.x, 340);
  document.objects[0]!.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    moved.motion_data.layers[0]![0]!.polygon,
    before.motion_data.layers[0]![0]!.polygon,
  );
  assert.deepEqual(sortedPoints(moved.motion_data.layers[0]![0]!.obstacles[0]!.polygon.points), [
    [435, 335],
    [435, 355],
    [455, 335],
    [455, 355],
  ]);
  // A plane-specific blocker no longer blocks ground when raised above it.
  document.objects[0]!.transform.dz = 100;
  assert.equal(
    compileAssetGameplay(document, assets, bounds).motion_data.layers[0]![0]!.obstacles.length,
    0,
  );
});

test("asset-only compilation constructs motion areas, fresh references and doors", () => {
  const { document, assets } = assetCompilerFixture();
  const result = compileAssetGameplay(document, assets, bounds);
  assert.equal(result.motion_data.layers.length, 2);
  assert.equal(result.motion_data.layers[0]!.length, 2);
  assert.deepEqual(result.motion_data.graph_bytes, []);
  assert.equal("marker" in result, false);
  assert.deepEqual(result.sight_obstacles[0]!.material_indices, []);
  assert.equal(result.doors[0]!.sector_out, 0);
  assert.equal(result.doors[0]!.sector_in, 2);
  assert.deepEqual(result.doors[0]!.point_out, [380, 350]);
  delete document.sourceMap;
  document.objects[0]!.source = { map: "other", obstacle: 0 };
  document.objects[0]!.obstacle = undefined;
  assert.deepEqual(compileAssetGameplay(document, assets, bounds), result);
});
test("movement blocker holes preserve walkable islands and invalid contours fail", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.doors = [];
  hut.gameplay!.movementBlockers = [
    {
      id: "courtyard-wall",
      node: "building-999",
      polygon: [
        [10, 10],
        [80, 10],
        [80, 80],
        [10, 80],
      ],
      height: 0,
      holes: [
        [
          [15, 15],
          [70, 15],
          [70, 70],
          [15, 70],
        ],
      ],
    },
  ];
  const compiled = compileAssetGameplay(document, assets, bounds);
  assert.equal(compiled.motion_data.layers[0]!.length, 3);
  hut.gameplay!.movementBlockers[0]!.height = [0, 0, 1, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /must be planar/);
});
test("moving, rotating and duplicating assets rebuilds their geometry and connections", () => {
  const { document, assets } = assetCompilerFixture();
  const before = compileAssetGameplay(document, assets, bounds);
  for (const part of document.objects) {
    part.transform.dx += 200;
    part.transform.dy += 100;
  }
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    moved.doors[0]!.point_out,
    before.doors[0]!.point_out.map((v, i) => v + (i ? 100 : 200)),
  );
  // Duplicate the complete asset; local IDs and provenance are intentionally identical.
  const clone = structuredClone(document.objects[0]!);
  clone.id = "hut-b-body";
  clone.group = "hut-b";
  clone.transform = { ...IDENTITY_TRANSFORM };
  document.objects.push(clone);
  document.groups.push({
    id: "hut-b",
    transform: { ...IDENTITY_TRANSFORM, dx: 1100, dy: 400, rot_deg: 90 },
  });
  const duplicated = compileAssetGameplay(document, assets, bounds);
  assert.equal(duplicated.doors.length, 2);
  assert.equal(duplicated.sight_obstacles.length, 2);
  const door = duplicated.doors[1]!;
  assert.notEqual(door.sector_in, duplicated.doors[0]!.sector_in);
  assert.equal(door.point_in[0], door.point_out[0]);
  assert.equal(door.point_in[1] - door.point_out[1], 23); // 40 * sin(35°), quantized
});
test("missing metadata and disconnected doors fail; no level-data fallback", () => {
  const { document, assets, hut } = assetCompilerFixture();
  delete hut.gameplay;
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /Missing asset gameplay definitions.*hut/,
  );
  const fresh = assetCompilerFixture();
  fresh.hut.gameplay!.doors[0]!.inside = [500, 500, 0];
  assert.throws(
    () => compileAssetGameplay(fresh.document, fresh.assets, bounds),
    /inside must resolve/,
  );
  fresh.hut.gameplay!.doors[0]!.node = "absent";
  assert.throws(
    () => validateAssetGameplay(fresh.hut.gameplay, fresh.hut),
    /unknown gameplay node/,
  );
});
test("passages without click polygons retain their navigation endpoints", () => {
  const { document, assets, hut } = assetCompilerFixture();
  const before = compileAssetGameplay(document, assets, bounds);
  hut.gameplay!.doors[0]!.polygon = [];
  const result = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(
    result.doors,
    before.doors.map((d) => ({ ...d, door_sector: { points: [] } })),
  );
  hut.gameplay!.doors[0]!.polygon = [
    [1, 1],
    [2, 2],
  ];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /invalid gameplay polygon/);
});

test("connection errors distinguish blocked geometry from a height mismatch", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.doors[0]!.outside = [45, 45, 0];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /projected \[345,345\].*"blocked":true/,
  );
  hut.gameplay!.doors[0]!.outside = [20, 20, 1];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /"height":0,"blocked":false/);
});
test("door transition lock rules remain asset-local and survive placement", () => {
  const { document, assets, hut } = assetCompilerFixture();
  const door = hut.gameplay!.doors[0]!;
  door.locked = true;
  door.unlockable = true;
  door.type = 7;
  door.afterTransition = {
    locked: false,
    unlockable: false,
    lockedVillains: true,
    lockedCivilians: true,
  };
  const result = compileAssetGameplay(document, assets, bounds).doors[0]!;
  assert.equal(result.door_type, 7);
  assert.equal(result.locked_pc, true);
  assert.equal(result.locked_pc_after_patch, false);
  assert.equal(result.unlockable_after_patch, false);
  assert.equal(result.locked_npc_villain_after_patch, true);
  assert.equal(result.locked_npc_civilian_after_patch, true);
  for (const part of document.objects) part.transform.dx += 100;
  const moved = compileAssetGameplay(document, assets, bounds).doors[0]!;
  assert.deepEqual(moved.point_in, [result.point_in[0] + 100, result.point_in[1]]);
  assert.equal(moved.locked_npc_villain_after_patch, true);
});
test("adjacent same-height asset surfaces are joined without a blocking seam", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.surfaces[1]!.polygon = [
    [90, 0],
    [200, 0],
    [200, 100],
    [90, 100],
  ];
  hut.gameplay!.doors = [];
  const result = compileAssetGameplay(document, assets, bounds);
  assert.equal(result.motion_data.layers[0]!.length, 1);
  assert.equal(result.motion_data.layers[0]![0]!.obstacles.length, 1);
});

test("elevation translates motion in projected coordinates while sight remains in world coordinates", () => {
  const { document, assets } = assetCompilerFixture();
  for (const part of document.objects) part.transform.dz = 100;
  const result = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(result.doors[0]!.point_out, [380, 250]);
  assert.equal(result.sight_obstacles[0]!.points[0]!.y, 340);
  assert.equal(result.sight_obstacles[0]!.points[0]!.z_bottom, 100);
  assert.deepEqual(result.motion_data.layers[0]![0]!.polygon.points[0], [300, 200]);
  const floor = result.sight_obstacles.find(
    (o) => Array.isArray(o.projection_area) && o.projection_area[0] === 0,
  )!;
  assert.equal(floor.points[0]!.z_top, 100);
  assert.equal(floor.points[0]!.y, 300);
});

test("sloped surfaces preserve height, holes and the intersecting slice of solids", () => {
  const { document, assets, hut } = slopedAssetCompilerFixture();
  const result = compileAssetGameplay(document, assets, bounds);
  const area = result.motion_data.layers[0]![0]!;
  assert.equal(area.obstacles.length, 2); // Authored hole and the solid crossing the ramp.
  const projection = result.sight_obstacles.find((o) => Array.isArray(o.projection_area))!;
  assert.deepEqual(
    projection.points.map((p) => p.z_top),
    [0, 100, 100, 0],
  );
  // Raise the solid above the whole ramp; it must stop blocking navigation.
  for (const point of hut.parts[0]!.obstacle_local_game!.points) {
    point.z_bottom = 110;
    point.z_top = 140;
  }
  assert.equal(
    compileAssetGameplay(document, assets, bounds).motion_data.layers[0]![0]!.obstacles.length,
    1,
  );
});

test("sloped asset placement transforms the height plane without mission content", () => {
  const { document, assets } = slopedAssetCompilerFixture();
  document.objects[1]!.group = "hut-a";
  document.groups[0]!.transform = { ...IDENTITY_TRANSFORM, dx: 800, dy: 200, rot_deg: 90, dz: 30 };
  const result = compileAssetGameplay(document, assets, bounds);
  const surface = result.sight_obstacles.find((o) => Array.isArray(o.projection_area))!;
  const plane = heightPlane(surface.points.map((p) => [p.x, p.y - p.z_top, p.z_top]));
  for (const p of surface.points)
    assert.ok(Math.abs(planeHeight(plane, [p.x, p.y - p.z_top]) - p.z_top) < 1e-4);
  assert.ok(Math.max(...surface.points.map((p) => p.z_top)) > 129);
});

test("non-planar and degenerate surfaces fail instead of silently flattening", () => {
  const { document, assets, hut } = slopedAssetCompilerFixture();
  hut.gameplay!.surfaces[0]!.height = [0, 100, 110, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /must be planar/);
  hut.gameplay!.surfaces[0]!.polygon = [
    [0, 0],
    [10, 0],
    [20, 0],
  ];
  hut.gameplay!.surfaces[0]!.height = 0;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /nondegenerate/);
});

test("ordinary passages can connect to a lift surface in either direction", () => {
  const { hut, document, assets } = liftAssetCompilerFixture();
  const low = hut.gameplay!.lifts![0]!.doors[0]!;
  for (const reverse of [false, true]) {
    hut.gameplay!.doors = [
      {
        ...low,
        id: "passage",
        type: 0,
        outside: reverse ? low.inside : low.outside,
        inside: reverse ? low.outside : low.inside,
      },
    ];
    const compiled = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]);
    const door = compiled.doors[0]!;
    assert.equal(reverse ? door.sector_out : door.sector_in, compiled.lifts![0]!.motion_area_index);
    assert.equal(reverse ? door.layer_out : door.layer_in, compiled.motion_data.layers.length - 1);
    assert.equal(door.door_type, 0);
  }
});

test("lift surfaces use the reserved layer and rebuild endpoint references", () => {
  const { document, assets } = liftAssetCompilerFixture();
  const result = compileAssetGameplay(document, assets, bounds);
  assert.equal(result.motion_data.layers.length, 3);
  assert.equal(result.motion_data.layers.at(-1)![0]!.is_lift, true);
  assert.equal(result.doors.length, 0);
  const lift = result.lifts![0]!;
  assert.equal(lift.lift_type, 1);
  assert.equal(lift.direction, 4);
  assert.equal(lift.motion_area_index, 3); // Ground blocker occupies sector 1.
  assert.deepEqual(
    lift.doors.map((d) => d.layer_out),
    [0, 1],
  );
  assert.ok(lift.doors.every((d) => d.layer_in === 2 && d.sector_in === 3));
  const clone = structuredClone(document.objects[0]!);
  clone.id = "stairs-copy";
  clone.group = "stairs-copy";
  clone.transform = { ...IDENTITY_TRANSFORM };
  document.objects.push(clone);
  document.groups.push({
    id: "stairs-copy",
    transform: { ...IDENTITY_TRANSFORM, dx: 1000, dy: 700, rot_deg: 90 },
  });
  const duplicated = compileAssetGameplay(document, assets, bounds);
  assert.equal(duplicated.lifts!.length, 2);
  assert.notEqual(duplicated.lifts![0]!.motion_area_index, duplicated.lifts![1]!.motion_area_index);
  assert.equal(duplicated.lifts![1]!.direction, 8);
});

test("lift validation rejects missing traversal endpoints and mismatched surface ownership", () => {
  const { hut, document, assets } = liftAssetCompilerFixture();
  hut.gameplay!.lifts![0]!.doors.pop();
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /at least two traversal doors/,
  );
  hut.gameplay!.lifts![0]!.surface = "absent";
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /needs its own surface/);
});

test("interior entrances share a fresh virtual sector independent of motion polygons", () => {
  const { document, assets } = interiorAssetCompilerFixture();
  const result = compileAssetGameplay(document, assets, bounds);
  const doors = result.buildings![0]!.Building.doors;
  assert.equal(result.doors.length, 1);
  assert.equal(doors.length, 2);
  assert.ok(doors.every((d) => d.sector_in === 4 && d.layer_in === 1 && d.sector_out === 0));
  assert.equal(doors[1]!.locked_pc, true);
  assert.ok(doors.every((d) => d.locked_npc_civilian));
  const clone = structuredClone(document.objects[0]!);
  clone.id = "house-copy";
  clone.group = "house-copy";
  clone.transform.dx += 600;
  document.objects.push(clone);
  document.groups.push({ id: "house-copy", transform: { ...IDENTITY_TRANSFORM } });
  const duplicated = compileAssetGameplay(document, assets, bounds);
  assert.equal(duplicated.buildings!.length, 2);
  assert.notEqual(
    duplicated.buildings![0]!.Building.doors[0]!.sector_in,
    duplicated.buildings![1]!.Building.doors[0]!.sector_in,
  );
});

test("export frame clips sloped navigation and generated receivers without modifying the document", () => {
  const { document, assets } = slopedAssetCompilerFixture();
  const before = structuredClone(document);
  const geometry = compileAssetGameplay(document, assets, [320, 200, 100, 140]);
  assert.ok(geometry.motion_data.layers.flat().length);
  for (const area of geometry.motion_data.layers.flat())
    for (const [x, y] of area.polygon.points)
      assert.ok(x >= 0 && x <= 100 && y >= 0 && y <= 140, `outside: ${x},${y}`);
  const receivers = geometry.sight_obstacles.filter((s) => s.projection_area && !s.solid);
  assert.ok(receivers.length);
  for (const receiver of receivers)
    for (const p of receiver.points) {
      assert.ok(p.x >= 0 && p.x <= 100 && p.y - p.z_top >= 0 && p.y - p.z_top <= 140);
      assert.ok(Math.abs(p.z_top - (p.x + 20) / 2) < 1e-6);
    }
  assert.deepEqual(document, before);
  assert.match(geometry.warnings!.join("\n"), /clipped to the export frame/);
});

test("cropping a preserved movement contour cannot retain navigation beyond the image", () => {
  const { document, assets } = preservedBoundaryCompilerFixture();
  const geometry = compileAssetGameplay(document, assets, [300, 300, 50, 50]);
  assert.ok(geometry.motion_data.layers.flat().length);
  for (const area of geometry.motion_data.layers.flat())
    for (const [x, y] of area.polygon.points) assert.ok(x >= 0 && x <= 50 && y >= 0 && y <= 50);
});

test("cropped material regions rebuild ground and physical receiver indices", () => {
  const { document, assets, hut } = slopedAssetCompilerFixture();
  hut.gameplay!.materials = [
    {
      id: "outside",
      node: "building-999",
      material: 5,
      ground: true,
      obstacles: ["building-999"],
      polygon: [
        [0, 0, 0],
        [10, 0, 0],
        [10, 10, 0],
        [0, 10, 0],
      ],
    },
    {
      id: "partial",
      node: "building-999",
      material: 2,
      ground: true,
      obstacles: ["building-999"],
      polygon: [
        [10, 0, 0],
        [100, 70, 0],
        [100, 100, 0],
        [10, 100, 0],
      ],
    },
  ];
  const geometry = compileAssetGameplay(document, assets, [320, 200, 100, 140]);
  assert.equal(geometry.material_sectors!.length, 1);
  assert.deepEqual(geometry.sight_material_indices, [0]);
  assert.deepEqual(geometry.sight_obstacles[0]!.material_indices, [0]);
  for (const [x, y] of geometry.material_sectors![0]!.polygon.points) {
    assert.ok(x >= 0 && x <= 100 && y >= 0 && y <= 140);
    assert.ok(
      Number.isInteger(x) && Number.isInteger(y),
      "native material polygons use integer pixels",
    );
  }
});

test("cropped strict exports omit unavailable feature anchors with explicit warnings", () => {
  for (const fixture of [
    assetCompilerFixture,
    jumpAssetCompilerFixture,
    lightAssetCompilerFixture,
    liftAssetCompilerFixture,
    sightTransitionCompilerFixture,
  ]) {
    const { document, assets } = fixture();
    const before = structuredClone(document);
    const geometry = compileAssetGameplay(document, assets, [0, 0, 320, 330]);
    assert.match(geometry.warnings!.join("\n"), /omitted/);
    assert.deepEqual(document, before);
    for (const area of geometry.motion_data.layers.flat())
      for (const [x, y] of area.polygon.points) assert.ok(x >= 0 && x <= 320 && y >= 0 && y <= 330);
  }
});

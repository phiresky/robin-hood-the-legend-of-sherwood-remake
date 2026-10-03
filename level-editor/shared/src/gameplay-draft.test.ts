import test from "node:test";
import assert from "node:assert/strict";
import { validateAssetGameplay } from "./asset-gameplay.ts";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import {
  assetCompilerFixture,
  jumpAssetCompilerFixture,
  liftAssetCompilerFixture,
  compoundLiftCompilerFixture,
  maskAssetCompilerFixture,
  interiorAssetCompilerFixture,
  lightAssetCompilerFixture,
} from "../test-fixtures/asset-gameplay.ts";

const bounds: [number, number, number, number] = [0, 0, 2000, 2000];

test("draft gameplay compiles unchanged geometry and warns once per asset issue", () => {
  const { document, assets, hut } = assetCompilerFixture();
  const body = document.objects[0]!;
  document.objects.push({
    ...body,
    id: "hut-b-body",
    group: "hut-b",
    transform: { ...body.transform, dx: 900 },
  });
  document.groups.push({ ...document.groups[0]!, id: "hut-b" });
  const complete = compileAssetGameplay(document, assets, bounds);
  hut.gameplay!.draft = {
    issues: ["Occlusion mask ownership is incomplete.", "Actor traversal remains unverified."],
  };
  const draft = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(draft.warnings, [
    "Draft gameplay asset hut: Occlusion mask ownership is incomplete.",
    "Draft gameplay asset hut: Actor traversal remains unverified.",
    ...(complete.warnings ?? []),
  ]);
  assert.deepEqual({ ...draft, warnings: undefined }, { ...complete, warnings: undefined });
});

test("draft status never bypasses missing or invalid gameplay definitions", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.draft = { issues: ["Door geometry needs review."] };
  hut.gameplay!.doors[0]!.inside = [1900, 1900, 0];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /inside must resolve/);
  delete hut.gameplay;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /Missing asset gameplay/);
});

test("draft metadata requires explicit nonempty distinct issue descriptions", () => {
  const { hut } = assetCompilerFixture();
  for (const draft of [
    null,
    [],
    {},
    { issues: [] },
    { issues: [" "] },
    { issues: [2] },
    { issues: ["Pending masks", "Pending masks"] },
  ]) {
    assert.throws(
      () => validateAssetGameplay({ ...hut.gameplay, draft }, hut),
      /invalid gameplay draft issues/,
    );
  }
});

test("best effort omits missing definitions without inventing walkable ground", () => {
  const { document, assets, hut } = assetCompilerFixture();
  delete hut.gameplay;
  const geometry = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.ok(geometry.warnings?.some((message) => message.includes("Asset hut: gameplay omitted")));
  assert.ok(geometry.warnings?.some((message) => message.includes("no traversable ground")));
  assert.ok(geometry.motion_data.layers.every((layer) => layer.length === 0));
  assert.equal(geometry.sight_obstacles.length, 0);
});

test("best effort preserves available gameplay and reports unsupported wall splines", () => {
  const { document, assets } = assetCompilerFixture();
  const expected = compileAssetGameplay(document, assets, bounds);
  document.splines = [
    {
      id: "wall",
      name: "Wall",
      kind: "wall",
      points: [
        [0, 0, 0],
        [100, 0, 0],
      ],
      closed: false,
      width: 10,
      repeatLength: 20,
    },
  ];
  const geometry = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.deepEqual(geometry.motion_data, expected.motion_data);
  assert.deepEqual(geometry.doors, expected.doors);
  assert.ok(geometry.warnings?.some((message) => message.includes("Wall spline wall")));
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /Wall spline/);
});

test("best effort omits disconnected doors and jump pairs but rejects malformed geometry", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.doors[0]!.inside = [1900, 1900, 0];
  const geometry = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(geometry.doors.length, 0);
  assert.ok(
    geometry.warnings?.some((message) => message.includes("Door hut-a/hut/passage: omitted")),
  );
  hut.gameplay!.surfaces[0]!.polygon = [[0, 0]];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds, { bestEffort: true }),
    /invalid gameplay polygon/,
  );
  const jump = jumpAssetCompilerFixture();
  jump.hut.gameplay!.jumpZones![0]!.anchor = [1900, 1900, 0];
  const detached = compileAssetGameplay(jump.document, jump.assets, bounds, { bestEffort: true });
  assert.equal(detached.jump_line_pairs, undefined);
  assert.ok(
    detached.warnings?.some((message) => message.includes("landing surface is unavailable")),
  );
});

test("best effort omits a disconnected lift but retains the asset collision and landings", () => {
  const { document, assets, hut } = liftAssetCompilerFixture();
  hut.gameplay!.lifts![0]!.doors[0]!.outside = [1900, 1900, 0];
  const original = structuredClone(hut);
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /outside must resolve/);
  const geometry = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(geometry.lifts, undefined);
  assert.ok(geometry.sight_obstacles.some((obstacle) => obstacle.solid));
  assert.ok(geometry.motion_data.layers.some((layer) => layer.length > 0));
  assert.equal(
    geometry.warnings?.filter((warning) => warning.startsWith("Lift hut-a/hut/stairs:")).length,
    1,
  );
  assert.deepEqual(hut, original);
});

test("disconnected compound lift removes all joined traversal pieces without dropping their owners", () => {
  const { document, assets, hut } = compoundLiftCompilerFixture();
  hut.gameplay!.lifts![0]!.doors[0]!.outside = [1900, 1900, 0];
  const geometry = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(geometry.lifts, undefined);
  assert.ok(geometry.sight_obstacles.some((obstacle) => obstacle.solid));
  assert.ok(geometry.motion_data.layers.some((layer) => layer.length > 0));
  assert.equal(geometry.warnings?.filter((warning) => warning.startsWith("Lift ")).length, 1);
});

test("separated stair pieces preserve their collision instead of failing the whole export", () => {
  const { document, assets } = compoundLiftCompilerFixture();
  document.objects.find((part) => part.id === "upper-body")!.transform.dx += 400;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /exactly one other segment/);
  const geometry = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(geometry.lifts, undefined);
  assert.ok(geometry.sight_obstacles.some((obstacle) => obstacle.solid));
  assert.equal(geometry.warnings?.filter((warning) => warning.startsWith("Lift ")).length, 2);
});

test("ambiguous stair sockets omit every competing piece instead of choosing a pair", () => {
  const { document, assets } = compoundLiftCompilerFixture();
  const upper = document.objects.find((part) => part.id === "upper-body")!;
  document.objects.push({ ...structuredClone(upper), id: "duplicate-upper", group: "duplicate" });
  document.groups.push({
    ...structuredClone(document.groups.find((group) => group.id === "upper")!),
    id: "duplicate",
  });
  const geometry = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(geometry.lifts, undefined);
  assert.ok(geometry.sight_obstacles.some((obstacle) => obstacle.solid));
  assert.equal(geometry.warnings?.filter((warning) => warning.startsWith("Lift ")).length, 1);
});

test("best effort omits unavailable masks and their empty transitions without dangling references", () => {
  const { document, assets, hut } = maskAssetCompilerFixture();
  for (const mask of hut.gameplay!.masks!) mask.anchor = [1900, 1900, 0];
  const geometry = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(geometry.masks, undefined);
  assert.deepEqual(geometry.movement_transitions, []);
  assert.equal(geometry.doors.length, 1);
  assert.equal(geometry.warnings?.filter((warning) => warning.startsWith("Mask ")).length, 2);
});

test("best effort removes empty interiors before allocating runtime room sectors", () => {
  const { document, assets, hut } = interiorAssetCompilerFixture();
  for (const interior of hut.gameplay!.interiors!)
    for (const door of interior.doors) door.outside = [1900, 1900, 0];
  const geometry = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(geometry.buildings, undefined);
  assert.ok(geometry.motion_data.layers.some((layer) => layer.length > 0));
});

test("best effort keeps valid light receivers and warns for disconnected points and segments", () => {
  const { document, assets, hut } = lightAssetCompilerFixture();
  const light = hut.gameplay!.lights![0]!;
  light.receivers = [
    [20, 20, 0],
    [20, 25, 5],
  ];
  light.receiverSegments = [
    [
      [25, 20, -1],
      [25, 22, 1],
    ],
    [
      [25, 25, 5],
      [25, 27, 7],
    ],
  ];
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiving segment/);
  const result = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(result.light_sectors!.length, 2);
  assert.equal(result.light_sectors![0]!.ambience, 1);
  assert.equal(
    result.warnings!.filter((message) => message.startsWith("Light receiver omitted:")).length,
    2,
  );
  light.receivers = [[20, 25, 5]];
  delete light.receiverSegments;
  assert.throws(() => compileAssetGameplay(document, assets, bounds), /receiver 0 must resolve/);
  assert.equal(
    compileAssetGameplay(document, assets, bounds, { bestEffort: true }).light_sectors!.length,
    1,
  );
  light.receivers = [[200, 20, 0]];
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds, { bestEffort: true }),
    /outside the light contour/,
  );
});

test("best effort omits an unanchored light region without a matching receiving plane", () => {
  const { document, assets, hut } = lightAssetCompilerFixture();
  for (const point of hut.gameplay!.lights![0]!.polygon) point[2] = 5;
  assert.throws(
    () => compileAssetGameplay(document, assets, bounds),
    /overlap at least one receiving layer/,
  );
  const result = compileAssetGameplay(document, assets, bounds, { bestEffort: true });
  assert.equal(result.light_sectors!.length, 1);
  assert.equal(result.light_sectors![0]!.ambience, 2);
  assert.ok(
    result.warnings!.some((message) => message.startsWith("Light region hut-a/hut/day-shadow:")),
  );
});

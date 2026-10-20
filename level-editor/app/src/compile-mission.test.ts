import test from "node:test";
import assert from "node:assert/strict";
import { type Level3D, validateMission, parseStoredMap } from "@rle/shared";
import { compileMap } from "./map-compile.ts";
import { assetCompilerFixture } from "../../shared/test-fixtures/asset-gameplay.ts";
import { compileMission } from "./compile-mission.ts";
import type { CompiledAssetGeometry } from "../../shared/src/asset-gameplay.ts";

const polygon = {
  points: [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ] as [number, number][],
};
const area = {
  is_lift: false,
  state_id: 0,
  polygon,
  skeleton_segments: [] as never[],
  flags: 0,
  obstacles: [],
};
const geometry: CompiledAssetGeometry = {
  motion_data: { layers: [[area], [area], []], graph_bytes: [] },
  sight_obstacles: [
    {
      projection_area: [1, 1],
      points: polygon.points.map(([x, y]) => ({ x, y: y + 20, z_top: 20, z_bottom: 20 })),
      opaque: false,
      solid: false,
      mouse: true,
      show_shadow_polygon: false,
      default_material: 0,
      material_indices: [],
    },
  ],
  doors: [],
};
function document(): Level3D {
  return {
    mission: {
      version: 1,
      spawnPoints: [
        { id: "start", name: "Start", position: [30, 50, 20], direction: 4, profile: 0 },
      ],
      soldiers: [
        {
          id: "guard",
          name: "Guard",
          position: [30, 50, 0],
          direction: 8,
          profile: "Guard",
          allegiance: 2,
        },
      ],
    },
  } as Level3D;
}

test("explicit mission markers bind to the corresponding elevation and native sector", () => {
  const result = compileMission(document(), [0, 0, 100, 100], geometry);
  assert.deepEqual(result.spawn_points, [
    { position: [30, 30], direction: 4, profile: 0, sector: 1, layer: 1, projection_area: 0 },
  ]);
  assert.deepEqual(result.soldiers, [
    {
      position: [30, 50],
      direction: 8,
      profile: "Guard",
      allegiance: 2,
      sector: 0,
      layer: 0,
      projection_area: 65535,
    },
  ]);
  assert.deepEqual(result.warnings, []);
});

test("ground placements bind on nonzero navigation layers", () => {
  const shifted = structuredClone(geometry);
  shifted.motion_data.layers.unshift([]);
  shifted.sight_obstacles[0]!.projection_area = [1, 2];
  const doc = document();
  doc.mission!.spawnPoints[0]!.position = [30, 50, 0];
  const result = compileMission(doc, [0, 0, 100, 100], shifted);
  assert.deepEqual(result.spawn_points, [
    { position: [30, 50], direction: 4, profile: 0, sector: 0, layer: 1, projection_area: 65535 },
  ]);
  assert.equal(result.soldiers[0]!.layer, 1);
  assert.deepEqual(result.warnings, []);
});

test("invalid placement warns and omits only the affected marker", () => {
  const doc = document();
  doc.mission!.spawnPoints[0]!.position[2] = 10;
  assert.throws(() => compileMission(doc, [0, 0, 100, 100], geometry), /one walkable surface/);
  const result = compileMission(doc, [0, 0, 100, 100], geometry, true);
  assert.equal(result.spawn_points.length, 0);
  assert.equal(result.soldiers.length, 1);
  assert.match(result.warnings[0]!, /start.*omitted/);
});

test("map without mission authoring produces no mission entities", () => {
  const doc = document();
  delete doc.mission;
  assert.deepEqual(compileMission(doc, [0, 0, 100, 100], geometry), {
    spawn_points: [],
    soldiers: [],
    warnings: [],
  });
});

test("campaign spawn slots omit profiles and retain import warnings", () => {
  const doc = document();
  delete doc.mission!.spawnPoints[0]!.profile;
  doc.mission!.importWarnings = ["Patrol scripts are not editable yet."];
  const result = compileMission(doc, [0, 0, 1000, 1000], geometry);
  assert.equal("profile" in result.spawn_points[0]!, false);
  assert.deepEqual(result.warnings, ["Imported mission: Patrol scripts are not editable yet."]);
});

test("switch-hidden receiving planes still bind mission marker elevation", () => {
  const switched = structuredClone(geometry);
  switched.movement_transitions = [
    {
      id: "drawbridge",
      waypoint: [20, 20],
      sector: 1,
      layer: 1,
      active: true,
      definitive: false,
      apply_polygon: { points: [] },
      no_apply_polygon: { points: [] },
      motion_changes: [],
      applied_sight: [0],
    },
  ];
  const result = compileMission(document(), [0, 0, 100, 100], switched);
  assert.equal(result.spawn_points[0]!.projection_area, 0);
  assert.equal(result.spawn_points[0]!.layer, 1);
  assert.deepEqual(result.warnings, []);
  assert.deepEqual(
    result.spawn_points,
    compileMission(document(), [0, 0, 100, 100], geometry).spawn_points,
  );
});

test("mission validation rejects duplicate IDs and invalid native fields", () => {
  const mission = document().mission!;
  validateMission(mission);
  mission.soldiers[0]!.id = "start";
  assert.throws(() => validateMission(mission), /Mission:/);
  mission.soldiers[0]!.id = "guard";
  mission.spawnPoints[0]!.direction = 16;
  assert.throws(() => validateMission(mission), /Mission:/);
});

test("map export preserves separate mission authoring in reopenable editor data", () => {
  const fixture = assetCompilerFixture();
  fixture.document.mission = document().mission;
  fixture.document.mission!.spawnPoints[0]!.position = [320, 320, 0];
  fixture.document.mission!.soldiers[0]!.position = [450, 320, 0];
  const compiled = compileMap(fixture.document, [200, 200, 500, 500], fixture.assets);
  assert.equal("spawn_player" in compiled.descriptor, false);
  assert.deepEqual(compiled.descriptor.spawn_points?.[0]?.position, [120, 120]);
  assert.deepEqual(compiled.descriptor.soldiers?.[0]?.position, [250, 120]);
  assert.deepEqual(
    parseStoredMap(compiled.editorDocument, fixture.assets).mission,
    fixture.document.mission,
  );
});

test("initial transition blockers exclude mission placements", () => {
  const blocked = structuredClone(geometry);
  blocked.motion_data.layers[0]![0] = { ...area, obstacles: [{ state_id: 1, polygon }] };
  blocked.sight_obstacles[0]!.projection_area = [2, 1];
  const result = compileMission(document(), [0, 0, 100, 100], blocked, true);
  assert.equal(result.soldiers.length, 0);
  assert.equal(result.spawn_points[0]!.sector, 2);
  assert.match(result.warnings[0]!, /guard/);
});

test("resizing omits out-of-frame mission markers even in strict export and retains editor data", () => {
  const doc = document();
  const before = structuredClone(doc);
  const result = compileMission(doc, [0, 0, 100, 40], geometry);
  assert.equal(result.spawn_points.length, 1);
  assert.equal(result.soldiers.length, 0);
  assert.match(result.warnings.join("\n"), /guard.*outside.*retained/);
  assert.deepEqual(doc, before);
});

test("native source staging keeps complete patch slots and script topology without claiming execution", async () => {
  const { prepareNativeMissionSource } = await import("./compile-mission.ts");
  const { readFile } = await import("node:fs/promises");
  const source = new URL("../../library/mission-states/croisement02/source/", import.meta.url);
  const bytes = new Map([
    [
      "Data/Levels/Emb05_FoB_MP.rhm.json",
      new Uint8Array(await readFile(new URL("Emb05_FoB_MP.rhm.json", source))),
    ],
    [
      "Data/Levels/Croisement02.rhp.json",
      new Uint8Array(await readFile(new URL("Croisement02.rhp.json", source))),
    ],
    // Pinning alone deliberately does not attest that a program decodes or can execute.
    ["Data/Levels/Emb05_FoB_MP.scb", new Uint8Array([1, 2, 3])],
  ]);
  const pin = async (path: string) => ({
    path,
    sha256: Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes.get(path)!)))
      .map((b) => b.toString(16).padStart(2, "0"))
      .join(""),
  });
  const request = {
    version: 1 as const,
    mission: "Emb05_FoB_MP",
    map: "Croisement02",
    mission_data: await pin("Data/Levels/Emb05_FoB_MP.rhm.json"),
    level_data: await pin("Data/Levels/Croisement02.rhp.json"),
    script: await pin("Data/Levels/Emb05_FoB_MP.scb"),
    runtime_resources: [],
  };
  const read = async (path: string) => {
    const row = bytes.get(path);
    if (!row) throw Error("Missing " + path);
    return row;
  };
  const result = await prepareNativeMissionSource(request, read);
  assert.equal(result.level.patches.length, 9);
  assert.equal(
    result.identities.find((r) => r.kind === "mission-patch" && r.index === 0)!.patch_slot,
    9,
  );
  assert.equal(result.identities.filter((r) => r.kind === "map-animation").length, 15);
  assert.ok(result.required_script_classes.includes("filet01_80000019"));
  assert.deepEqual(
    result.files.get(request.mission_data.path),
    bytes.get(request.mission_data.path),
  );
  assert.equal(result.status, "source-pinned-runtime-validation-required");
  assert.equal(result.runtime_validation_required, true);
  bytes.get(request.script.path)![0] = 4;
  await assert.rejects(prepareNativeMissionSource(request, read), /hash differs/);
  await assert.rejects(
    prepareNativeMissionSource(
      { ...request, script: { ...request.script, path: "Data/Levels/other.scb" } },
      read,
    ),
    /script path/,
  );
});

test("native appearance mapping is exhaustive and has one authoritative patch per compiled transition", async () => {
  const { validateNativeAppearanceBindings: validate } =
    await import("../../shared/src/compile-appearance-bindings.ts");
  const identities = [
    { kind: "mission-patch" as const, index: 0 },
    { kind: "map-animation" as const, index: 1 },
  ];
  const bindings = [
    {
      source: identities[0]!,
      delivery: { kind: "compiled-transition" as const, transition_id: "gate" },
    },
    { source: identities[1]!, delivery: { kind: "source-native" as const } },
  ];
  assert.deepEqual(
    [...validate(identities, bindings, [{ id: "gate" }])],
    [["gate", "mission-patch:0"]],
  );
  assert.throws(() => validate(identities, bindings.slice(0, 1), [{ id: "gate" }]), /Incomplete/);
  assert.throws(() => validate(identities, bindings, []), /Missing/);
  assert.throws(() => validate(identities, [...bindings, bindings[0]!], [{ id: "gate" }]), /owner/);
  assert.throws(
    () =>
      compileMap({ map: "native" } as Level3D, [0, 0, 32, 32], undefined, {
        missionMode: "native-scripted",
      }),
    /sandbox fallback is forbidden/,
  );
});

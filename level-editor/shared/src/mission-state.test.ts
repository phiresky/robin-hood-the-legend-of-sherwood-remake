import test from "node:test";
import assert from "node:assert/strict";
import { validateMissionStateContract } from "./mission-state.ts";

test("mission state contracts require exact model/source hashes and explicit finite timing", () => {
  const c = {
    version: 1,
    mission: "S03",
    mission_data_sha256: "a".repeat(64),
    level_data_sha256: "b".repeat(64),
    camera_elevation_deg: 35,
    targets: [
      {
        id: "sign",
        target_index: 4,
        target_sha256: "c".repeat(64),
        source: {
          id: "sign",
          role: "objects",
          model: "sign.glb",
          model_sha256: "d".repeat(64),
          resources: [],
        },
        model_origin: [0, 0, 0],
        representation: "physical",
        actions: [{ action: 0, clip: "turn", timing: { mode: "loop", cycleTicks: 64 } }],
      },
    ],
  };
  validateMissionStateContract(c);
  const duplicate = structuredClone(c);
  duplicate.targets.push(duplicate.targets[0]!);
  assert.throws(() => validateMissionStateContract(duplicate), /duplicate/);
  c.targets[0]!.actions[0]!.timing.cycleTicks = 0;
  assert.throws(() => validateMissionStateContract(c), /cycle/);
  c.targets[0]!.actions[0]!.timing.cycleTicks = 64;
  c.targets[0]!.source.model = "../escape.glb";
  assert.throws(() => validateMissionStateContract(c), /model/);
});

test("extraction roles require a pinned complete embedded map-animation model", () => {
  const contract = {
    version: 1,
    mission: "S03",
    mission_data_sha256: "a".repeat(64),
    level_data_sha256: "b".repeat(64),
    camera_elevation_deg: 35,
    targets: [
      {
        id: "tree",
        source_kind: "map-animation",
        target_index: 5,
        target_sha256: "c".repeat(64),
        source: {
          id: "tree",
          role: "objects",
          model: "tree.glb",
          model_sha256: "d".repeat(64),
          resources: [],
        },
        extraction_context: { path: "tree-roles.json", sha256: "e".repeat(64) },
        model_origin: [0, 0, 0],
        representation: "physical",
        actions: [{ action: 0, clip: "phases", timing: { mode: "loop", cycleTicks: 56 } }],
      },
    ],
  };
  validateMissionStateContract(contract);
  for (const change of [
    (t: any) => {
      t.extraction_context.path = "../bad";
    },
    (t: any) => {
      t.extraction_context.sha256 = "bad";
    },
    (t: any) => {
      t.source_kind = "mission-target";
    },
    (t: any) => {
      t.source.model_scene = "subset";
    },
    (t: any) => {
      t.source.resources.push({ path: "external.bin", sha256: "f".repeat(64) });
    },
    (t: any) => {
      t.representation = "native-appearance";
    },
  ]) {
    const invalid = structuredClone(contract);
    change(invalid.targets[0]);
    assert.throws(() => validateMissionStateContract(invalid), /extraction context/);
  }
});

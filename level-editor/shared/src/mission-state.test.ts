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

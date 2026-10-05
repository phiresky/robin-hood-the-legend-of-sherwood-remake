import test from "node:test";
import assert from "node:assert/strict";
import { validateStateDelivery, type StateDeliveryContract } from "./state-delivery.ts";

export function deliveryFixture(): StateDeliveryContract {
  const source = (id: string) => ({
    id,
    role: "objects" as const,
    model: `${id}.glb`,
    model_sha256: "a".repeat(64),
    resources: [],
  });
  const frame = {
    path: "frame.png",
    sha256: "a".repeat(64),
    width: 1,
    height: 1,
    offset: [0, 0] as [number, number],
    delay: 2,
  };
  return {
    version: 1,
    scope: "controlled-state-preview",
    native: {
      version: 1,
      mission: "test",
      mission_data_sha256: "a".repeat(64),
      level_data_sha256: "a".repeat(64),
      camera_elevation_deg: 35,
      scope: "map-art-and-listed-effects",
      background: frame,
      origin: [0, 0],
      elements: [
        {
          id: "body",
          source: { kind: "mission-target", index: 0, sha256: "a".repeat(64) },
          active: false,
          frames: [frame, frame],
          loop: false,
          display_position: [0, 0],
          sort_position: [0, 0],
          display_order: 0,
          creation_order: 0,
          polyline: [],
        },
      ],
    },
    families: [
      {
        id: "trap",
        element_ids: ["body"],
        background_ids: [],
        body_terminal_tick: 3,
        physical: { initial: [source("initial")], applied: [source("applied")] },
      },
    ],
  };
}
test("delivery requires independent physical endpoints and exact native terminal timing", () => {
  const c = deliveryFixture();
  validateStateDelivery(c);
  const bad = structuredClone(c);
  bad.families[0]!.body_terminal_tick = 6;
  assert.throws(() => validateStateDelivery(bad), /terminal timing/);
  bad.families[0]!.body_terminal_tick = 3;
  bad.families[0]!.physical.applied = [];
  assert.throws(() => validateStateDelivery(bad), /physical endpoint/);
  const duplicate = structuredClone(c);
  duplicate.families.push({ ...duplicate.families[0]!, id: "other" });
  assert.throws(() => validateStateDelivery(duplicate), /shared transition/);
  c.native.elements[0]!.loop = true;
  assert.throws(() => validateStateDelivery(c), /transition element/);
});

test("physical placement accepts finite scene translations and rejects invalid vectors", () => {
  const c = deliveryFixture();
  c.families[0]!.physical.initial[0]!.position = [1, -2, 3];
  validateStateDelivery(c);
  c.families[0]!.physical.initial[0]!.position = [1, NaN, 3];
  assert.throws(() => validateStateDelivery(c), /physical source/);
  c.families[0]!.physical.initial[0]!.position = [1, 2] as unknown as [number, number, number];
  assert.throws(() => validateStateDelivery(c), /physical source/);
});

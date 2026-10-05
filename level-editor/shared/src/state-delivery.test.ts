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

test("an explicitly absent endpoint is valid but an unprepared empty model list is not", async () => {
  const { physicalEndpointSources } = await import("./state-delivery.ts");
  const c = deliveryFixture();
  c.families[0]!.physical.initial = { kind: "absent" };
  validateStateDelivery(c);
  assert.deepEqual(physicalEndpointSources(c.families[0]!.physical.initial), []);
  c.families[0]!.physical.applied = [];
  assert.throws(() => validateStateDelivery(c), /physical endpoint/);
});

test("static replacements bind the exact source, object and both placement transforms", async () => {
  const { verifyStaticStateReplacements } = await import("./state-delivery.ts");
  const c = deliveryFixture();
  const transform = { dx: 1, dy: 2, dz: 3, rot_deg: 0 };
  const groupTransform = { dx: 20, dy: 30, dz: 0, rot_deg: 15 };
  c.families[0]!.static_replacements = {
    initial: [],
    applied: [
      {
        object_id: "fence",
        node: "asset:timber:part",
        asset_id: "timber",
        model_sha256: "b".repeat(64),
        transform,
        group_id: "placed-fence",
        group_transform: groupTransform,
      },
    ],
  };
  const document = {
    objects: [{ id: "fence", node: "asset:timber:part", transform, group: "placed-fence" }],
    groups: [{ id: "placed-fence", transform: groupTransform }],
    assetSources: [{ id: "timber", model_sha256: "b".repeat(64) }],
  } as unknown as import("./level3d.ts").Level3D;
  verifyStaticStateReplacements(c, document);
  for (const mutate of [
    (d: typeof document) => {
      d.objects[0]!.node = "asset:other:part";
    },
    (d: typeof document) => {
      d.assetSources![0]!.model_sha256 = "c".repeat(64);
    },
    (d: typeof document) => {
      d.objects[0]!.transform.dx += 1;
    },
    (d: typeof document) => {
      d.groups[0]!.transform.rot_deg += 1;
    },
  ]) {
    const changed = structuredClone(document);
    mutate(changed);
    assert.throws(() => verifyStaticStateReplacements(c, changed), /displayed placement/);
  }
});

test("loop preview timeline uses the selected loop while other effects keep independent periods", async () => {
  const { validateNativeLoopPreview, nativeLoopPreviewPeriod } =
    await import("./state-delivery.ts");
  const native = deliveryFixture().native;
  native.elements[0]!.active = true;
  native.elements[0]!.loop = true;
  native.elements.push({
    ...structuredClone(native.elements[0]!),
    id: "ambient",
    source: { kind: "map-animation", index: 0, sha256: "b".repeat(64) },
    creation_order: 1,
    frames: [{ ...native.elements[0]!.frames[0]!, delay: 8 }],
  });
  const c = {
    version: 1 as const,
    scope: "controlled-native-loop-preview" as const,
    native,
    focus_element_id: "body",
  };
  validateNativeLoopPreview(c);
  assert.equal(nativeLoopPreviewPeriod(c), 6);
  c.native.elements[0]!.loop = false;
  assert.throws(() => validateNativeLoopPreview(c), /visible looping focus/);
});

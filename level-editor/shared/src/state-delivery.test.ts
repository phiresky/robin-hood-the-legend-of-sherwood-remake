import test from "node:test";
import assert from "node:assert/strict";
import {
  validateStateDelivery,
  stateDeliveryLoopsAfterTransition,
  physicalEndpointSources,
  validateNativeLoopPreview,
  nativeLoopPreviewPeriod,
  type StateDeliveryContract,
} from "./state-delivery.ts";

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
  const source = physicalEndpointSources(c.families[0]!.physical.initial)[0]!;
  source.position = [1, -2, 3];
  validateStateDelivery(c);
  source.position = [1, NaN, 3];
  assert.throws(() => validateStateDelivery(c), /physical source/);
  source.position = [1, 2] as unknown as [number, number, number];
  assert.throws(() => validateStateDelivery(c), /physical source/);
});

test("an explicitly absent endpoint is valid but an unprepared empty model list is not", () => {
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

test("loop preview timeline uses the selected loop while other effects keep independent periods", () => {
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

test("patch-only branch requires a distinct initial target and owns its phase timing", () => {
  const c = deliveryFixture(),
    e = c.native.elements[0]!,
    frame = e.frames[0]!;
  e.initial_frame = frame;
  e.frames = [];
  c.native.patch_states = [
    {
      id: "bag",
      source: { kind: "mission-patch", index: 0, sha256: "b".repeat(64) },
      profile: { path: "profile.json", sha256: "c".repeat(64), name: "net", center: [0, 0] },
      integrate_in_background: false,
      elevation: 1,
      layer: "ordered",
      display_position: [0, 0],
      sort_position: [0, 0],
      display_order: 1,
      creation_order: 1,
      polyline: [],
      definitive: false,
      initial: [],
      transition: [frame],
      final: [frame],
      initial_loop: true,
      final_loop: true,
    },
  ];
  const f = c.families[0]!;
  f.element_ids = [];
  f.hidden_initial_element_ids = [e.id];
  f.patch_ids = ["bag"];
  f.body_terminal_tick = 2;
  validateStateDelivery(c);
  f.body_terminal_tick = 3;
  assert.throws(() => validateStateDelivery(c), /timing/);
  f.body_terminal_tick = 2;
  f.patch_ids.push("bag");
  assert.throws(() => validateStateDelivery(c), /shared patch/);
  f.patch_ids.pop();
  c.families.push({ ...structuredClone(f), id: "other" });
  assert.throws(() => validateStateDelivery(c), /hidden initial target/);
});

test("only a declared nonempty final loop continues after the transition", () => {
  const c = deliveryFixture(),
    f = c.families[0]!,
    frame = c.native.elements[0]!.frames[0]!;
  assert.equal(stateDeliveryLoopsAfterTransition(c, f.id), false);
  c.native.background_states = [
    {
      id: "receiver",
      source: { kind: "mission-patch", index: 0, sha256: "b".repeat(64) },
      display_position: [0, 0],
      restore_bounds: [0, 0, 1, 1],
      definitive: false,
      initial: [],
      transition: [frame],
      final: [frame],
      initial_loop: false,
      final_loop: true,
    },
  ];
  assert.equal(stateDeliveryLoopsAfterTransition(c, f.id), false);
  f.background_ids = ["receiver"];
  assert.equal(stateDeliveryLoopsAfterTransition(c, f.id), true);
  c.native.background_states[0]!.final_loop = false;
  assert.equal(stateDeliveryLoopsAfterTransition(c, f.id), false);
  c.native.background_states[0]!.final_loop = true;
  c.native.background_states[0]!.final = [];
  assert.equal(stateDeliveryLoopsAfterTransition(c, f.id), false);
  assert.throws(() => stateDeliveryLoopsAfterTransition(c, "missing"), /Unknown state family/);
});

test("source-only patches require one focus, preserve timing, and never fabricate endpoints", async () => {
  const { validateNativePatchPreview, nativePatchPreviewTerminal, nativePatchPreviewLoops } =
    await import("./state-delivery.ts");
  const native = deliveryFixture().native,
    frame = native.background;
  native.elements = [];
  native.patch_states = [
    {
      id: "focus",
      source: { kind: "mission-patch", index: 0, sha256: "a".repeat(64) },
      profile: { path: "profile.json", sha256: "a".repeat(64), name: "cover", center: [0, 0] },
      integrate_in_background: true,
      activation: "phases",
      restore_bounds: [0, 0, 1, 1],
      elevation: 0,
      layer: "background",
      display_position: [0, 0],
      sort_position: [0, 0],
      display_order: 0,
      creation_order: 0,
      polyline: [],
      definitive: true,
      initial: [frame],
      transition: [frame, frame],
      final: [],
      initial_loop: true,
      final_loop: true,
    },
  ];
  const contract = {
    version: 1,
    kind: "native-patch",
    scope: "Source artwork only",
    native,
    focus_patch_id: "focus",
  };
  validateNativePatchPreview(contract);
  assert.equal(nativePatchPreviewTerminal(contract), 5);
  assert.equal(nativePatchPreviewLoops(contract), false);
  native.patch_states[0]!.final = [frame];
  assert.equal(nativePatchPreviewLoops(contract), true);
  assert.throws(
    () => validateNativePatchPreview({ ...contract, focus_patch_id: "missing" }),
    /controlled focus/,
  );
  assert.throws(
    () => validateNativePatchPreview({ ...contract, physical: { kind: "absent" } }),
    /Invalid/,
  );
  native.background = { ...native.background, width: 3 };
  const other = structuredClone(native.patch_states[0]!);
  other.id = "context";
  other.source.index = 1;
  other.creation_order = 1;
  other.display_position = [2, 0];
  other.sort_position = [2, 0];
  other.restore_bounds = [2, 0, 1, 1];
  native.patch_states.push(other);
  assert.throws(() => validateNativePatchPreview(contract), /controlled focus/);
  other.activation = "initial-only";
  validateNativePatchPreview(contract);
});

test("discrete patch frames preserve complete multi-model endpoints and independent patch clocks", async () => {
  const { StateEventClocks } = await import("./state-event-clocks.ts");
  const c = deliveryFixture(),
    f = c.families[0]!,
    frame = c.native.background;
  c.native.elements = [];
  c.native.patch_states = [
    {
      id: "fence",
      source: { kind: "mission-patch", index: 7, sha256: "b".repeat(64) },
      profile: { path: "profile.json", sha256: "c".repeat(64), name: "fence", center: [0, 0] },
      integrate_in_background: true,
      activation: "phases",
      restore_bounds: [0, 0, 1, 1],
      elevation: 0,
      layer: "background",
      display_position: [0, 0],
      sort_position: [0, 0],
      display_order: 0,
      creation_order: 0,
      polyline: [],
      definitive: true,
      initial: [frame],
      transition: [frame],
      final: [],
      initial_loop: true,
      final_loop: true,
    },
  ];
  f.element_ids = [];
  f.patch_ids = ["fence"];
  f.body_terminal_tick = 2;
  const applied = physicalEndpointSources(f.physical.applied);
  applied.push({
    ...structuredClone(applied[0]!),
    id: "ground",
    role: "ground",
    model: "ground.glb",
  });
  f.physical.motion = {
    kind: "native-patch-model-frames",
    patch_id: "fence",
    source_motion_sha256: "d".repeat(64),
    model_frames: [structuredClone(applied)],
  };
  validateStateDelivery(c);
  const clocks = new StateEventClocks(c);
  for (const [sequence, phase] of (
    ["initial", "forward", "applied", "initial"] as const
  ).entries()) {
    clocks.apply({
      mission: c.native.mission,
      mission_data_sha256: c.native.mission_data_sha256,
      level_data_sha256: c.native.level_data_sha256,
      sequence,
      simulation_tick: sequence * 100,
      samples: [
        { kind: "patch", id: "fence", source: c.native.patch_states[0]!.source, phase, tick: 0 },
      ],
    });
    const pose = clocks.physicalPose(f.id);
    if (phase === "forward") assert.equal(pose.model_frame, 0);
    else assert.equal(pose.endpoint, phase === "applied" ? "applied" : "initial");
  }
  const missingGround = structuredClone(c);
  if (missingGround.families[0]!.physical.motion!.kind !== "native-patch-model-frames")
    throw Error("fixture");
  missingGround.families[0]!.physical.motion!.model_frames[0]!.pop();
  assert.throws(() => validateStateDelivery(missingGround), /terminal model frame differs/);
  const duplicate = structuredClone(c);
  if (duplicate.families[0]!.physical.motion!.kind !== "native-patch-model-frames")
    throw Error("fixture");
  duplicate.families[0]!.physical.motion!.model_frames[0]!.push(structuredClone(applied[0]!));
  assert.throws(() => validateStateDelivery(duplicate), /physical source/);
  for (const motion of [
    {
      kind: "native-patch-translation" as const,
      patch_id: "fence",
      source_motion_sha256: "d".repeat(64),
      translation_frames: [[0, 0, 0] as [number, number, number]],
    },
    {
      kind: "native-patch-node-poses" as const,
      patch_id: "fence",
      source_motion_sha256: "d".repeat(64),
      node_frames: [[]],
    },
  ]) {
    const invalid = structuredClone(c);
    invalid.families[0]!.physical.motion = motion;
    assert.throws(() => validateStateDelivery(invalid), /invalid reviewed patch motion binding/);
  }
});

test("patch clip contracts pin every phase and preserve separate clocks", async () => {
  const { physicalPatchClipPose } = await import("./state-delivery.ts");
  const { StateEventClocks } = await import("./state-event-clocks.ts");
  const c = deliveryFixture(),
    f = c.families[0]!,
    frame = c.native.background;
  c.native.elements = [];
  f.element_ids = [];
  f.patch_ids = ["net"];
  f.body_terminal_tick = 5;
  c.native.patch_states = [
    {
      id: "net",
      source: { kind: "mission-patch", index: 0, sha256: "b".repeat(64) },
      profile: { path: "net.json", sha256: "c".repeat(64), name: "net", center: [0, 0] },
      integrate_in_background: false,
      elevation: 1,
      layer: "ordered",
      display_position: [0, 0],
      sort_position: [0, 0],
      display_order: 0,
      creation_order: 0,
      polyline: [],
      definitive: true,
      initial: [],
      transition: [frame, frame],
      final: Array(14).fill(frame),
      initial_loop: true,
      final_loop: true,
    },
  ];
  const binding = physicalEndpointSources(f.physical.applied)[0]!;
  f.physical.motion = {
    kind: "native-patch-clips",
    patch_id: "net",
    source_motion_sha256: "c".repeat(64),
    transition: { binding: { ...binding, id: "lift" }, clip: "lift", duration_ticks: 6 },
    final: { binding, clip: "sway", duration_ticks: 42 },
  };
  f.physical.motion.transition.material_frames = [0, 1].map((i) => ({
    duration_ticks: 3,
    materials: [
      {
        node: "net",
        slot: 0,
        material_index: i,
        material_name: `frame${i}`,
        texture_index: i,
        tex_coord: 1,
        alpha_mode: "OPAQUE",
      },
    ],
  }));
  validateStateDelivery(c);
  for (const change of [
    (frames: NonNullable<import("./state-delivery.ts").PhysicalPatchClip["material_frames"]>) =>
      frames.pop(),
    (frames: NonNullable<import("./state-delivery.ts").PhysicalPatchClip["material_frames"]>) =>
      frames[0]!.duration_ticks++,
    (frames: NonNullable<import("./state-delivery.ts").PhysicalPatchClip["material_frames"]>) =>
      frames[0]!.materials.push(frames[0]!.materials[0]!),
    (frames: NonNullable<import("./state-delivery.ts").PhysicalPatchClip["material_frames"]>) =>
      (frames[0]!.materials[0]!.node = "foreign"),
  ]) {
    const bad = structuredClone(c);
    const motion = bad.families[0]!.physical.motion;
    assert.ok(motion?.kind === "native-patch-clips");
    change(motion.transition.material_frames!);
    assert.throws(() => validateStateDelivery(bad), /material frame/);
  }
  assert.equal(physicalPatchClipPose(c, f.id, "forward", 4).clip_tick, 4);
  assert.equal(physicalPatchClipPose(c, f.id, "forward", 5).clip_tick, 0);
  assert.equal(physicalPatchClipPose(c, f.id, "applied", 49).clip_tick, 7);
  const clocks = new StateEventClocks(c);
  clocks.apply({
    mission: c.native.mission,
    mission_data_sha256: c.native.mission_data_sha256,
    level_data_sha256: c.native.level_data_sha256,
    sequence: 0,
    simulation_tick: 100,
    samples: [
      {
        kind: "patch",
        id: "net",
        source: c.native.patch_states[0]!.source,
        phase: "applied",
        tick: 49,
      },
    ],
  });
  assert.equal(clocks.physicalPose(f.id).clip_tick, 7);
  for (const mutate of [
    (m: typeof f.physical.motion) => {
      if (m?.kind === "native-patch-clips") m.transition.duration_ticks = 5;
    },
    (m: typeof f.physical.motion) => {
      if (m?.kind === "native-patch-clips") m.transition.binding.model_sha256 = "bad";
    },
    (m: typeof f.physical.motion) => {
      if (m?.kind === "native-patch-clips") m.final = { kind: "absent" };
    },
  ]) {
    const bad = structuredClone(c);
    mutate(bad.families[0]!.physical.motion);
    assert.throws(() => validateStateDelivery(bad));
  }
  c.native.patch_states[0]!.final = [];
  f.physical.applied = { kind: "absent" };
  f.physical.motion.final = { kind: "absent" };
  validateStateDelivery(c);
  assert.equal(physicalPatchClipPose(c, f.id, "applied", 0).visible, false);
});

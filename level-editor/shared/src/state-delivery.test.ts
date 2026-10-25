import test from "node:test";
import assert from "node:assert/strict";
import {
  validateStateDelivery,
  stateDeliveryLoopsAfterTransition,
  physicalEndpointSources,
  validateNativeLoopPreview,
  nativeLoopPreviewPeriod,
  nativeRemainderFrame,
  nativeRemainderNodeFrame,
  physicalPatchPose,
  physicalFamilyEndpoints,
  nativeElementFrameStartTick,
  verifyStateGameplayLinks,
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
  // A transient effect may disappear after its last authored transition frame.
  const absent = structuredClone(c);
  absent.families[0]!.physical.applied = { kind: "absent" };
  validateStateDelivery(absent);
  assert.deepEqual(physicalPatchPose(absent, f.id, 0), {
    endpoint: "initial",
    translation: [0, 0, 0],
    model_frame: 0,
  });
  const terminal = physicalPatchPose(absent, f.id, f.body_terminal_tick);
  assert.equal(terminal.endpoint, "applied");
  assert.equal(terminal.model_frame, undefined);
  assert.deepEqual(physicalEndpointSources(absent.families[0]!.physical.applied), []);
  const absentClocks = new StateEventClocks(absent);
  for (const [sequence, phase] of (["forward", "applied", "initial"] as const).entries()) {
    absentClocks.apply({
      mission: absent.native.mission,
      mission_data_sha256: absent.native.mission_data_sha256,
      level_data_sha256: absent.native.level_data_sha256,
      sequence,
      simulation_tick: sequence * 100,
      samples: [
        {
          kind: "patch",
          id: "fence",
          source: absent.native.patch_states![0]!.source,
          phase,
          tick: 0,
        },
      ],
    });
    const pose = absentClocks.physicalPose(f.id);
    if (phase === "forward") assert.equal(pose.model_frame, 0);
    else {
      assert.equal(pose.endpoint, phase === "applied" ? "applied" : "initial");
      assert.equal(pose.model_frame, undefined);
    }
  }
  for (const endpoint of [[], { kind: "absent", extra: true }, null]) {
    const malformed = structuredClone(absent);
    malformed.families[0]!.physical.applied = endpoint as never;
    assert.throws(() => validateStateDelivery(malformed), /physical endpoint|patch motion binding/);
  }
  const absentTranslation = structuredClone(absent);
  absentTranslation.families[0]!.physical.motion = {
    kind: "native-patch-translation",
    patch_id: "fence",
    source_motion_sha256: "d".repeat(64),
    translation_frames: [[0, 0, 0]],
  };
  assert.throws(
    () => validateStateDelivery(absentTranslation),
    /invalid reviewed patch motion binding/,
  );
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
  c.native.patch_states[0]!.initial = [{ ...frame }];
  assert.throws(() => validateStateDelivery(c), /transparent initial frame proof/);
  f.physical.motion.transparent_initial_frames = structuredClone(c.native.patch_states[0]!.initial);
  for (const key of ["path", "sha256", "width", "height", "offset", "delay"] as const) {
    const bad = structuredClone(c);
    const motion = bad.families[0]!.physical.motion;
    assert.ok(motion?.kind === "native-patch-clips");
    const proof = motion.transparent_initial_frames![0]!;
    if (key === "path") proof.path = "foreign.png";
    else if (key === "sha256") proof.sha256 = "f".repeat(64);
    else if (key === "offset") proof.offset = [1, 0];
    else proof[key]++;
    assert.throws(() => validateStateDelivery(bad), /transparent initial frame proof/);
  }
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

test("native remainder binds every source frame of one family element to a pinned mask or null", () => {
  const c = deliveryFixture();
  const mask = { path: "mask.png", sha256: "b".repeat(64), width: 1, height: 1 };
  const family = c.families[0]!;
  family.native_remainder = [
    {
      element_id: "body",
      encoding: "source-domain-v1",
      drawn_values: [2, 3],
      priority: "remainder-over-physical",
      frames: [null, { mask, physical: "applied" }],
    },
  ];
  validateStateDelivery(c);
  const variant = (edit: (r: NonNullable<typeof family.native_remainder>[number]) => void) => {
    const bad = structuredClone(c);
    edit(bad.families[0]!.native_remainder![0]!);
    return bad;
  };
  const rejects = [
    variant((r) => (r.element_id = "missing")),
    variant((r) => (r.drawn_values = [3])),
    variant((r) => (r.drawn_values = [1, 2] as unknown as (2 | 3)[])),
    variant((r) => (r.drawn_values = [3, 2])),
    variant((r) => (r.drawn_values = [2, 2])),
    variant((r) => (r.priority = "physical-over-remainder" as "remainder-over-physical")),
    variant((r) => (r.encoding = "other" as "source-domain-v1")),
    variant((r) => Object.assign(r, { extra: true })),
  ];
  for (const bad of rejects)
    assert.throws(() => validateStateDelivery(bad), /native remainder binding/);
  for (const bad of [variant((r) => r.frames.pop()), variant((r) => (r.frames = [null, null]))])
    assert.throws(() => validateStateDelivery(bad), /remainder frames differ/);
  for (const bad of [
    variant((r) => (r.frames[1]!.mask.width = 2)),
    variant((r) => (r.frames[1]!.mask.sha256 = "B".repeat(64))),
    variant((r) => (r.frames[1]!.mask.path = "../mask.png")),
    variant((r) => (r.frames[0] = { mask, physical: "applied" })),
    variant((r) => Object.assign(r.frames[1]!, { tint: 1 })),
  ])
    assert.throws(() => validateStateDelivery(bad), /remainder mask/);
  const unpaired = variant((r) => (r.frames[1]!.physical = "initial" as "applied"));
  assert.throws(() => validateStateDelivery(unpaired), /physical pairing/);
  const absent = structuredClone(c);
  absent.families[0]!.physical.applied = { kind: "absent" };
  assert.throws(() => validateStateDelivery(absent), /physical pairing/);
  const duplicate = structuredClone(c);
  duplicate.families[0]!.native_remainder!.push(
    structuredClone(duplicate.families[0]!.native_remainder![0]!),
  );
  assert.throws(() => validateStateDelivery(duplicate), /native remainder binding/);
  const empty = structuredClone(c);
  empty.families[0]!.native_remainder = [];
  assert.throws(() => validateStateDelivery(empty), /invalid native remainder/);
  const moving = structuredClone(c);
  moving.families[0]!.physical.motion = {
    kind: "native-element-node-poses",
    element_id: "body",
    source_motion_sha256: "a".repeat(64),
    node_frames: [0, 1].map(() => [
      { node: "lid", matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1] },
    ]),
  };
  const stationary = structuredClone(moving);
  delete stationary.families[0]!.native_remainder;
  validateStateDelivery(stationary);
  assert.throws(() => validateStateDelivery(moving), /physical pairing/);
});

test("native remainder selection returns only bound source frames", () => {
  const c = deliveryFixture();
  const mask = { path: "mask.png", sha256: "b".repeat(64), width: 1, height: 1 };
  c.families[0]!.native_remainder = [
    {
      element_id: "body",
      encoding: "source-domain-v1",
      drawn_values: [2],
      priority: "remainder-over-physical",
      frames: [null, { mask, physical: "applied" }],
    },
  ];
  assert.equal(nativeRemainderFrame(c.families[0]!, "body", 0), undefined);
  assert.equal(nativeRemainderFrame(c.families[0]!, "other", 1), undefined);
  assert.deepEqual(nativeRemainderFrame(c.families[0]!, "body", 1)?.mask, mask);
  delete c.families[0]!.native_remainder;
  assert.equal(nativeRemainderFrame(c.families[0]!, "body", 1), undefined);
});

/** One target with four 3-tick frames: native frames 0..1 (ticks 0..5), clip from frame 2 (tick 6). */
export function elementClipFixture(): StateDeliveryContract {
  const c = deliveryFixture();
  const element = c.native.elements[0]!;
  element.frames = [0, 1, 2, 3].map(() => ({ ...element.frames[0]! }));
  const family = c.families[0]!;
  family.body_terminal_tick = 9;
  const applied = (family.physical.applied as { position?: number[] }[])[0]!;
  applied.position = [1, 2, 3];
  family.physical.motion = {
    kind: "native-element-clip",
    element_id: "body",
    source_motion_sha256: "b".repeat(64),
    start_frame: 2,
    binding: {
      id: "clip",
      role: "objects",
      model: "clip.glb",
      model_sha256: "c".repeat(64),
      resources: [],
      position: [1, 2, 3],
    },
    clip: "fall",
    duration_ticks: 3,
  };
  return c;
}

test("target clip keeps native frames until its start frame, then samples the clip on the target clock", () => {
  const c = elementClipFixture();
  validateStateDelivery(c);
  assert.equal(nativeElementFrameStartTick(c.native.elements[0]!.frames, 2), 6);
  const endpoints = physicalFamilyEndpoints(c.families[0]!);
  assert.equal(endpoints.length, 3);
  assert.deepEqual(
    physicalEndpointSources(endpoints[2]!).map((s) => s.id),
    ["clip"],
  );
  assert.deepEqual(physicalPatchPose(c, "trap"), { endpoint: "initial", translation: [0, 0, 0] });
  for (const tick of [0, 3, 5])
    assert.deepEqual(physicalPatchPose(c, "trap", tick), {
      endpoint: "applied",
      translation: [0, 0, 0],
      visible: false,
      native_element: true,
    });
  for (const tick of [6, 7, 8])
    assert.deepEqual(physicalPatchPose(c, "trap", tick), {
      endpoint: "applied",
      translation: [0, 0, 0],
      model_frame: 0,
      clip_phase: "transition",
      clip_tick: tick,
    });
  for (const tick of [9, 40])
    assert.deepEqual(physicalPatchPose(c, "trap", tick), {
      endpoint: "applied",
      translation: [0, 0, 0],
    });
});

test("target clip validation is exact about keys, start frame, timing, placement and owner", () => {
  type Clip = Extract<
    NonNullable<StateDeliveryContract["families"][number]["physical"]["motion"]>,
    { kind: "native-element-clip" }
  >;
  const reject = (edit: (c: StateDeliveryContract, m: Clip) => void, message: RegExp) => {
    const c = elementClipFixture();
    edit(c, c.families[0]!.physical.motion as Clip);
    assert.throws(() => validateStateDelivery(c), message);
  };
  reject((_, m) => Object.assign(m, { clip_start_tick: 6 }), /target clip binding/);
  reject((_, m) => delete (m as Partial<Clip>).duration_ticks, /target clip binding/);
  reject((_, m) => (m.source_motion_sha256 = "x"), /target clip binding/);
  reject((_, m) => (m.element_id = "other"), /target clip binding/);
  reject((c) => (c.families[0]!.patch_ids = ["p"]), /target clip binding|patch state/);
  reject((c) => (c.native.elements[0]!.loop = true), /transition element/);
  reject((c) => (c.families[0]!.physical.applied = { kind: "absent" }), /target clip binding/);
  for (const start of [0, 4, 1.5, -1])
    reject((c, m) => {
      m.start_frame = start;
      m.duration_ticks = 9 - 3 * Math.trunc(start);
    }, /start frame/);
  reject((_, m) => (m.duration_ticks = 4), /timing differs/);
  reject((_, m) => (m.duration_ticks = 0), /timing differs/);
  reject((_, m) => (m.clip = " "), /clip name/);
  reject((_, m) => (m.binding.id = "applied"), /placement differs/);
  reject((_, m) => (m.binding.position = [1, 2, 4]), /placement differs/);
  reject((_, m) => delete m.binding.position, /placement differs/);
  reject((_, m) => (m.binding.role = "ground"), /placement differs/);
  reject((_, m) => (m.binding.model_sha256 = "short"), /physical source/);
  reject(
    (c) =>
      (c.families[0]!.native_remainder = [
        {
          element_id: "body",
          encoding: "source-domain-v1",
          drawn_values: [2],
          priority: "remainder-over-physical",
          frames: [null, null, null, null],
        },
      ]),
    /physical pairing/,
  );
});

function linkFixture() {
  const c = deliveryFixture();
  const frame = c.native.elements[0]!.frames[0]!;
  c.native.background_states = [
    {
      id: "receiver",
      source: { kind: "mission-patch", index: 0, sha256: "b".repeat(64) },
      display_position: [0, 0],
      restore_bounds: [0, 0, 1, 1],
      definitive: true,
      initial: [],
      transition: [frame],
      final: [],
      initial_loop: false,
      final_loop: false,
    },
  ];
  c.families[0]!.background_ids = ["receiver"];
  c.families[0]!.gameplay_transition_ids = ["west/west/movement-change-6"];
  const compiled = {
    movement_transitions: [
      { id: "west/west/movement-change-6", definitive: true, initial_sight: [1] },
      { id: "central/central/movement-change-5", definitive: true, initial_sight: [0] },
    ],
    sight_obstacles: [{}, {}],
  };
  return { c, compiled };
}

test("gameplay transition links are well formed, unique across families and bound to a state", () => {
  const { c } = linkFixture();
  validateStateDelivery(c);
  const reject = (edit: (c: StateDeliveryContract) => void, message: RegExp) => {
    const bad = structuredClone(c);
    edit(bad);
    assert.throws(() => validateStateDelivery(bad), message);
  };
  for (const ids of [[], [""], ["no-slash"], ["a/"], ["/a"], ["a//b"], ["a/b", "a/b"], [7]])
    reject(
      (bad) => (bad.families[0]!.gameplay_transition_ids = ids as string[]),
      /invalid gameplay transition links/,
    );
  reject(
    (bad) => (bad.families[0]!.gameplay_transition_ids = "a/b" as unknown as string[]),
    /invalid gameplay transition links/,
  );
  reject((bad) => (bad.families[0]!.background_ids = []), /require a bound background/);
  reject((bad) => {
    const e = bad.native.elements[0]!;
    bad.native.elements.push({
      ...e,
      id: "other",
      source: { ...e.source, index: 1, sha256: "c".repeat(64) },
      creation_order: 1,
      display_order: 1,
    });
    const r = bad.native.background_states![0]!;
    bad.native.background_states!.push({
      ...r,
      id: "r2",
      source: { ...r.source, index: 1, sha256: "d".repeat(64) },
      display_position: [5, 5],
      restore_bounds: [5, 5, 1, 1],
    });
    bad.families.push({
      ...structuredClone(bad.families[0]!),
      id: "other",
      element_ids: ["other"],
      background_ids: ["r2"],
    });
  }, /linked by two families/);
});

test("gameplay links resolve to one compiled, retained transition of the same permanence", () => {
  const { c, compiled } = linkFixture();
  assert.deepEqual(verifyStateGameplayLinks(c, compiled, []), [
    {
      family: "trap",
      transition_id: "west/west/movement-change-6",
      definitive: true,
      initial_sight: [1],
      applied_sight: [],
    },
  ]);
  assert.deepEqual(verifyStateGameplayLinks(deliveryFixture(), undefined, []), []);
  assert.throws(() => verifyStateGameplayLinks(c, undefined, []), /requires compiled/);
  assert.throws(
    () => verifyStateGameplayLinks(c, { movement_transitions: compiled.movement_transitions }, []),
    /requires compiled/,
  );
  assert.throws(
    () => verifyStateGameplayLinks(c, { ...compiled, movement_transitions: [] }, []),
    /does not resolve/,
  );
  const aliased = structuredClone(compiled);
  aliased.movement_transitions.push({
    id: "copy",
    aliases: ["west/west/movement-change-6"],
  } as never);
  assert.throws(() => verifyStateGameplayLinks(c, aliased, []), /does not resolve/);
  const renamed = structuredClone(compiled);
  renamed.movement_transitions[0]!.id = "canonical/id";
  Object.assign(renamed.movement_transitions[0]!, { aliases: ["west/west/movement-change-6"] });
  assert.equal(verifyStateGameplayLinks(c, renamed, [])[0]!.transition_id, "canonical/id");
  assert.throws(
    () =>
      verifyStateGameplayLinks(c, compiled, [
        "Transition west/west/movement-change-6: control omitted; retained its initial state; x",
      ]),
    /omitted/,
  );
  const temporary = structuredClone(compiled);
  temporary.movement_transitions[0]!.definitive = false;
  assert.throws(() => verifyStateGameplayLinks(c, temporary, []), /permanence/);
  const outside = structuredClone(compiled);
  outside.movement_transitions[0]!.initial_sight = [2];
  assert.throws(() => verifyStateGameplayLinks(c, outside, []), /outside the compiled map/);
});

test("contracts without the new fields keep their validation and poses unchanged", () => {
  const plain = deliveryFixture();
  const before = JSON.stringify(plain);
  validateStateDelivery(plain);
  assert.equal(JSON.stringify(plain), before);
  assert.deepEqual(physicalFamilyEndpoints(plain.families[0]!).length, 2);
  // Unknown family keys stay ignored exactly as before; only the declared new fields are checked.
  const unknown = structuredClone(plain);
  Object.assign(unknown.families[0]!, { gameplay_transition_id: "bogus" });
  validateStateDelivery(unknown);
  const nodes = deliveryFixture();
  nodes.families[0]!.physical.motion = {
    kind: "native-element-node-poses",
    element_id: "body",
    source_motion_sha256: "b".repeat(64),
    node_frames: [0, 1].map(() => [
      { node: "n", matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1] },
    ]),
  };
  validateStateDelivery(nodes);
  assert.deepEqual(
    [undefined, 0, 2, 3].map((t) => physicalPatchPose(nodes, "trap", t)),
    [
      { endpoint: "initial", translation: [0, 0, 0] },
      { endpoint: "applied", translation: [0, 0, 0], node_frame: 0 },
      { endpoint: "applied", translation: [0, 0, 0], node_frame: 0 },
      { endpoint: "applied", translation: [0, 0, 0] },
    ],
  );
});

test("moving remainders bind the exact source owner and pose, including flattened staged frames", () => {
  const c = deliveryFixture();
  const f = c.families[0]!;
  const identity = [{ node: "body", matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1] }];
  f.physical.motion = {
    kind: "native-element-node-poses",
    element_id: "body",
    source_motion_sha256: "c".repeat(64),
    node_frames: [identity, identity],
  };
  const pairing = { kind: "source-frame-node-pose" as const, element_id: "body", frame: 1 };
  f.native_remainder = [
    {
      element_id: "body",
      encoding: "source-domain-v1",
      drawn_values: [2, 3],
      priority: "remainder-over-physical",
      frames: [
        null,
        {
          mask: { path: "mask.png", sha256: "b".repeat(64), width: 1, height: 1 },
          physical: pairing,
        },
      ],
    },
  ];
  validateStateDelivery(c);
  assert.equal(nativeRemainderNodeFrame(f, "body", 1, pairing), 1);
  for (const wrong of [
    { ...pairing, frame: 0 },
    { ...pairing, element_id: "other" },
    { ...pairing, extra: true },
    "applied" as const,
  ]) {
    const bad = structuredClone(c);
    bad.families[0]!.native_remainder![0]!.frames[1]!.physical = wrong;
    assert.throws(() => validateStateDelivery(bad), /physical pairing/);
  }
  f.physical.motion = {
    kind: "native-element-stage-poses",
    source_motion_sha256: "c".repeat(64),
    stages: [
      { element_id: "early", node_frames: [identity, null, identity] },
      { element_id: "body", node_frames: [null, identity] },
    ],
  };
  assert.equal(nativeRemainderNodeFrame(f, "body", 1, pairing), 2);
  assert.throws(
    () => nativeRemainderNodeFrame(f, "body", 0, { ...pairing, frame: 0 }),
    /physical pairing/,
  );
  delete f.physical.motion;
  assert.throws(() => nativeRemainderNodeFrame(f, "body", 1, pairing), /physical pairing/);
});

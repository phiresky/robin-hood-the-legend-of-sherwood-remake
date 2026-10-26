import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { componentClipFixture } from "../../shared/test-fixtures/component-clips.ts";
import { captureStateAppearance } from "./state-appearance-player.ts";
import {
  ComponentClips,
  assertComponentClipTerminal,
  componentMeshDrawable,
} from "./component-clips.ts";

function fixture() {
  const f = componentClipFixture(),
    root = new THREE.Group(),
    applied = new THREE.Group();
  const material = new THREE.MeshBasicMaterial({ color: 0x806040 });
  const old = new THREE.Mesh(new THREE.BoxGeometry(), material);
  old.name = "old";
  root.add(old);
  const geometry = new THREE.BoxGeometry();
  geometry.morphTargetsRelative = true;
  geometry.morphAttributes.position = [
    new THREE.Float32BufferAttribute(
      Array.from({ length: geometry.getAttribute("position").count }, () => [0.5, 0, 0]).flat(),
      3,
    ),
  ];
  const wreck = new THREE.Mesh(geometry, material);
  wreck.name = "wreck";
  root.add(wreck);
  const end = new THREE.Mesh(new THREE.BoxGeometry().translate(0.5, 0, 0), material.clone());
  end.name = "wreck";
  applied.add(end);
  const times = [0, 1 / 25, 4 / 25];
  const clip = new THREE.AnimationClip("collapse", -1, [
    new THREE.VectorKeyframeTrack(
      "old.scale",
      times,
      [1, 1, 1, 0, 0, 0, 0, 0, 0],
      THREE.InterpolateDiscrete,
    ),
    new THREE.VectorKeyframeTrack(
      "wreck.scale",
      times,
      [0, 0, 0, 1, 1, 1, 1, 1, 1],
      THREE.InterpolateDiscrete,
    ),
    new THREE.NumberKeyframeTrack(
      "wreck.morphTargetInfluences",
      times,
      [0, 0.5, 1],
      THREE.InterpolateDiscrete,
    ),
  ]);
  const template = captureStateAppearance(root, [clip]);
  return { ...f, root, applied, old, wreck, end, clip, template };
}
test("component sampler preserves morphs, explicit visibility, hidden compositor roots and backwards seek", () => {
  const f = fixture(),
    clips = new ComponentClips(f.applied, f.motion, [f.template]);
  const root = clips.roots.get("cart")!;
  const parent = new THREE.Group();
  parent.visible = false;
  parent.add(root);
  for (const [tick, active, weight] of [
    [0, "old", 0],
    [1, "wreck", 0.5],
    [4, "wreck", 1],
    [0, "old", 0],
  ] as const) {
    clips.apply([{ component_id: "body", element_id: "cart", clip_tick: tick }]);
    const old = root.getObjectByName("old") as THREE.Mesh,
      wreck = root.getObjectByName("wreck") as THREE.Mesh;
    assert.equal(old.visible, active === "old");
    assert.equal(wreck.visible, active === "wreck");
    assert.equal(wreck.morphTargetInfluences![0], weight);
  }
  clips.reset();
  assert.equal(root.visible, false);
  clips.dispose();
});
test("evaluated terminal comparison accepts declared zero-scale retirement and rejects visible defects", () => {
  const mutations = [
    ["retired visible", (f: ReturnType<typeof fixture>) => f.old.scale.setScalar(1), /visibility/],
    ["retired plane", (f: ReturnType<typeof fixture>) => f.old.scale.set(1, 1, 0), /visibility/],
    ["missing active", (f: ReturnType<typeof fixture>) => (f.wreck.visible = false), /visibility/],
    [
      "bad morph",
      (f: ReturnType<typeof fixture>) => (f.wreck.morphTargetInfluences![0] = 0),
      /evaluated terminal/,
    ],
    [
      "bad anchor",
      (f: ReturnType<typeof fixture>) => (f.root.position.x = 1),
      /evaluated terminal/,
    ],
    [
      "extra mesh",
      (f: ReturnType<typeof fixture>) => {
        const m = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
        m.name = "extra";
        f.root.add(m);
      },
      /extraneous/,
    ],
    ["missing node", (f: ReturnType<typeof fixture>) => f.root.remove(f.wreck), /missing declared/],
    [
      "wrong material",
      (f: ReturnType<typeof fixture>) =>
        (f.wreck.material = new THREE.MeshBasicMaterial({ color: "red" })),
      /material differs/,
    ],
    [
      "bad normal",
      (f: ReturnType<typeof fixture>) => f.wreck.geometry.getAttribute("normal").setXYZ(0, 0, 1, 0),
      /normals differ/,
    ],
    [
      "hidden nonfinite",
      (f: ReturnType<typeof fixture>) => {
        f.old.visible = false;
        f.old.position.x = NaN;
      },
      /nonfinite matrix/,
    ],
    [
      "wrong UV",
      (f: ReturnType<typeof fixture>) => f.wreck.geometry.getAttribute("uv").setX(0, 0.123),
      /uv differs/,
    ],
    [
      "wrong index",
      (f: ReturnType<typeof fixture>) => f.wreck.geometry.index!.setX(0, 9),
      /indices differ/,
    ],
  ] as const;
  const good = fixture();
  good.old.scale.setScalar(0);
  good.wreck.morphTargetInfluences![0] = 1;
  assert.doesNotThrow(() => assertComponentClipTerminal(good.applied, good.root, good.component));
  for (const [name, change, pattern] of mutations) {
    const f = fixture();
    f.old.scale.setScalar(0);
    f.wreck.morphTargetInfluences![0] = 1;
    change(f);
    assert.throws(() => assertComponentClipTerminal(f.applied, f.root, f.component), pattern, name);
  }
});
test("endpoint comparison rejects changed texture pixels and one-axis-zero is still drawable", () => {
  const f = fixture();
  f.old.scale.setScalar(0);
  f.wreck.morphTargetInfluences![0] = 1;
  const a = new THREE.DataTexture(new Uint8Array([1, 2, 3, 255]), 1, 1),
    b = new THREE.DataTexture(new Uint8Array([1, 2, 4, 255]), 1, 1);
  (f.end.material as THREE.MeshBasicMaterial).map = a;
  f.wreck.material = new THREE.MeshBasicMaterial({ color: 0x806040, map: b });
  assert.throws(
    () => assertComponentClipTerminal(f.applied, f.root, f.component),
    /texture pixels/,
  );
  f.old.scale.set(0, 1, 1);
  f.root.updateMatrixWorld(true);
  assert.equal(componentMeshDrawable(f.old), true);
});
test("missing channels, wrong schedules, malformed mesh ownership and unpinned clip identity fail at load", () => {
  for (const mutation of [
    (f: ReturnType<typeof fixture>) => (f.component.clip = "missing"),
    (f: ReturnType<typeof fixture>) => (f.component.visibility_phases[0]!.active_nodes = ["wreck"]),
    (f: ReturnType<typeof fixture>) => {
      f.component.nodes.push("unbound");
      f.component.terminal.retired_nodes.push("unbound");
    },
    (f: ReturnType<typeof fixture>) => {
      f.wreck.geometry.morphAttributes.position = [];
      f.wreck.updateMorphTargets();
    },
  ]) {
    const f = fixture();
    mutation(f);
    assert.throws(() => new ComponentClips(f.applied, f.motion, [f.template]));
  }
});

test("constant or already-retired channels may hold while the pinned clip reaches its endpoint", () => {
  const f = fixture();
  f.clip.tracks[0] = new THREE.VectorKeyframeTrack(
    "old.scale",
    [0, 1 / 25],
    [1, 1, 1, 0, 0, 0],
    THREE.InterpolateDiscrete,
  );
  const template = captureStateAppearance(f.root, [f.clip]);
  const clips = new ComponentClips(f.applied, f.motion, [template]);
  clips.apply([{ component_id: "body", element_id: "cart", clip_tick: 4 }]);
  assert.equal(clips.roots.get("cart")!.getObjectByName("old")!.visible, false);
  clips.dispose();
});

import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { DEFAULT_LIGHTING, gameToScene } from "@rle/shared";
import {
  movementTransitionCompilerFixture,
  joinedTransitionCompilerFixture,
  endpointAppearanceCompilerFixture,
  unavailableTerrainControlCompilerFixture,
} from "../../shared/test-fixtures/asset-gameplay.ts";
import { validateAssetGameplay } from "../../shared/src/asset-gameplay.ts";
import {
  bindBakeAppearances,
  planAppearanceRegions,
  bakeAppearanceRegions,
  bakeAppearanceRegionsAsync,
} from "./map-appearance-bake.ts";
import { compileMap, type BakeBounds } from "./map-compile.ts";
import { bakeScene, contentBakeBounds } from "./map-bake-render.ts";
import { PatchDisplay, applyPlacementPatches } from "./patch-display.ts";

const camera = movementTransitionCompilerFixture().document.camera;

test("an unavailable terrain gate freezes its visuals and barriers without orphan state bindings", () => {
  const { document, assets } = unavailableTerrainControlCompilerFixture();
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets, { bestEffort: true });
  const geometry = compiled.descriptor.asset_geometry!;
  assert.equal(geometry.movement_transitions, undefined);
  const obstacles = geometry.motion_data.layers.flat().flatMap((area) => area.obstacles);
  assert.equal(obstacles.length, 2);
  assert.ok(obstacles.every((obstacle) => obstacle.state_id === 0));
  assert.ok(geometry.warnings?.some((warning) => warning.includes("initial visual state")));
  const root = new THREE.Group();
  root.userData.map_bake_object_id = "hut-a-body";
  const closed = mesh(10, 10, "gate-cover"),
    open = mesh(40, 10, "gate-cover");
  closed.userData = { reveal_hide_when_applied: ["gate-cover"] };
  root.add(closed, open);
  const display = new PatchDisplay();
  // An applied viewport preview must not leak into the fallback export.
  display.set("gate-cover", true);
  display.apply(root);
  assert.equal(closed.visible, false);
  assert.equal(open.visible, true);
  const snapshot = bakeScene([root]);
  const warnings: string[] = [];
  bindBakeAppearances(snapshot, document, assets, [], (message) => warnings.push(message));
  display.clear();
  display.apply(snapshot);
  assert.deepEqual(
    snapshot.children[0]!.children.map((node) => node.visible),
    [true, false],
  );
  assert.equal(closed.visible, false);
  assert.equal(open.visible, true);
  assert.deepEqual(planAppearanceRegions(snapshot, camera, [0, 0, 2000, 2000], [], false), []);
  assert.ok(warnings.some((warning) => warning.includes("initial visual state")));
  for (const node of [closed, open]) {
    node.geometry.dispose();
    if (!Array.isArray(node.material)) node.material.dispose();
  }
});

test("async appearance rendering resets state after cancellation between frames", async () => {
  const root = new THREE.Group();
  const child = new THREE.Group();
  child.userData.reveal_hide_when_applied = ["gate"];
  root.add(child);
  const initial = { color: new Uint8Array(4), depth: new Uint16Array(1) };
  await assert.rejects(
    bakeAppearanceRegionsAsync(
      root,
      [{ bounds: [0, 0, 1, 1], patches: ["gate"] }],
      1,
      initial,
      async () => {
        assert.equal(child.visible, false);
        await Promise.resolve();
        throw new Error("cancelled rendering");
      },
    ),
    /cancelled rendering/,
  );
  assert.equal(child.visible, true);
});

test("async appearance combinations render only their region and reset independent controls", async () => {
  const root = new THREE.Group();
  const children = ["left", "right"].map((patch) => {
    const child = new THREE.Group();
    child.userData.reveal_hide_when_applied = [patch];
    root.add(child);
    return child;
  });
  const initial = { color: new Uint8Array(32), depth: new Uint16Array(8).fill(7) };
  const calls: BakeBounds[] = [];
  const regions = await bakeAppearanceRegionsAsync(
    root,
    [
      { bounds: [0, 0, 1, 1], patches: ["left"] },
      { bounds: [2, 1, 2, 1], patches: ["right"] },
    ],
    4,
    initial,
    async () => {
      throw new Error("unexpected full-frame render");
    },
    async (bounds) => {
      calls.push(bounds);
      assert.deepEqual(
        children.map((child) => child.visible),
        calls.length === 1 ? [false, true] : [true, false],
      );
      return {
        color: new Uint8Array(bounds[2] * bounds[3] * 4),
        depth: new Uint16Array(bounds[2] * bounds[3]).fill(9),
      };
    },
  );
  assert.deepEqual(calls, [
    [0, 0, 1, 1],
    [2, 1, 2, 1],
  ]);
  assert.deepEqual(
    regions.map((region) => Array.from(region.states[0]!.depth)),
    [[7], [7, 7]],
  );
  assert.deepEqual(
    regions.map((region) => Array.from(region.states[1]!.depth)),
    [[9], [9, 9]],
  );
  assert.ok(children.every((child) => child.visible));
});

test("one detached gate does not freeze a valid copy's appearance or movement state", () => {
  const { document, assets } = unavailableTerrainControlCompilerFixture();
  const original = document.objects[0]!;
  document.objects.push({ ...structuredClone(original), id: "valid-body", group: "valid" });
  document.groups.push({
    ...structuredClone(document.groups[0]!),
    id: "valid",
    transform: { ...document.groups[0]!.transform, dz: -4 },
  });
  const geometry = compileMap(document, [0, 0, 2000, 2000], assets, { bestEffort: true }).descriptor
    .asset_geometry!;
  const transitions = geometry.movement_transitions!;
  assert.equal(transitions.length, 1);
  assert.equal(transitions[0]!.id, "valid/hut/barriers");
  assert.equal(transitions[0]!.has_appearance, true);
  assert.equal(transitions[0]!.motion_changes.length, 1);
  const root = new THREE.Group();
  for (const id of ["hut-a-body", "valid-body"]) {
    const wrapper = new THREE.Group();
    wrapper.userData.map_bake_object_id = id;
    const closed = new THREE.Group(),
      open = new THREE.Group();
    closed.userData.reveal_hide_when_applied = ["gate-cover"];
    open.userData.reveal_show_when_applied = ["gate-cover"];
    wrapper.add(closed, open);
    root.add(wrapper);
  }
  const display = new PatchDisplay();
  display.apply(root);
  const warnings: string[] = [];
  bindBakeAppearances(root, document, assets, transitions, (message) => warnings.push(message));
  for (const applied of [false, true, false]) {
    display.set(transitions[0]!.id, applied);
    display.apply(root);
    assert.deepEqual(
      root.children[0]!.children.map((node) => node.visible),
      [true, false],
    );
    assert.deepEqual(
      root.children[1]!.children.map((node) => node.visible),
      [!applied, applied],
    );
  }
  assert.ok(warnings.length);
});

test("best effort freezes missing appearance controls while preserving available combinations", () => {
  const { document, assets, hut } = movementTransitionCompilerFixture();
  hut.gameplay!.movementTransitions![0]!.appearances = ["roof"];
  const transitions = compileMap(document, [0, 0, 2000, 2000], assets).descriptor.asset_geometry!
    .movement_transitions!;
  const root = new THREE.Group();
  root.userData.map_bake_object_id = document.objects[0]!.id;
  const rules = [
    { reveal_material_patch: "missing", reveal_material_state: "covered" },
    {
      reveal_material_patch: "missing",
      reveal_material_state: "revealed",
      reveal_hide_when_applied: ["roof"],
    },
    { reveal_show_when_applied: ["missing"] },
    { reveal_show_when_applied: ["missing", "roof"] },
    { reveal_hide_when_applied: ["missing", "roof"] },
  ];
  for (const rule of rules) {
    const node = new THREE.Group();
    node.userData = rule;
    root.add(node);
  }
  const display = new PatchDisplay();
  display.apply(root);
  const warnings: string[] = [];
  bindBakeAppearances(root, document, assets, transitions, (message) => warnings.push(message));
  display.apply(root);
  assert.deepEqual(
    root.children.map((node) => node.visible),
    [true, false, false, false, true],
  );
  display.set("hut-a/hut/barriers", true);
  display.apply(root);
  assert.deepEqual(
    root.children.map((node) => node.visible),
    [true, false, false, true, false],
  );
  display.clear();
  display.apply(root);
  assert.deepEqual(
    root.children.map((node) => node.visible),
    [true, false, false, false, true],
  );
  assert.ok(warnings.length >= 5);
});

test("endpoint models and duplicated placements use one independent switch per asset", () => {
  const { document, assets, alias } = endpointAppearanceCompilerFixture();
  const originals = document.objects.filter((p) => p.group);
  document.groups.push({ ...structuredClone(document.groups[0]!), id: "hut-b" });
  for (const part of originals) {
    const copy = structuredClone(part);
    copy.id = copy.id.replace("hut-a", "hut-b");
    copy.group = "hut-b";
    copy.transform.dx += 500;
    document.objects.push(copy);
  }
  const transitions = compileMap(document, [0, 0, 2000, 2000], assets).descriptor.asset_geometry!
    .movement_transitions!;
  assert.equal(transitions.length, 2);
  const root = new THREE.Group();
  const available = new Set(["asset:hut:building-999", `asset:${alias}:scenery-open`]);
  for (const part of document.objects.filter((p) => p.group)) {
    const wrapper = new THREE.Group();
    wrapper.userData.map_bake_object_id = part.id;
    const model = new THREE.Group();
    applyPlacementPatches(model, document, part, available);
    // Applied model material rules retain their unremapped local name.
    if (part.node.includes(alias)) {
      const material = new THREE.Group();
      material.userData.reveal_material_patch = "state";
      material.userData.reveal_material_state = "revealed";
      model.add(material);
    }
    wrapper.add(model);
    root.add(wrapper);
  }
  bindBakeAppearances(root, document, assets, transitions);
  const display = new PatchDisplay();
  const visibility = () => root.children.map((wrapper) => wrapper.children[0]!.visible);
  display.apply(root);
  assert.deepEqual(visibility(), [true, false, true, false]);
  display.set("hut-a/hut/barriers", true);
  display.apply(root);
  assert.deepEqual(visibility(), [false, true, true, false]);
  assert.equal(root.children[1]!.children[0]!.children[0]!.visible, true);
  display.set("hut-b/hut/barriers", true);
  display.apply(root);
  assert.deepEqual(visibility(), [false, true, false, true]);
  display.clear();
  display.apply(root);
  assert.deepEqual(visibility(), [true, false, true, false]);
  document.assetSources!.find((ref) => ref.id === alias)!.descriptor_sha256 = "1".repeat(64);
  assert.throws(() => bindBakeAppearances(root, document, assets, transitions), /pinned primary/);
});
const bounds: BakeBounds = [0, 0, 100, 100];
function mesh(x: number, y: number, patch: string) {
  const geometry = new THREE.BufferGeometry().setFromPoints(
    [
      [x, y],
      [x + 10, y],
      [x, y + 10],
    ].map(([px, py]) => new THREE.Vector3(...gameToScene(camera, px!, py!, 0))),
  );
  const node = new THREE.Mesh(geometry, new THREE.MeshBasicMaterial());
  node.userData.reveal_show_when_applied = [patch];
  node.visible = false;
  return node;
}
function scene() {
  const root = new THREE.Group();
  root.add(mesh(10, 10, "a"), mesh(70, 70, "b"));
  return root;
}
const transitions = [{ id: "a" }, { id: "b" }];

test("sunlit independent switches stay bounded and respect non-casting geometry", () => {
  const root = scene();
  const lighting = { ...DEFAULT_LIGHTING, enabled: true, sunElevation: 45 };
  const plans = planAppearanceRegions(root, camera, [0, 0, 8000, 8000], transitions, lighting);
  assert.deepEqual(
    plans.map((plan) => plan.patches),
    [["a"], ["b"]],
  );
  for (const child of root.children) child.userData.noSunShadow = true;
  assert.deepEqual(
    planAppearanceRegions(root, camera, bounds, transitions, lighting),
    planAppearanceRegions(root, camera, bounds, transitions, false),
  );
  assert.deepEqual(
    planAppearanceRegions(root, camera, bounds, transitions, { ...lighting, enabled: false }),
    planAppearanceRegions(root, camera, bounds, transitions, false),
  );
});

test("appearance planning includes hidden variants and merges intersecting dependencies", () => {
  const root = scene();
  assert.deepEqual(
    planAppearanceRegions(root, camera, bounds, transitions, false).map((p) => p.patches),
    [["a"], ["b"]],
  );
  const overlap = mesh(15, 15, "b");
  root.add(overlap);
  const plans = planAppearanceRegions(root, camera, bounds, transitions, false);
  assert.equal(plans.length, 1);
  assert.deepEqual(plans[0]!.patches, ["a", "b"]);
  assert.ok(plans[0]!.bounds[0] <= 10 && plans[0]!.bounds[2] >= 70);
  root.remove(overlap);
  assert.deepEqual(planAppearanceRegions(root, camera, bounds, transitions, true), [
    { bounds, patches: ["a", "b"] },
  ]);
});

test("appearance planning respects hidden owners and validates bindings and state budgets", () => {
  const root = scene();
  const hidden = new THREE.Group();
  hidden.visible = false;
  hidden.add(mesh(0, 0, "unbound"));
  root.add(hidden);
  const frame = contentBakeBounds(root, camera, true);
  assert.ok(frame[0] >= 9 && frame[1] >= 9);
  assert.ok(frame[0] + frame[2] >= 80 && frame[1] + frame[3] >= 80);
  assert.equal(planAppearanceRegions(root, camera, bounds, transitions, false).length, 2);
  hidden.visible = true;
  assert.throws(() => planAppearanceRegions(root, camera, bounds, transitions, false), /Unbound/);
  root.remove(hidden);
  assert.throws(
    () => planAppearanceRegions(root, camera, [0, 0, 8000, 8000], transitions, true),
    /64 megapixels/,
  );
});

test("appearance combinations crop full-frame fields and restore initial state after failures", () => {
  const root = scene();
  const plans = planAppearanceRegions(root, camera, bounds, transitions, false);
  const render = () => {
    const state = Number(root.children[0]!.visible) + 2 * Number(root.children[1]!.visible);
    return {
      color: new Uint8Array(100 * 100 * 4).fill(state),
      depth: Uint16Array.from({ length: 10000 }, (_, i) => i + state),
    };
  };
  const images = bakeAppearanceRegions(root, plans, 100, render(), render);
  assert.equal(images[0]!.states[1]!.color[0], 1);
  assert.equal(images[1]!.states[1]!.color[0], 2);
  for (const image of images) {
    const [x, y, w, h] = image.bounds;
    assert.equal(image.states[0]!.depth[0], y * 100 + x);
    assert.equal(image.states[0]!.depth.length, w * h);
    assert.equal(image.states[0]!.depth[w], (y + 1) * 100 + x);
  }
  assert.ok(root.children.every((node) => !node.visible));
  assert.throws(
    () =>
      bakeAppearanceRegions(root, plans, 100, render(), () => {
        throw new Error("GPU failed");
      }),
    /GPU failed/,
  );
  assert.ok(root.children.every((node) => !node.visible));
});

test("asset-local appearance bindings follow duplicated placements and reject missing definitions", () => {
  const { document, assets, hut } = movementTransitionCompilerFixture();
  hut.gameplay!.movementTransitions![0]!.appearances = ["roof"];
  const copy = structuredClone(document.objects[0]!);
  copy.id = "hut-b-body";
  copy.group = "hut-b";
  copy.transform.dx += 500;
  document.objects.push(copy);
  document.groups.push({ ...structuredClone(document.groups[0]!), id: "hut-b" });
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets);
  const transitions = compiled.descriptor.asset_geometry!.movement_transitions!;
  const root = new THREE.Group();
  for (const id of ["hut-a-body", "hut-b-body"]) {
    const wrapper = new THREE.Group();
    wrapper.userData.map_bake_object_id = id;
    wrapper.add(mesh(0, 0, "roof"));
    root.add(wrapper);
  }
  const unbound = root.clone(true);
  bindBakeAppearances(root, document, assets, transitions);
  assert.deepEqual(
    root.children.map((wrapper) => wrapper.children[0]!.userData.reveal_show_when_applied),
    [["hut-a/hut/barriers"], ["hut-b/hut/barriers"]],
  );
  document.groups[0]!.patches = { hut: { roof: "preview-a" } };
  document.groups[1]!.patches = { hut: { roof: "preview-b" } };
  const preview = unbound.clone(true);
  preview.children[0]!.children[0]!.userData.reveal_show_when_applied = ["preview-a"];
  preview.children[1]!.children[0]!.userData.reveal_show_when_applied = ["preview-b"];
  const remapped = compileMap(document, [0, 0, 2000, 2000], assets);
  bindBakeAppearances(
    preview,
    document,
    assets,
    remapped.descriptor.asset_geometry!.movement_transitions!,
  );
  assert.deepEqual(
    preview.children.map((wrapper) => wrapper.children[0]!.userData.reveal_show_when_applied),
    [["hut-a/hut/barriers"], ["hut-b/hut/barriers"]],
  );
  document.groups[1]!.patches = { hut: { roof: "preview-a" } };
  assert.equal(
    compileMap(document, [0, 0, 2000, 2000], assets).descriptor.asset_geometry!
      .movement_transitions!.length,
    2,
  );
  delete hut.gameplay!.movementTransitions![0]!.appearances;
  assert.throws(
    () => bindBakeAppearances(unbound, document, assets, transitions),
    /Missing asset gameplay binding/,
  );
  hut.gameplay!.movementTransitions![0]!.appearances = ["roof", "roof"];
  assert.throws(
    () => validateAssetGameplay(hut.gameplay, hut),
    /multiply controlled transition appearance/,
  );
});

test("asset contact joins bind both appearances and detach when a member moves", () => {
  const { document, assets, wing, wingPart } = joinedTransitionCompilerFixture();
  const compile = () =>
    compileMap(document, [0, 0, 2000, 2000], assets).descriptor.asset_geometry!
      .movement_transitions!;
  const joined = compile();
  assert.equal(joined.length, 1);
  assert.deepEqual(joined[0]!.aliases, ["wing/wing/barriers"]);
  assert.equal(joined[0]!.initial_sight!.length, 2);
  assert.equal(joined[0]!.applied_sight!.length, 2);
  const bind = (transitions: ReturnType<typeof compile>) => {
    const root = new THREE.Group();
    for (const id of ["hut-a-body", "wing-body"]) {
      const wrapper = new THREE.Group();
      wrapper.userData.map_bake_object_id = id;
      wrapper.add(mesh(0, 0, "preview-roof"));
      root.add(wrapper);
    }
    bindBakeAppearances(root, document, assets, transitions);
    return root.children.map((wrapper) => wrapper.children[0]!.userData.reveal_show_when_applied);
  };
  assert.deepEqual(bind(joined), [[joined[0]!.id], [joined[0]!.id]]);
  wingPart.transform.dx += 1;
  const separate = compile();
  assert.equal(separate.length, 2);
  assert.deepEqual(bind(separate), [["hut-a/hut/barriers"], ["wing/wing/barriers"]]);
  wingPart.transform.dx -= 1;
  const copies = document.objects
    .filter((p) => p.group)
    .map((p) => {
      const copy = structuredClone(p);
      copy.id += "-copy";
      copy.group += "-copy";
      copy.transform.dy += 500;
      return copy;
    });
  document.objects.push(...copies);
  document.groups.push(
    ...document.groups.map((g) => ({ ...structuredClone(g), id: `${g.id}-copy` })),
  );
  const duplicated = compile();
  assert.equal(duplicated.length, 2);
  assert.deepEqual(
    new Set(duplicated.flatMap((t) => t.aliases!)),
    new Set(["wing/wing/barriers", "wing-copy/wing/barriers"]),
  );
  wing.gameplay!.movementTransitions![0]!.definitive = true;
  assert.throws(compile, /identical world triggers/);
  wing.gameplay!.movementTransitions![0]!.join!.point[0] = NaN;
  assert.throws(compile, /invalid transition join anchor/);
});

test("different assets cannot share a preview alias without explicit join definitions", () => {
  const { document, assets, hut, wing } = joinedTransitionCompilerFixture();
  delete hut.gameplay!.movementTransitions![0]!.join;
  delete wing.gameplay!.movementTransitions![0]!.join;
  assert.throws(
    () => compileMap(document, [0, 0, 2000, 2000], assets),
    /joined gameplay transition/,
  );
});

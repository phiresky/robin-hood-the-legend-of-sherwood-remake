import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import {
  nativeEffectCamera,
  renderNativeRemainderLayer,
  renderPhysicalEffect,
  validateNativeEffectMaterial,
} from "./native-physical-effect-preview.ts";

test("native effect camera preserves source pixel registration at multiple physical heights", () => {
  const box = new THREE.Box3(new THREE.Vector3(820, -3, 1500), new THREE.Vector3(890, 42, 1600));
  const v = nativeEffectCamera(box, 35);
  for (const p of [box.min, box.max, box.getCenter(new THREE.Vector3())]) {
    const projected = p.clone().project(v.camera);
    assert.ok(Math.abs(v.x + ((projected.x + 1) * v.width) / 2 - p.x) < 1e-8);
    const expected = p.z * Math.sin((35 * Math.PI) / 180) - p.y * Math.cos((35 * Math.PI) / 180);
    assert.ok(Math.abs(v.y + ((1 - projected.y) * v.height) / 2 - expected) < 1e-8);
    assert.ok(projected.z > -1 && projected.z < 1);
  }
});

function rendererFixture() {
  const events: string[] = [];
  const original = { name: "original" };
  let target: unknown = original,
    clearAlpha = 0.8;
  const viewport = new THREE.Vector4(3, 4, 5, 6),
    scissor = new THREE.Vector4(7, 8, 9, 10);
  const color = new THREE.Color(0.1, 0.2, 0.3);
  let scissorTest = true;
  const renderer = {
    xr: { enabled: false },
    shadowMap: { enabled: true, needsUpdate: true },
    capabilities: { maxTextureSize: 4096 },
    autoClear: true,
    toneMapping: THREE.ACESFilmicToneMapping,
    getRenderTarget: () => target,
    getActiveCubeFace: () => 2,
    getActiveMipmapLevel: () => 1,
    getViewport: (v: THREE.Vector4) => v.copy(viewport),
    getScissor: (v: THREE.Vector4) => v.copy(scissor),
    getScissorTest: () => scissorTest,
    getClearColor: (c: THREE.Color) => c.copy(color),
    getClearAlpha: () => clearAlpha,
    setRenderTarget: (value: unknown) => {
      target = value;
    },
    setViewport: (...v: [THREE.Vector4] | [number, number, number, number]) => {
      if (v.length === 1) viewport.copy(v[0]);
      else viewport.set(...v);
    },
    setScissor: (v: THREE.Vector4) => scissor.copy(v),
    setScissorTest: (v: boolean) => {
      scissorTest = v;
    },
    setClearColor: (v: THREE.Color | number, a: number) => {
      color.set(v);
      clearAlpha = a;
    },
    clear: () => events.push("clear"),
    render: () => events.push("render"),
    readRenderTargetPixels: (
      _t: unknown,
      _x: number,
      _y: number,
      _w: number,
      _h: number,
      bytes: Uint8Array,
    ) => {
      bytes.set([50, 25, 0, 255, 20, 30, 40, 255]);
    },
  };
  const restored = () => {
    assert.equal(target, original);
    assert.equal(scissorTest, true);
    assert.equal(clearAlpha, 0.8);
    assert.deepEqual(viewport.toArray(), [3, 4, 5, 6]);
    assert.deepEqual(scissor.toArray(), [7, 8, 9, 10]);
    assert.deepEqual(color.toArray(), [0.1, 0.2, 0.3]);
    assert.equal(renderer.autoClear, true);
    assert.equal(renderer.toneMapping, THREE.ACESFilmicToneMapping);
    assert.equal(renderer.shadowMap.enabled, true);
    assert.equal(renderer.shadowMap.needsUpdate, true);
  };
  return { renderer, events, restored };
}

test("isolated physical pixels flip readback rows and restore all borrowed renderer state", () => {
  const f = rendererFixture();
  const pixels = renderPhysicalEffect(
    f.renderer as unknown as THREE.WebGLRenderer,
    new THREE.Scene(),
    new THREE.Camera(),
    1,
    2,
  );
  assert.deepEqual(Array.from(pixels.data), [20, 30, 40, 255, 50, 25, 0, 255]);
  assert.deepEqual(f.events, ["clear", "render"]);
  f.restored();
});

test("failed isolated render restores the existing target, viewport and clear state", () => {
  const f = rendererFixture();
  f.renderer.render = () => {
    throw Error("shader failed");
  };
  assert.throws(
    () =>
      renderPhysicalEffect(
        f.renderer as unknown as THREE.WebGLRenderer,
        new THREE.Scene(),
        new THREE.Camera(),
        1,
        2,
      ),
    /shader failed/,
  );
  f.restored();
});

test("fractional alpha fails without silently changing color-space blending", () => {
  const f = rendererFixture();
  f.renderer.readRenderTargetPixels = (_t, _x, _y, _w, _h, bytes) => {
    bytes.fill(128);
  };
  assert.throws(
    () =>
      renderPhysicalEffect(
        f.renderer as unknown as THREE.WebGLRenderer,
        new THREE.Scene(),
        new THREE.Camera(),
        1,
        2,
      ),
    /binary cutout alpha/,
  );
  f.restored();
});

for (const kind of ["map-animation", "mission-target"] as const)
test(`${kind} physical substitution rejects stale identity and unsampled phases`, async () => {
  const { NativePhysicalEffectPreview } = await import("./native-physical-effect-preview.ts");
  const hash = "a".repeat(64);
  const element = {
    id: "b05",
    source: { kind, index: 12, sha256: hash },
    active: true,
    frames: [
      { path: "a.png", sha256: hash, width: 1, height: 1, offset: [0, 0], delay: 1 },
      { path: "b.png", sha256: hash, width: 1, height: 1, offset: [0, 0], delay: 1 },
    ],
    loop: true,
    display_position: [0, 0],
    sort_position: [0, 0],
    display_order: 0,
    creation_order: 0,
    polyline: [],
  };
  const native = {
    mission: "S03",
    camera_elevation_deg: 35,
    elements: [element],
  } as unknown as import("../../shared/src/native-state-presentation.ts").NativeStatePresentationContract;
  const binding = {
    source_kind: kind === "map-animation" ? kind : undefined,
    target_index: 12,
    target_sha256: hash,
  } as import("../../shared/src/mission-state.ts").MissionStateContract["targets"][number];
  const object = new THREE.Group();
  assert.throws(
    () =>
      new NativePhysicalEffectPreview(native, [
        { object, tick: () => 0, binding: { ...binding, target_sha256: "b".repeat(64) } },
      ]),
    /source identity/,
  );
  const preview = new NativePhysicalEffectPreview(native, [{ object, tick: () => 0, binding }]);
  const pixels = { width: 1, height: 1, data: new Uint8Array(4) };
  const snapshot = {
    mission: "S03",
    revision: 1,
    origin: [0, 0],
    background: pixels,
    draws: [{ element, frame: 1, sourceTick: 0, sourceActive: true, pixels, x: 0, y: 0 }],
  } as import("./native-state-presentation.ts").NativeLoopDrawSnapshot;
  assert.throws(
    () => preview.sample({} as THREE.WebGLRenderer, snapshot),
    /pose and native frame differ/,
  );
  snapshot.draws[0]!.frame = 0;
  snapshot.draws[0]!.sourceTick = 1;
  assert.throws(
    () => preview.sample({} as THREE.WebGLRenderer, snapshot),
    /pose and native frame differ/,
  );
  snapshot.draws[0]!.sourceTick = 0;
  snapshot.draws[0]!.element = {
    ...snapshot.draws[0]!.element,
    source: { ...snapshot.draws[0]!.element.source, sha256: "b".repeat(64) },
  };
  assert.throws(() => preview.sample({} as THREE.WebGLRenderer, snapshot), /snapshot identity/);
  assert.throws(
    () => new NativePhysicalEffectPreview(native, [{ object, tick: () => 0,
      binding: { ...binding, source_kind: kind === "map-animation" ? "mission-target" : "map-animation" } }]),
    /source identity/,
  );
  snapshot.draws[0]!.element = element as typeof snapshot.draws[number]["element"];
  object.visible = false;
  const unbound = { ...snapshot.draws[0]!, element: { ...snapshot.draws[0]!.element, id: "unbound" } };
  snapshot.draws.push(unbound);
  const sampled = preview.sample({} as THREE.WebGLRenderer, snapshot);
  assert.equal(sampled.draws[1], unbound);
  assert.equal(sampled.draws[0]!.element, element);
  assert.equal(sampled.draws[0]!.sourceTick, 0);
  assert.deepEqual(Array.from(sampled.draws[0]!.pixels.data), [0, 0, 0, 0]);

});

test("refined receiver isolates terrain and static solids, excluding effects, actors and helpers", async () => {
  const { withNativeReceiverVisibility } = await import("./native-physical-effect-preview.ts");
  const scene = new THREE.Scene(),
    terrain = new THREE.Group(),
    staticObjects = new THREE.Group(),
    effect = new THREE.Group();
  const mesh = () => new THREE.Mesh(new THREE.BufferGeometry(), new THREE.MeshBasicMaterial());
  const ground = mesh(),
    wall = mesh(),
    oldEffect = mesh(),
    butterfly = mesh(),
    actor = new THREE.Sprite(),
    helper = new THREE.Line();
  const alreadyHidden = mesh();
  alreadyHidden.visible = false;
  terrain.add(ground);
  staticObjects.add(wall, oldEffect, alreadyHidden, helper);
  effect.add(butterfly);
  scene.add(terrain, staticObjects, effect, actor);
  assert.throws(
    () =>
      withNativeReceiverVisibility(scene, [terrain, staticObjects], [oldEffect], () => {
        assert.equal(ground.visible, true);
        assert.equal(wall.visible, true);
        for (const node of [oldEffect, butterfly, actor, helper, alreadyHidden])
          assert.equal(node.visible, false);
        throw Error("render fails");
      }),
    /render fails/,
  );
  for (const node of [ground, wall, oldEffect, butterfly, actor, helper])
    assert.equal(node.visible, true);
  assert.equal(alreadyHidden.visible, false);
});

test("full refined receiver camera retains source origin and image extent independently of geometry bounds", () => {
  const v = nativeEffectCamera(
    new THREE.Box3(new THREE.Vector3(-50, -5, -80), new THREE.Vector3(2200, 300, 2700)),
    35,
    { x: 64, y: 128, width: 1792, height: 1152 },
  );
  const origin = new THREE.Vector3(64, 0, 128 / Math.sin((35 * Math.PI) / 180)).project(v.camera);
  assert.ok(Math.abs(origin.x + 1) < 1e-8);
  assert.ok(Math.abs(origin.y - 1) < 1e-8);
  assert.equal(v.width, 1792);
  assert.equal(v.height, 1152);
});

function sourceEmissiveMaterial() {
  const texture = new THREE.Texture();
  const material = new THREE.MeshStandardMaterial({
    map: texture,
    emissiveMap: texture,
    emissive: 0xffffff,
    metalness: 0,
    alphaTest: 0.5,
    vertexColors: true,
  });
  Object.assign(material.userData, {
    foliage_physical_opacity: true,
    opacity_semantics: "physical-coverage",
    source_ownership_semantics: "separate-mask",
    source_ownership_channel: "vertex-color-r",
    foliage_card_sides: "paired-one-sided",
  });
  return material;
}

test("source-emissive foliage keeps its exact material on the sampled root clone", async () => {
  const { TextureDisplay } = await import("./texture-display.ts");
  const material = sourceEmissiveMaterial();
  new TextureDisplay().material(material);
  const root = new THREE.Mesh(new THREE.BufferGeometry(), material);
  const clone = root.clone(true);
  assert.equal(clone.material, material);
  assert.doesNotThrow(() => validateNativeEffectMaterial(clone.material));
});

test("emissive qualification rejects unsupported lighting, UV, alpha and shader state", () => {
  const alterations: Array<(m: THREE.MeshStandardMaterial) => void> = [
    (m) => {
      m.emissiveIntensity = 2;
    },
    (m) => {
      m.emissive.setRGB(1, 0.9, 1);
    },
    (m) => {
      m.emissiveMap = new THREE.Texture();
    },
    (m) => {
      m.emissiveMap = m.map!.clone();
      m.emissiveMap.offset.x = 0.1;
    },
    (m) => {
      m.emissiveMap = m.map!.clone();
      m.emissiveMap.channel = 1;
    },
    (m) => {
      m.emissiveMap = m.map!.clone();
      m.emissiveMap.colorSpace = THREE.SRGBColorSpace;
    },
    (m) => {
      m.envMap = new THREE.Texture();
    },
    (m) => {
      m.normalMap = new THREE.Texture();
    },
    (m) => {
      m.transparent = true;
    },
    (m) => {
      m.alphaTest = 0.1;
    },
    (m) => {
      m.alphaToCoverage = true;
    },
    (m) => {
      m.alphaHash = true;
    },
    (m) => {
      m.userData = {};
    },
    (m) => {
      m.onBeforeCompile = () => {};
    },
    (m) => {
      m.customProgramCacheKey = () => "untrusted";
    },
  ];
  for (const alter of alterations) {
    const material = sourceEmissiveMaterial();
    alter(material);
    assert.throws(() => validateNativeEffectMaterial(material));
  }
  assert.throws(() => validateNativeEffectMaterial(new THREE.MeshStandardMaterial()));
  assert.throws(() => validateNativeEffectMaterial(new THREE.MeshPhysicalMaterial()));
  assert.doesNotThrow(() => validateNativeEffectMaterial(new THREE.MeshBasicMaterial()));
});

test("ownership display hook cannot launder prior or subsequently changed shader callbacks", async () => {
  const { TextureDisplay } = await import("./texture-display.ts");
  const before = sourceEmissiveMaterial();
  before.onBeforeCompile = () => {};
  new TextureDisplay().material(before);
  assert.throws(() => validateNativeEffectMaterial(before));
  const after = sourceEmissiveMaterial();
  new TextureDisplay().material(after);
  after.onBeforeCompile = () => {};
  assert.throws(() => validateNativeEffectMaterial(after));
});


test("opaque source-only emission accepts the sign material without allowing lighting or coverage modifiers", () => {
  const make = () => new THREE.MeshStandardMaterial({
    color: 0,
    emissive: 0xffffff,
    emissiveMap: new THREE.Texture(),
    metalness: 1,
    roughness: 1,
    side: THREE.DoubleSide,
  });
  assert.doesNotThrow(() => validateNativeEffectMaterial(make()));
  for (const alter of [
    (m: THREE.MeshStandardMaterial) => { m.color.set(0xffffff); },
    (m: THREE.MeshStandardMaterial) => { m.emissiveIntensity = 0.8; },
    (m: THREE.MeshStandardMaterial) => { m.map = new THREE.Texture(); },
    (m: THREE.MeshStandardMaterial) => { m.envMap = new THREE.Texture(); },
    (m: THREE.MeshStandardMaterial) => { m.alphaTest = 0.5; },
    (m: THREE.MeshStandardMaterial) => { m.vertexColors = true; },
    (m: THREE.MeshStandardMaterial) => { m.onBeforeCompile = () => {}; },
    (m: THREE.MeshStandardMaterial) => { m.userData.source_ownership_semantics = "separate-mask"; },
    (m: THREE.MeshStandardMaterial) => { m.emissiveMap!.channel = 1; },
  ]) {
    const material = make();
    alter(material);
    assert.throws(() => validateNativeEffectMaterial(material), /unsupported source-emissive/);
  }
});

test("remainder layer renders a hidden applied root in the complete source frame without changing it", () => {
  const f = rendererFixture();
  let rendered: THREE.Scene | undefined, camera: THREE.OrthographicCamera | undefined;
  f.renderer.render = ((scene: THREE.Scene, view: THREE.OrthographicCamera) => {
    rendered = scene;
    camera = view;
    f.events.push("render");
  }) as unknown as () => number;
  const parent = new THREE.Group();
  parent.position.set(4, 0, 0);
  const root = new THREE.Group();
  root.visible = false;
  root.add(new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial()));
  parent.add(root);
  const pixels = renderNativeRemainderLayer(
    f.renderer as unknown as THREE.WebGLRenderer,
    root,
    { origin: [64, 128], width: 1, height: 2 },
    35,
  );
  assert.deepEqual(Array.from(pixels.data), [20, 30, 40, 255, 50, 25, 0, 255]);
  assert.equal(root.visible, false);
  assert.equal(root.parent, parent);
  const clone = rendered!.children[0]!;
  assert.equal(clone.visible, true);
  assert.equal(new THREE.Vector3().setFromMatrixPosition(clone.matrixWorld).x, 4);
  const corner = new THREE.Vector3(64, 0, 128 / Math.sin((35 * Math.PI) / 180)).project(camera!);
  assert.ok(Math.abs(corner.x + 1) < 1e-8 && Math.abs(corner.y - 1) < 1e-8);
  f.restored();
  const lit = new THREE.Group();
  lit.add(new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial()));
  assert.throws(
    () =>
      renderNativeRemainderLayer(
        f.renderer as unknown as THREE.WebGLRenderer,
        lit,
        { origin: [0, 0], width: 1, height: 2 },
        35,
      ),
    /unsupported source-emissive material/,
  );
  const lamp = root.clone();
  lamp.add(new THREE.PointLight());
  assert.throws(
    () =>
      renderNativeRemainderLayer(
        f.renderer as unknown as THREE.WebGLRenderer,
        lamp,
        { origin: [0, 0], width: 1, height: 2 },
        35,
      ),
    /light-free/,
  );
  assert.throws(
    () =>
      renderNativeRemainderLayer(
        f.renderer as unknown as THREE.WebGLRenderer,
        new THREE.Group(),
        { origin: [0, 0], width: 1, height: 2 },
        35,
      ),
    /no visible model/,
  );
});

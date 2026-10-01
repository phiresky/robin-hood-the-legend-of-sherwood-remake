import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import {
  createTerrainGrid,
  gameToScene,
  parseLevel3D,
  type GameTransform,
  type Level3D,
} from "@rle/shared";
import type { Selection } from "./document-commands.ts";
import { EditorViewport } from "./editor-viewport.ts";

function fixture(showElevation = () => false) {
  let document: Level3D | null = null;
  let selection: Selection = null;
  const viewport = new EditorViewport({
    document: () => document,
    selection: () => selection,
    level: () => null,
    showObstacles: () => false,
    showElevation,
    onSelection: (next) => {
      selection = next;
      viewport.syncSelection(next);
    },
    commitTransform: () => {
      throw new Error("Unexpected gesture");
    },
  });
  return {
    viewport,
    selection: () => selection,
    publish: (next: Level3D, rebuildFraming = true, deferredBinding = false) => {
      if (!deferredBinding) document = next;
      viewport.syncViews(next, rebuildFraming);
      document = next;
    },
  };
}

test("map replacement owns retirement and releases detached ground exactly once", () => {
  const { viewport } = fixture();
  const asset = new THREE.Group();
  const ground = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  asset.add(ground);
  let disposals = 0;
  ground.geometry.addEventListener("dispose", () => disposals++);
  viewport.replaceMap(asset, ground, new Map());
  assert.notEqual(ground.parent, asset);
  assert.ok(ground.parent);
  viewport.replaceMap(new THREE.Group(), null, new Map());
  assert.equal(ground.parent, null);
  assert.equal(viewport.groundNode, null);
  assert.equal(disposals, 1);
  viewport.dispose();
  viewport.dispose();
  assert.equal(disposals, 1);
  assert.throws(() => viewport.replaceMap(new THREE.Group(), null, new Map()), /Disposed/);
});

function documentFixture() {
  return parseLevel3D({
    version: 1,
    map: "York",
    sceneAssets: [],
    size: [100, 200],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    groups: [{ id: "house", transform: { dx: 10, dy: 20, dz: 0, rot_deg: 15 } }],
    objects: [
      {
        id: "part",
        group: "house",
        node: "building-000",
        kind: "building",
        source: { map: "York", obstacle: 0 },
        transform: { dx: 1, dy: 2, dz: 0, rot_deg: 0 },
        obstacle: {
          points: [
            { x: 1, y: 2, z_bottom: 0, z_top: 4 },
            { x: 8, y: 2, z_bottom: 0, z_top: 4 },
            { x: 3, y: 9, z_bottom: 0, z_top: 4 },
          ],
          opaque: true,
          solid: true,
          mouse: false,
          show_shadow_polygon: false,
          default_material: 0,
          material_indices: [],
          projection_area: {},
        },
      },
    ],
  });
}

test("repeated normal clicks retain group selection; only Alt-click selects a part", () => {
  const { viewport, publish, selection } = fixture();
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(10, 10, 10), new THREE.MeshBasicMaterial());
  const source = new THREE.Group();
  source.add(mesh);
  viewport.replaceMap(source, null, new Map([["building-000", mesh]]));
  publish(documentFixture());
  const root = (viewport as unknown as { objectsRoot: THREE.Group }).objectsRoot;
  root.updateWorldMatrix(true, true);
  const center = new THREE.Box3().setFromObject(root).getCenter(new THREE.Vector3());
  const camera = new THREE.OrthographicCamera(-100, 100, 100, -100, -10000, 10000);
  camera.position.copy(center).add(new THREE.Vector3(0, 100, 100));
  camera.lookAt(center);
  Object.assign(viewport, {
    camera,
    orbit: { target: center },
    renderer: {
      domElement: { getBoundingClientRect: () => ({ left: 0, top: 0, width: 200, height: 200 }) },
    },
  });
  const pick = (alt: boolean) =>
    (viewport as unknown as { pick: (event: unknown, alt: boolean) => void }).pick(
      { clientX: 100, clientY: 100 },
      alt,
    );
  try {
    pick(false);
    assert.deepEqual(selection(), { kind: "group", id: "house" });
    pick(false);
    assert.deepEqual(selection(), { kind: "group", id: "house" });
    pick(true);
    assert.deepEqual(selection(), { kind: "part", id: "part" });
    pick(false);
    assert.deepEqual(selection(), { kind: "group", id: "house" });
  } finally {
    Object.assign(viewport, { renderer: null });
    viewport.dispose();
  }
});

test("scene revisions retire selection without disposing shared reconstruction resources", () => {
  const { viewport, publish, selection } = fixture();
  const document = documentFixture();
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  const asset = new THREE.Group();
  asset.add(mesh);
  let geometryDisposals = 0;
  let sourceMaterialDisposals = 0;
  let tintDisposals = 0;
  mesh.geometry.addEventListener("dispose", () => geometryDisposals++);
  mesh.material.addEventListener("dispose", () => sourceMaterialDisposals++);
  const cloneMaterial = mesh.material.clone.bind(mesh.material);
  mesh.material.clone = () => {
    const material = cloneMaterial();
    material.addEventListener("dispose", () => tintDisposals++);
    return material;
  };
  viewport.replaceMap(asset, null, new Map([["building-000", mesh]]));
  publish(document);
  viewport.select({ kind: "part", id: "part" });
  assert.equal(tintDisposals, 0);
  publish({ ...document, objects: [], groups: [] });
  assert.equal(selection(), null);
  assert.equal(tintDisposals, 1);
  assert.equal(geometryDisposals, 0);
  assert.equal(sourceMaterialDisposals, 0);
  publish(document); // Undo reconstructs a view from the still-owned source.
  viewport.select({ kind: "group", id: "house" });
  viewport.replaceMap(new THREE.Group(), null, new Map());
  assert.equal(selection(), null);
  assert.equal(tintDisposals, 2);
  assert.equal(geometryDisposals, 1);
  assert.equal(sourceMaterialDisposals, 1);
  viewport.dispose();
  assert.equal(geometryDisposals, 1);
});

test("missing reconstruction nodes fail at the scene projection boundary", () => {
  const { viewport, publish } = fixture();
  viewport.replaceMap(new THREE.Group(), null, new Map());
  assert.throws(() => publish(documentFixture()), /Missing source node building-000/);
  viewport.dispose();
});

test("gizmo binding commits through the document owner and picking resolves the live revision", () => {
  let document = documentFixture();
  let selection: Selection = null;
  const commits: GameTransform[] = [];
  const viewport = new EditorViewport({
    document: () => document,
    selection: () => selection,
    level: () => null,
    showObstacles: () => false,
    showElevation: () => false,
    onSelection: (next) => {
      selection = next;
      viewport.syncSelection(next);
    },
    commitTransform: (transform) => {
      commits.push(transform);
      document = {
        ...document,
        objects: document.objects.map((o) => ({ ...o, transform })),
      };
      viewport.syncViews(document);
    },
  });
  // The production control invokes these exact owner callbacks. A fake control
  // observes binding without requiring a WebGL context in this geometry test.
  let attached: THREE.Object3D | null = null;
  Object.assign(viewport, {
    gizmo: {
      attach: (object: THREE.Object3D) => {
        attached = object;
      },
      detach: () => {
        attached = null;
      },
    },
  });
  const callbacks = viewport as unknown as {
    commitGizmo(): void;
    partViews: Map<string, { wrapper: THREE.Object3D }>;
    partOfHit(hit: { object: THREE.Object3D }): Level3D["objects"][number] | null;
  };
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  const asset = new THREE.Group();
  asset.add(mesh);
  viewport.replaceMap(asset, null, new Map([["building-000", mesh]]));
  viewport.syncViews(document);
  viewport.select({ kind: "part", id: "part" });
  assert.ok(attached);
  const wrapper = callbacks.partViews.get("part")!.wrapper;
  assert.notEqual(attached, wrapper, "Gizmo uses an independent rotated frame");
  const picked = wrapper.children[0].children[0];
  assert.equal(callbacks.partOfHit({ object: picked }), document.objects[0]);
  const delta = gameToScene(document.camera, 7, -3, 2);
  wrapper.position.add(new THREE.Vector3(...delta));
  callbacks.commitGizmo();
  assert.deepEqual(commits, [{ dx: 8, dy: -1, dz: 2, rot_deg: 0 }]);
  assert.equal(callbacks.partOfHit({ object: picked }), document.objects[0]);
  callbacks.commitGizmo();
  assert.equal(commits.length, 1, "unchanged gizmo must not append another history revision");
  viewport.dispose();
  assert.equal(attached, null);
});

test("perspective preserves target-plane framing, scales by distance, and returns to orthographic", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-200, 200, 100, -100, -100000, 100000);
  camera.position.set(0, 0, 1000);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  Object.assign(viewport, {
    camera,
    frustum: 100,
    container: { clientWidth: 800, clientHeight: 400 },
    orbit: { target: new THREE.Vector3() },
  });
  const access = viewport as unknown as { activeCamera(): THREE.Camera };
  const before = new THREE.Vector3(40, 20, 0).project(camera);
  viewport.setPerspective(45);
  const perspective = access.activeCamera();
  const after = new THREE.Vector3(40, 20, 0).project(perspective);
  assert.ok(Math.abs(before.x - after.x) < 1e-8);
  assert.ok(Math.abs(before.y - after.y) < 1e-8);
  const near = new THREE.Vector3(40, 0, 50).project(perspective);
  const far = new THREE.Vector3(40, 0, -50).project(perspective);
  assert.ok(near.x > far.x);
  const ray = new THREE.Raycaster();
  ray.setFromCamera(new THREE.Vector2(after.x, after.y), perspective);
  const hit = ray.ray.intersectPlane(
    new THREE.Plane(new THREE.Vector3(0, 0, 1), 0),
    new THREE.Vector3(),
  );
  assert.ok(hit!.distanceTo(new THREE.Vector3(40, 20, 0)) < 1e-6);
  viewport.setPerspective(0);
  assert.equal(access.activeCamera(), camera);
  assert.equal(camera.zoom, 1);
  viewport.dispose();
});

test("perspective keeps a deep map's apparent size across lens angles and zoom levels", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000);
  camera.position.set(0, 0, 10000);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  const bounds = new THREE.Box3(
    new THREE.Vector3(-800, -900, -1500),
    new THREE.Vector3(800, 900, 1500),
  );
  Object.assign(viewport, {
    camera,
    frustum: 1000,
    container: { clientWidth: 800, clientHeight: 400 },
    orbit: { target: new THREE.Vector3() },
    framingBounds: bounds,
  });
  const access = viewport as unknown as { activeCamera(): THREE.PerspectiveCamera };
  for (const zoom of [0.5, 1, 4]) {
    camera.zoom = zoom;
    camera.updateProjectionMatrix();
    const size = (lens: THREE.Camera) =>
      Math.sqrt(
        [-1500, 1500].reduce((sum, z) => {
          const p = new THREE.Vector3(800, 900, z).project(lens);
          return sum + p.x ** 2 + p.y ** 2;
        }, 0) / 2,
      );
    const expected = size(camera);
    for (const fov of [1, 15, 30, 45, 65]) {
      viewport.setPerspective(fov);
      const actual = size(access.activeCamera());
      assert.ok(Math.abs(actual - expected) < 1e-8, `framing at ${fov} degrees, zoom ${zoom}`);
    }
  }
  viewport.dispose();
});

test("perspective pan translates camera and target along the floor without refitting the lens", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000);
  camera.position.set(1800, 3000, 6000);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  const orbit = new OrbitControls(camera);
  Object.assign(orbit, { domElement: { clientWidth: 800, clientHeight: 400 } });
  Object.assign(viewport, {
    camera,
    orbit,
    frustum: 1000,
    container: { clientWidth: 800, clientHeight: 400 },
    framingBounds: new THREE.Box3(
      new THREE.Vector3(-800, -900, -1500),
      new THREE.Vector3(800, 900, 1500),
    ),
  });
  const access = viewport as unknown as { activeCamera(): THREE.Camera };
  for (const fov of [1, 15, 45, 65]) {
    viewport.setPerspective(fov);
    const lens = access.activeCamera();
    const initial = lens.position.clone();
    const target = orbit.target.clone();
    const controlHeight = camera.position.y;
    for (const [dx, dy] of [
      [50, 80],
      [-120, 40],
      [20, -170],
    ]) {
      orbit.pan(dx, dy);
      const translated = access.activeCamera().position.clone();
      assert.ok(Math.abs(translated.y - initial.y) < 1e-8);
      assert.ok(Math.abs(camera.position.y - controlHeight) < 1e-8);
      assert.ok(Math.abs(orbit.target.y - target.y) < 1e-8);
      assert.ok(translated.sub(initial).distanceTo(orbit.target.clone().sub(target)) < 1e-8);
    }
    assert.ok(orbit.target.distanceTo(target) > 1);
  }
  viewport.setPerspective(0);
  assert.equal(orbit.screenSpacePanning, true);
  Object.assign(orbit, { domElement: null });
  viewport.dispose();
});

test("free and 16-angle orbit preserve the cursor pivot and camera distance", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000);
  camera.position.set(1800, 3000, 6000);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  const orbit = new OrbitControls(camera);
  Object.assign(viewport, {
    camera,
    orbit,
    frustum: 1000,
    container: { clientWidth: 800, clientHeight: 400 },
    framingBounds: new THREE.Box3(
      new THREE.Vector3(-1800, -200, -500),
      new THREE.Vector3(1800, 900, 500),
    ),
  });
  const access = viewport as unknown as {
    activeCamera(): THREE.Camera;
    setupCursorOrbit(el: HTMLCanvasElement): void;
    objectsRoot: THREE.Group;
  };
  const floor = new THREE.Mesh(
    new THREE.PlaneGeometry(20000, 20000),
    new THREE.MeshBasicMaterial({ side: THREE.DoubleSide }),
  );
  access.objectsRoot.add(floor);
  floor.updateWorldMatrix(true, false);
  const element = Object.assign(new EventTarget(), {
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 400 }),
    setPointerCapture() {},
    releasePointerCapture() {},
    hasPointerCapture: () => false,
  });
  access.setupCursorOrbit(element as unknown as HTMLCanvasElement);
  const pointer = (type: string, x: number, y: number) =>
    element.dispatchEvent(
      Object.assign(new Event(type), { button: 2, pointerId: 1, clientX: x, clientY: y }),
    );
  for (const snap of [true, false])
    for (const fov of [0, 1, 15, 45, 65]) {
      viewport.setPerspective(fov);
      viewport.setRotationSnap(snap);
      const initialRotation = access.activeCamera().quaternion.clone();
      if (snap) {
        const back = new THREE.Vector3(0, 0, 1).applyQuaternion(initialRotation);
        const sector = Math.atan2(back.x, back.z) / (Math.PI / 8);
        assert.ok(Math.abs(sector - Math.round(sector)) < 1e-8, "enabling snaps immediately");
      }
      const ray = new THREE.Raycaster();
      ray.setFromCamera(new THREE.Vector2(0.25, -0.1), access.activeCamera());
      const pivot = ray.ray.intersectPlane(
        new THREE.Plane(new THREE.Vector3(0, 1, 0), 0),
        new THREE.Vector3(),
      )!;
      assert.ok(pivot);
      const initialDistance = access.activeCamera().position.distanceTo(pivot);
      pointer("pointerdown", 500, 220);
      for (const [x, y] of [
        [510, 220],
        [520, 220],
        [526, 220],
        [600, 220],
        [680, 160],
        [440, 260],
        [500, 220],
      ]) {
        pointer("pointermove", x, y);
        const lens = access.activeCamera();
        if (snap) {
          const back = new THREE.Vector3(0, 0, 1).applyQuaternion(lens.quaternion);
          const sector = Math.atan2(back.x, back.z) / (Math.PI / 8);
          assert.ok(Math.abs(sector - Math.round(sector)) < 1e-8);
          if (x === 510 || x === 520)
            assert.ok(
              lens.quaternion.angleTo(initialRotation) < 1e-7,
              "hold angle until crossing sector boundary",
            );
          if (x === 526)
            assert.ok(
              Math.abs(lens.quaternion.angleTo(initialRotation) - Math.PI / 8) < 1e-7,
              "jump exactly one sprite angle",
            );
        } else if (x === 510) {
          assert.ok(
            lens.quaternion.angleTo(initialRotation) > 0.01,
            "disabling restores continuous rotation",
          );
        }
        assert.ok(
          Math.abs(lens.position.distanceTo(pivot) - initialDistance) < 1e-7,
          `orbit distance at ${fov} degrees`,
        );
        const projected = pivot.clone().project(lens);
        assert.ok(Math.abs(projected.x - 0.25) < 1e-8);
        assert.ok(Math.abs(projected.y + 0.1) < 1e-8);
      }
      pointer("pointerup", 500, 220);
      assert.ok(
        Math.abs(access.activeCamera().position.distanceTo(pivot) - initialDistance) < 1e-7,
      );
    }
  viewport.dispose();
  floor.geometry.dispose();
  floor.material.dispose();
});

test("repeated perspective wheel zoom keeps approaching the ground and enlarging objects", () => {
  for (const fov of [1, 15, 45, 65]) {
    const { viewport } = fixture();
    const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000);
    camera.position.set(0, 3000, 6000);
    camera.lookAt(0, 0, 0);
    camera.updateMatrixWorld();
    const orbit = new OrbitControls(camera);
    Object.assign(viewport, {
      camera,
      orbit,
      frustum: 1000,
      container: { clientWidth: 800, clientHeight: 400 },
      framingBounds: new THREE.Box3(
        new THREE.Vector3(-1800, 0, -1500),
        new THREE.Vector3(1800, 900, 1500),
      ),
    });
    const access = viewport as unknown as { activeCamera(): THREE.Camera };
    viewport.setPerspective(fov);
    const initialHeight = access.activeCamera().position.y;
    const initialSize = new THREE.Vector3(1, 0, 0).project(access.activeCamera()).x;
    for (let step = 1; step <= 8; step++) {
      orbit.dollyIn(0.5);
      const lens = access.activeCamera();
      assert.ok(Math.abs(lens.position.y - initialHeight / 2 ** step) < 1e-7);
      const size = new THREE.Vector3(1, 0, 0).project(lens).x;
      assert.ok(
        Math.abs(size / initialSize - 2 ** step) < 1e-6,
        `continued magnification at ${fov} degrees, step ${step}`,
      );
    }
    for (let step = 0; step < 8; step++) {
      orbit.dollyOut(0.5);
      access.activeCamera();
    }
    assert.ok(Math.abs(access.activeCamera().position.y - initialHeight) < 1e-7);
    viewport.dispose();
  }
});

test("narrow perspective uses tight scene bounds instead of losing depth precision", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000);
  camera.position.set(0, 0, 10000);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  Object.assign(viewport, {
    camera,
    frustum: 1000,
    container: { clientWidth: 800, clientHeight: 400 },
    orbit: { target: new THREE.Vector3() },
    projectionBounds: new THREE.Sphere(new THREE.Vector3(), 3000),
  });
  const access = viewport as unknown as { activeCamera(): THREE.PerspectiveCamera };
  for (const fov of [1, 2, 5, 10, 20, 30]) {
    viewport.setPerspective(fov);
    const lens = access.activeCamera();
    for (const z of [-3000, 3000])
      assert.ok(Math.abs(new THREE.Vector3(0, 0, z).project(lens).z) < 1);
    const a = new THREE.Vector3(0, 0, 0).project(lens).z;
    const b = new THREE.Vector3(0, 0, 0.1).project(lens).z;
    assert.ok(
      (Math.abs(a - b) * 2 ** 24) / 2 > 10,
      `0.1-unit surfaces need distinct depth values at ${fov} degrees`,
    );
  }
  viewport.dispose();
});

test("slider preserves average scale smoothly through the old silhouette switch near 15 degrees", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000);
  camera.position.set(0, 0, 10000);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  const points = [
    new THREE.Vector3(-600, -900, 350),
    new THREE.Vector3(800, 1000, -410),
    new THREE.Vector3(600, -600, 100),
  ];
  Object.assign(viewport, {
    camera,
    frustum: 1000,
    container: { clientWidth: 800, clientHeight: 400 },
    orbit: { target: new THREE.Vector3() },
    framingBounds: new THREE.Box3().setFromPoints(points),
    framingPoints: points,
  });
  const extent = (lens: THREE.Camera) =>
    Math.sqrt(
      points.reduce((sum, p) => {
        const projected = p.clone().project(lens);
        return sum + projected.x ** 2 + projected.y ** 2;
      }, 0) / points.length,
    );
  const expected = extent(camera);
  const access = viewport as unknown as { activeCamera(): THREE.Camera };
  for (const fov of [0, 0.01, ...Array.from({ length: 65 }, (_, i) => i + 1), 15, 0]) {
    viewport.setPerspective(fov);
    assert.ok(
      Math.abs(extent(access.activeCamera()) - expected) < 1e-8,
      `apparent extent at ${fov} degrees`,
    );
  }
  const transition = (2 * Math.atan(1000 / 7600) * 180) / Math.PI;
  const scale = (fov: number) => {
    viewport.setPerspective(fov);
    return new THREE.Vector3(0, 100, 0).project(access.activeCamera()).y;
  };
  const epsilon = 0.001;
  const middle = scale(transition);
  const before = (middle - scale(transition - epsilon)) / epsilon;
  const after = (scale(transition + epsilon) - middle) / epsilon;
  assert.ok(
    Math.abs(before - after) < 1e-6,
    "no sudden change in zoom response when silhouette vertices exchange dominance",
  );
  viewport.dispose();
});

test("both cameras retain separation of nearby surfaces while zooming out", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000, -100000, 100000);
  camera.position.set(0, 0, 10000);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  Object.assign(viewport, {
    camera,
    frustum: 1000,
    container: { clientWidth: 800, clientHeight: 400 },
    orbit: { target: new THREE.Vector3() },
    framingBounds: new THREE.Box3(
      new THREE.Vector3(-800, -900, -1500),
      new THREE.Vector3(800, 900, 1500),
    ),
  });
  const access = viewport as unknown as {
    activeCamera(): THREE.PerspectiveCamera | THREE.OrthographicCamera;
  };
  for (const zoom of [0.1, 0.25, 0.5, 1, 4]) {
    camera.zoom = zoom;
    for (const fov of [0, 1, 15, 30, 65]) {
      viewport.setPerspective(fov);
      const lens = access.activeCamera();
      assert.ok(lens.far - lens.near <= 3256.001);
      const a = new THREE.Vector3(0, 0, 1500).project(lens).z;
      const b = new THREE.Vector3(0, 0, 1500.01).project(lens).z;
      assert.ok(
        (Math.abs(a - b) * 2 ** 24) / 2 > 10,
        `0.01-unit separation at zoom ${zoom}, lens ${fov}`,
      );
    }
  }
  viewport.dispose();
});

test("standalone resources survive undo-style instance removal and retire exactly once", () => {
  const { viewport, publish } = fixture();
  viewport.replaceMap(new THREE.Group(), null, new Map());
  const model = new THREE.Group();
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  model.add(mesh);
  let disposed = 0;
  mesh.geometry.addEventListener("dispose", () => disposed++);
  const reference = {
    id: "house",
    descriptor: "3d-assets/house/asset.json",
    model: "3d-assets/house/model.glb",
    descriptor_sha256: "a".repeat(64),
    model_sha256: "b".repeat(64),
  };
  const sources = new Map([["asset:house:building-000", mesh]]);
  assert.equal(viewport.adoptAsset(reference, model, sources), true);
  assert.equal(viewport.adoptAsset(reference, new THREE.Group(), sources), false);
  assert.throws(
    () =>
      viewport.adoptAsset(
        { ...reference, model_sha256: "c".repeat(64) },
        new THREE.Group(),
        sources,
      ),
    /changed during/,
  );
  const document = documentFixture();
  document.assetSources = [reference];
  document.objects[0].node = "asset:house:building-000";
  publish(document);
  publish({ ...document, objects: [], groups: [] });
  assert.equal(disposed, 0);
  publish(document);
  viewport.replaceMap(new THREE.Group(), null, new Map());
  assert.equal(disposed, 1);
  viewport.dispose();
  assert.equal(disposed, 1);
});

test("asset drops convert the cursor's world ground hit to game coordinates", () => {
  const { viewport, publish } = fixture();
  const document = documentFixture();
  publish({ ...document, objects: [], groups: [] });
  const target = new THREE.Vector3(30, 0, 40);
  const camera = new THREE.OrthographicCamera(-100, 100, 100, -100, 0.1, 10000);
  camera.position.copy(target).add(new THREE.Vector3(100, 200, 300));
  camera.lookAt(target);
  camera.updateMatrixWorld();
  Object.assign(viewport, {
    camera,
    frustum: 100,
    container: { clientWidth: 400, clientHeight: 400 },
    orbit: { target, update() {} },
    renderer: {
      domElement: { getBoundingClientRect: () => ({ left: 20, top: 40, width: 400, height: 400 }) },
    },
  });
  for (const angle of [0, 45]) {
    viewport.setPerspective(angle);
    const position = viewport.assetDropPosition(220, 240)!;
    assert.ok(Math.abs(position[0] - 30) < 1e-6);
    assert.ok(
      Math.abs(position[1] - 40 * Math.sin((document.camera.elevation_deg * Math.PI) / 180)) < 1e-6,
    );
    assert.ok(Math.abs(position[2]) < 1e-6);
  }
  Object.assign(viewport, { renderer: null, orbit: null });
  viewport.dispose();
});

test("endpoint family registration reuses existing models and validates every pin before adoption", () => {
  const { viewport } = fixture();
  viewport.replaceMap(new THREE.Group(), null, new Map());
  const primary = {
    id: "house",
    descriptor: "3d-assets/house/asset.json",
    model: "3d-assets/house/model.glb",
    descriptor_sha256: "a".repeat(64),
    model_sha256: "b".repeat(64),
  };
  const applied = { ...primary, id: "house--state-applied", state_variant: "applied" as const };
  const first = new THREE.Group(),
    second = new THREE.Group();
  const initialNode = new THREE.Group(),
    appliedNode = new THREE.Group();
  first.add(initialNode);
  second.add(appliedNode);
  const initialKey = "asset:house:building-000",
    appliedKey = "asset:house--state-applied:building-001";
  assert.equal(viewport.adoptAsset(primary, first, new Map([[initialKey, initialNode]])), true);
  const sources = new Map([
    [initialKey, new THREE.Group()],
    [appliedKey, appliedNode],
  ]);
  assert.throws(
    () =>
      viewport.adoptAsset({ ...primary, model_sha256: "c".repeat(64) }, second, sources, [applied]),
    /changed during/,
  );
  assert.equal(second.parent, null);
  assert.throws(
    () => viewport.adoptAsset(primary, second, new Map([[initialKey, initialNode]]), [applied]),
    /no new model nodes/,
  );
  assert.equal(second.parent, null);
  assert.equal(viewport.adoptAsset(primary, second, sources, [applied]), true);
  assert.equal(viewport.adoptAsset(primary, new THREE.Group(), sources, [applied]), false);
  viewport.dispose();
});

test("orthographic ground picking includes visible points behind the ray origin", () => {
  const { viewport, publish } = fixture();
  const document = documentFixture();
  publish({ ...document, objects: [], groups: [] });
  const camera = new THREE.OrthographicCamera(-100, 100, 100, -100, -10000, 10000);
  camera.position.set(0, 10, 10);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  Object.assign(viewport, {
    camera,
    orbit: { target: new THREE.Vector3() },
    renderer: {
      domElement: {
        getBoundingClientRect: () => ({ left: 0, top: 0, width: 200, height: 200 }),
      },
    },
  });
  const position = viewport.assetDropPosition(100, 190);
  assert.ok(
    position && position.every(Number.isFinite),
    "lower-screen visible ground must be pickable",
  );
  Object.assign(viewport, { renderer: null, orbit: null });
  viewport.dispose();
});

test("source node names cannot redirect a hit to another asset's wrapper", () => {
  const { viewport, publish } = fixture();
  const document = documentFixture();
  document.objects.push({
    ...structuredClone(document.objects[0]),
    id: "other",
    node: "other-source",
  });
  const source = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  source.name = "other";
  const other = source.clone();
  const asset = new THREE.Group();
  asset.add(source, other);
  viewport.replaceMap(
    asset,
    null,
    new Map([
      ["building-000", source],
      ["other-source", other],
    ]),
  );
  publish(document);
  const internals = viewport as unknown as {
    partViews: Map<string, { wrapper: THREE.Group; meshes: THREE.Mesh[] }>;
    partOfHit(hit: { object: THREE.Object3D }): Level3D["objects"][number] | null;
  };
  const clicked = internals.partViews.get("part")!.meshes[0];
  assert.equal(clicked.name, "other");
  assert.equal(internals.partOfHit({ object: clicked })?.id, "part");
  viewport.dispose();
});

test("invalid wall previews report errors without breaking editing and recover after trimming", () => {
  const errors: string[] = [];
  const viewport = new EditorViewport({
    document: () => null,
    selection: () => null,
    level: () => null,
    showObstacles: () => false,
    showElevation: () => false,
    onSelection: () => {},
    commitTransform: () => {},
    onError: (message) => errors.push(message),
  });
  const source = new THREE.Group();
  for (const x of [0, 200]) {
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(100, 12, 40), new THREE.MeshBasicMaterial());
    mesh.position.x = x;
    source.add(mesh);
  }
  viewport.replaceMap(source, null, new Map([["asset:wall:building-000", source]]));
  const internals = viewport as unknown as { splines: import("./spline-layer.ts").SplineLayer };
  internals.splines.sync(
    [],
    { kind: "oblique-orthographic", elevation_deg: 35 },
    new Map([["asset:wall:building-000", source]]),
  );
  const mode: import("./spline-layer.ts").SplineEditMode = {
    path: {
      id: "wall",
      name: "Wall",
      kind: "wall",
      asset: "wall",
      axis: "x",
      points: [
        [0, 0, 0],
        [300, 0, 0],
      ],
      width: 12,
      repeatLength: 100,
      closed: false,
    },
    drawing: true,
    point: 0,
    append: () => {},
    move: () => {},
    selectPoint: () => {},
  };
  assert.doesNotThrow(() => viewport.setSplineEdit(mode));
  assert.match(errors[0]!, /Wall source has a gap/);
  assert.ok(internals.splines.controls.children.length > 0, "Draft handles must remain editable");
  viewport.setSplineEdit(mode);
  assert.equal(errors.length, 1, "Repeated preview attempts must not spam error dialogs");
  viewport.setSplineEdit({ ...mode, path: { ...mode.path, sourceEnd: 0.3 } });
  assert.ok(
    internals.splines.root.children.length > 1,
    "Trimming to the continuous first segment restores preview",
  );
  assert.doesNotThrow(() => viewport.setSplineEdit(null));
  viewport.dispose();
});

test("incremental edits rebuild changed wall assets and undo restores their geometry", () => {
  const { viewport } = fixture();
  const source = new THREE.Group();
  const short = new THREE.Mesh(new THREE.BoxGeometry(100, 12, 40), new THREE.MeshBasicMaterial());
  const tall = new THREE.Mesh(new THREE.BoxGeometry(100, 12, 80), new THREE.MeshBasicMaterial());
  source.add(short, tall);
  viewport.replaceMap(
    source,
    null,
    new Map([
      ["asset:short:building-000", short],
      ["asset:tall:building-000", tall],
    ]),
  );
  const document = documentFixture();
  document.objects = [];
  document.groups = [];
  document.splines = [
    {
      id: "wall",
      name: "Wall",
      kind: "wall",
      asset: "short",
      axis: "x",
      points: [
        [0, 0, 0],
        [300, 0, 0],
      ],
      width: 12,
      repeatLength: 100,
      closed: false,
    },
  ];
  const layer = (viewport as unknown as { splines: import("./spline-layer.ts").SplineLayer })
    .splines;
  const height = () => {
    layer.root.updateWorldMatrix(true, true);
    return new THREE.Box3().setFromObject(layer.root).getSize(new THREE.Vector3()).y;
  };
  viewport.syncViews(document, false);
  const originalHeight = height();
  const originalMesh = layer.root.children[1];
  viewport.syncViews(document, false);
  assert.equal(layer.root.children[1], originalMesh, "Unchanged splines retain their meshes");
  viewport.syncViews({ ...document, splines: [{ ...document.splines[0]!, asset: "tall" }] }, false);
  assert.ok(
    height() > originalHeight * 1.9,
    "Changing source updates the rendered wall immediately",
  );
  viewport.syncViews(document, false);
  assert.ok(Math.abs(height() - originalHeight) < 1e-5, "Undo restores the prior wall");
  viewport.dispose();
});

test("asset placement samples the edited connected terrain mesh", () => {
  const { viewport, publish } = fixture();
  const document = {
    ...documentFixture(),
    objects: [],
    groups: [],
    size: null,
    terrain: createTerrainGrid([0, 0, 400, 400], 100, 20),
  };
  publish(document);
  const local = gameToScene(document.camera, 200, 200, 20);
  const target = new THREE.Vector3(local[0], local[2], -local[1]);
  const camera = new THREE.OrthographicCamera(-100, 100, 100, -100, 0.1, 10000);
  camera.position.copy(target).add(new THREE.Vector3(0, 500, 0));
  camera.up.set(0, 0, -1);
  camera.lookAt(target);
  camera.updateMatrixWorld();
  Object.assign(viewport, {
    camera,
    frustum: 100,
    container: { clientWidth: 400, clientHeight: 400 },
    orbit: { target, update() {} },
    renderer: {
      domElement: { getBoundingClientRect: () => ({ left: 0, top: 0, width: 400, height: 400 }) },
    },
  });
  const position = viewport.assetDropPosition(200, 200)!;
  assert.ok(Math.abs(position[2] - 20) < 1e-4, `expected terrain height, got ${position}`);
  assert.ok(Math.abs(position[0] - 200) < 1e-4);
  const bounds = viewport.fitExportBounds();
  assert.ok(bounds[2] >= 401);
  Object.assign(viewport, { renderer: null, orbit: null });
  viewport.dispose();
});

test("grid editing uses its own handles and leaves the asset gizmo detached", () => {
  const { viewport, publish } = fixture();
  const grid = createTerrainGrid([100, 100, 200, 200], 100, 40);
  const document = { ...documentFixture(), objects: [], groups: [], terrain: grid };
  publish(document);
  let attached: THREE.Object3D | null = null;
  const gizmo = {
    dragging: false,
    showY: false,
    attach: (node: THREE.Object3D) => {
      attached = node;
    },
    detach: () => {
      attached = null;
    },
  };
  Object.assign(viewport, { gizmo });
  viewport.setTerrainEdit({ grid, camera: document.camera, commit: () => {} });
  assert.equal(attached, null);
  assert.equal(gizmo.showY, false);
  const edited = {
    ...grid,
    vertices: grid.vertices.map((vertex) => ({
      ...vertex,
      position: [vertex.position[0], vertex.position[1], 90] as [number, number, number],
    })),
  };
  viewport.previewTerrain(edited);
  const terrain = (viewport as unknown as { terrain: { root: THREE.Group } }).terrain.root;
  const box = new THREE.Box3().setFromObject(terrain);
  assert.ok(Math.abs(box.min.y - 90 / Math.cos((35 * Math.PI) / 180)) < 0.001);
  viewport.previewTerrain(null);
  viewport.setTerrainEdit(null);
  Object.assign(viewport, { gizmo: null });
  viewport.dispose();
});

test("cardinal and top camera controls preserve target, zoom and lens through quarter turns", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-100, 100, 100, -100, 0.1, 10000);
  const target = new THREE.Vector3(400, 50, -200);
  camera.position.copy(target).add(new THREE.Vector3(100, 200, 300));
  camera.lookAt(target);
  camera.zoom = 2.5;
  const orbit = new OrbitControls(camera);
  orbit.target.copy(target);
  orbit.update();
  Object.assign(viewport, { camera, orbit, frustum: 125, perspective: 30 });
  const beforeTop = camera.quaternion.clone();
  const flight = viewport as unknown as {
    flight: { start: number; ms: number } | null;
    stepFlight(): void;
  };
  viewport.topView();
  assert.ok(camera.quaternion.angleTo(beforeTop) < 1e-7, "top click does not snap the camera");
  assert.equal(flight.flight?.ms, 700);
  flight.flight!.start = performance.now() - 350;
  flight.stepFlight();
  assert.ok(camera.quaternion.angleTo(beforeTop) > 0.1, "top view interpolates halfway through");
  assert.ok(flight.flight);
  flight.flight!.start = performance.now() - 701;
  flight.stepFlight();
  assert.ok(camera.getWorldDirection(new THREE.Vector3()).y < -0.999999);
  const initialRotation = camera.quaternion.clone();
  viewport.setCardinalView("E");
  assert.ok(
    camera.quaternion.angleTo(initialRotation) < 1e-7,
    "cardinal click does not snap the camera",
  );
  assert.equal(orbit.enabled, false);
  assert.equal(flight.flight?.ms, 700, "uses the game-camera transition duration");
  flight.flight!.start = performance.now() - 350;
  flight.stepFlight();
  assert.ok(
    camera.quaternion.angleTo(initialRotation) > 0.1,
    "rotation advances during the transition",
  );
  assert.ok(flight.flight, "transition remains active halfway through");
  flight.flight!.start = performance.now() - 701;
  flight.stepFlight();
  assert.equal(flight.flight, null);
  assert.equal(orbit.enabled, true);
  const up = new THREE.Vector3(0, 1, 0).applyQuaternion(camera.quaternion);
  assert.ok(up.x > 0.999999, "east appears at the top in top view");
  assert.ok(camera.getWorldDirection(new THREE.Vector3()).y < -0.999999);
  for (const turn of [1, -1]) {
    const before = camera.quaternion.clone();
    viewport.rotateViewQuarterTurn(turn);
    assert.ok(camera.quaternion.angleTo(before) < 1e-7, "quarter-turn click does not snap");
    assert.equal(flight.flight?.ms, 700);
    flight.flight!.start = performance.now() - 701;
    flight.stepFlight();
    assert.ok(Math.abs(camera.quaternion.angleTo(before) - Math.PI / 2) < 1e-6);
  }
  assert.ok(new THREE.Vector3(0, 1, 0).applyQuaternion(camera.quaternion).distanceTo(up) < 1e-6);
  assert.deepEqual(orbit.target.toArray(), target.toArray());
  assert.equal(camera.zoom, 2.5);
  assert.equal((viewport as unknown as { perspective: number }).perspective, 30);
  assert.equal((viewport as unknown as { frustum: number }).frustum, 125);
  Object.assign(viewport, { camera: null, orbit: null });
  viewport.dispose();
});

test("asset drag previews follow the slope without mutating the saved height", () => {
  const { viewport, publish } = fixture();
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  const source = new THREE.Group();
  source.add(mesh);
  viewport.replaceMap(source, null, new Map([["building-000", mesh]]));
  const terrain = createTerrainGrid([0, 0, 400, 400], 100, 0);
  terrain.vertices.forEach((vertex) => {
    vertex.position[2] = vertex.position[0] / 2;
  });
  const document = { ...documentFixture(), terrain };
  publish(document);
  viewport.select({ kind: "group", id: "house" });
  const internals = viewport as unknown as {
    groupViews: Map<string, { wrapper: THREE.Object3D }>;
    previewTerrainFollowing(view: unknown): void;
  };
  const view = internals.groupViews.get("house")!;
  for (let repeat = 0; repeat < 2; repeat++) {
    viewport.syncViews(document, false);
    view.wrapper.position.x += 100;
    internals.previewTerrainFollowing(view);
    assert.ok(Math.abs(view.wrapper.position.z - 50 / Math.cos((35 * Math.PI) / 180)) < 1e-8);
  }
  assert.equal(document.groups[0]!.transform.dz, 0);
  viewport.dispose();
});

test("channel previews move attached assets and cancellation restores the committed surface", () => {
  const { viewport, publish } = fixture();
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  const source = new THREE.Group();
  source.add(mesh);
  viewport.replaceMap(source, null, new Map([["building-000", mesh]]));
  const document = { ...documentFixture(), terrain: createTerrainGrid([0, 0, 400, 400], 100, 0) };
  publish(document);
  const river: import("@rle/shared").LevelSpline = {
    id: "river",
    name: "River",
    kind: "river",
    points: [
      [0, 20 + 13 / 3, 0],
      [100, 20 + 13 / 3, 0],
    ],
    width: 60,
    closed: false,
    repeatLength: 32,
    channel: { enabled: true, bedDepth: 20, bankSlope: 1 },
  };
  const view = (
    viewport as unknown as { groupViews: Map<string, { wrapper: THREE.Object3D }> }
  ).groupViews.get("house")!;
  viewport.previewSpline(river);
  assert.ok(Math.abs(view.wrapper.position.z + 20 / Math.cos((35 * Math.PI) / 180)) < 1e-6);
  assert.equal(document.groups[0]!.transform.dz, 0);
  viewport.setSplineEdit(null);
  assert.equal(view.wrapper.position.z, 0);
  viewport.dispose();
});

test("camera turns orbit their focus without cutting inward and interrupt without jumping", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-100, 100, 100, -100, 0.1, 10000);
  const target = new THREE.Vector3(50, 70, -200);
  camera.position.copy(target).add(new THREE.Vector3(0, 200, 300));
  const orbit = new OrbitControls(camera);
  orbit.target.copy(target);
  orbit.update();
  Object.assign(viewport, { camera, orbit, frustum: 125, perspective: 30 });
  const radius = camera.position.distanceTo(target);
  const flight = viewport as unknown as { flight: { start: number } | null; stepFlight(): void };
  viewport.setCardinalView("S");
  for (const elapsed of [100, 250, 350, 500, 650]) {
    flight.flight!.start = performance.now() - elapsed;
    flight.stepFlight();
    assert.ok(Math.abs(camera.position.distanceTo(target) - radius) < 1e-6);
    assert.ok(
      camera
        .getWorldDirection(new THREE.Vector3())
        .dot(target.clone().sub(camera.position).normalize()) >
        1 - 1e-8,
      "focus stays centered",
    );
  }
  const position = camera.position.clone(),
    rotation = camera.quaternion.clone();
  viewport.topView();
  assert.ok(camera.position.distanceTo(position) < 1e-8);
  assert.ok(camera.quaternion.angleTo(rotation) < 1e-7);
  flight.flight!.start = performance.now() - 701;
  flight.stepFlight();
  const completed = camera.position.clone();
  orbit.update();
  assert.ok(
    camera.position.distanceTo(completed) < 1e-6,
    "returning control to orbit does not jump",
  );
  assert.ok(Math.abs(camera.position.distanceTo(target) - radius) < 1e-6);
  Object.assign(viewport, { camera: null, orbit: null });
  viewport.dispose();
});

test("lit terrain casts and receives shadows after replacement and retirement", () => {
  const { viewport, publish } = fixture();
  const document = {
    ...documentFixture(),
    objects: [],
    groups: [],
    terrain: createTerrainGrid([0, 0, 100, 100], 100, 0),
    lighting: { enabled: true, sunAzimuth: 90, sunElevation: 45, shadowOpacity: 0.5 },
  };
  const internal = viewport as unknown as {
    terrain: { root: THREE.Group };
    sunlight: { root: THREE.Group };
  };
  const receivers = () => {
    const meshes: THREE.Mesh[] = [];
    internal.sunlight.root.traverse((node) => {
      if (node instanceof THREE.Mesh) meshes.push(node);
    });
    return meshes;
  };
  publish(document);
  assert.equal(receivers().length, 0, "Lit terrain does not need a shadow overlay");
  const original = internal.terrain.root.children[0] as THREE.Mesh;
  assert.ok(original.material instanceof THREE.MeshLambertMaterial);
  assert.equal(original.receiveShadow, true);
  assert.equal(original.castShadow, true);
  const raised = {
    ...document,
    terrain: {
      ...document.terrain,
      vertices: document.terrain.vertices.map((v) => ({
        ...v,
        position: [v.position[0], v.position[1], 40] as [number, number, number],
      })),
    },
  };
  publish(raised);
  const replacement = internal.terrain.root.children[0] as THREE.Mesh;
  assert.notEqual(replacement.geometry, original.geometry);
  assert.equal(
    replacement.material,
    original.material,
    "Height edits retain terrain materials and textures",
  );
  assert.equal(replacement.receiveShadow, true);
  assert.equal(replacement.castShadow, true);
  assert.equal(receivers().length, 0);
  publish({ ...document, terrain: undefined });
  assert.equal(receivers().length, 0);
  viewport.dispose();
});

test("incremental revisions refresh sun settings and invalidate cached shadows", () => {
  const { viewport, publish } = fixture();
  const document = {
    ...documentFixture(),
    objects: [],
    groups: [],
    terrain: createTerrainGrid([0, 0, 400, 400], 100, 0),
  };
  const internal = viewport as unknown as {
    sunlight: { sun: THREE.DirectionalLight };
    renderer: { shadowMap: { enabled: boolean; needsUpdate: boolean } } | null;
  };
  const shadowMap = { enabled: false, needsUpdate: false };
  publish(document);
  internal.renderer = { shadowMap };
  try {
    const lighting = { enabled: true, sunAzimuth: 90, sunElevation: 35, shadowOpacity: 0.7 };
    publish({ ...document, lighting }, false, true);
    assert.equal(internal.sunlight.sun.visible, true);
    assert.equal(shadowMap.enabled, true);
    assert.equal(shadowMap.needsUpdate, true);
    const position = internal.sunlight.sun.position.clone();
    shadowMap.needsUpdate = false;
    publish(
      { ...document, lighting: { ...lighting, sunAzimuth: 270, shadowOpacity: 0.3 } },
      false,
      true,
    );
    assert.ok(internal.sunlight.sun.position.distanceTo(position) > 100);
    assert.equal(internal.sunlight.sun.shadow.intensity, 0.3);
    assert.equal(shadowMap.needsUpdate, true);
    shadowMap.needsUpdate = false;
    publish({ ...document, lighting: { ...lighting, enabled: false } }, false, true);
    assert.equal(internal.sunlight.sun.visible, false);
    assert.equal(shadowMap.enabled, false);
    assert.equal(shadowMap.needsUpdate, true);
  } finally {
    internal.renderer = null;
    viewport.dispose();
  }
});

test("elevation overlay follows modeled terrain revisions without reference-map data", () => {
  let enabled = true;
  const { viewport, publish } = fixture(() => enabled);
  const terrain = createTerrainGrid([0, 0, 128, 128], 128);
  for (const vertex of terrain.vertices) vertex.position[2] = vertex.position[0];
  const document = { ...documentFixture(), objects: [], groups: [], terrain };
  const overlay = (viewport as unknown as { overlayRoot: THREE.Group }).overlayRoot;
  const positions = () =>
    (overlay.children[0] as THREE.LineSegments).geometry.getAttribute("instanceStart");
  publish(document);
  assert.ok(positions().count > 0);
  const initial = positions();
  let disposed = false;
  (overlay.children[0] as THREE.LineSegments).geometry.addEventListener(
    "dispose",
    () => (disposed = true),
  );
  const flattened = { ...document, terrain: createTerrainGrid([0, 0, 128, 128], 128, 0) };
  publish(flattened, false, true);
  assert.equal(overlay.children.length, 0);
  assert.ok(disposed);
  publish(document, false, true);
  assert.deepEqual(Array.from(positions().array), Array.from(initial.array));
  enabled = false;
  viewport.buildOverlays();
  assert.equal(overlay.children.length, 0);
  viewport.dispose();
});

test("terrain drag previews coalesce and cancellation discards queued work", () => {
  const { viewport } = fixture();
  const internal = viewport as unknown as {
    terrainControls: { preview(grid: Level3D["terrain"] | null): void };
    flushTerrainPreview(): void;
  };
  const seen: (Level3D["terrain"] | null)[] = [];
  viewport.previewTerrain = (grid) => {
    seen.push(grid);
  };
  const a = createTerrainGrid([0, 0, 100, 100], 100, 10);
  const b = createTerrainGrid([0, 0, 100, 100], 100, 20);
  internal.terrainControls.preview(a);
  internal.terrainControls.preview(b);
  assert.equal(seen.length, 0);
  internal.flushTerrainPreview();
  internal.flushTerrainPreview();
  assert.deepEqual(seen, [b]);
  internal.terrainControls.preview(a);
  internal.terrainControls.preview(null);
  internal.flushTerrainPreview();
  assert.deepEqual(seen, [b, null]);
  internal.terrainControls.preview(a);
  viewport.setTerrainEdit(null);
  internal.flushTerrainPreview();
  assert.deepEqual(seen, [b, null]);
  viewport.dispose();
});

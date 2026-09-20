import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import {
  gameToScene,
  parseLevel3D,
  type GameTransform,
  type Level3D,
} from "@rle/shared";
import type { Selection } from "./document-commands.ts";
import { EditorViewport } from "./editor-viewport.ts";

function fixture() {
  let document: Level3D | null = null;
  let selection: Selection = null;
  const viewport = new EditorViewport({
    document: () => document,
    selection: () => selection,
    level: () => null,
    showObstacles: () => false,
    showElevation: () => false,
    onSelection: (next) => {
      selection = next;
    },
    commitTransform: () => {
      throw new Error("Unexpected gesture");
    },
  });
  return {
    viewport,
    selection: () => selection,
    publish: (next: Level3D) => {
      document = next;
      viewport.syncViews(next);
    },
  };
}

test("map replacement owns retirement and releases detached ground exactly once", () => {
  const { viewport } = fixture();
  const asset = new THREE.Group();
  const ground = new THREE.Mesh(
    new THREE.BoxGeometry(),
    new THREE.MeshBasicMaterial(),
  );
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
  assert.throws(
    () => viewport.replaceMap(new THREE.Group(), null, new Map()),
    /Disposed/,
  );
});

function documentFixture() {
  return parseLevel3D({
    version: 1,
    map: "York",
    glb: "york.glb",
    size: [100, 200],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    groups: [
      { id: "house", transform: { dx: 10, dy: 20, dz: 0, rot_deg: 15 } },
    ],
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

test("scene revisions retire selection without disposing shared reconstruction resources", () => {
  const { viewport, publish, selection } = fixture();
  const document = documentFixture();
  const mesh = new THREE.Mesh(
    new THREE.BoxGeometry(),
    new THREE.MeshBasicMaterial(),
  );
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
  assert.throws(
    () => publish(documentFixture()),
    /Missing source node building-000/,
  );
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
    partOfHit(hit: {
      object: THREE.Object3D;
    }): Level3D["objects"][number] | null;
  };
  const mesh = new THREE.Mesh(
    new THREE.BoxGeometry(),
    new THREE.MeshBasicMaterial(),
  );
  const asset = new THREE.Group();
  asset.add(mesh);
  viewport.replaceMap(asset, null, new Map([["building-000", mesh]]));
  viewport.syncViews(document);
  viewport.select({ kind: "part", id: "part" });
  assert.ok(attached);
  const wrapper = attached as THREE.Object3D;
  const picked = wrapper.children[0]!.children[0]!;
  assert.equal(callbacks.partOfHit({ object: picked }), document.objects[0]);
  const delta = gameToScene(document.camera, 7, -3, 2);
  wrapper.position.add(new THREE.Vector3(...delta));
  callbacks.commitGizmo();
  assert.deepEqual(commits, [{ dx: 8, dy: -1, dz: 2, rot_deg: 0 }]);
  assert.equal(callbacks.partOfHit({ object: picked }), document.objects[0]);
  callbacks.commitGizmo();
  assert.equal(
    commits.length,
    1,
    "unchanged gizmo must not append another history revision",
  );
  viewport.dispose();
  assert.equal(attached, null);
});

test("perspective preserves target-plane framing, scales by distance, and returns to orthographic", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-200, 200, 100, -100, -100000, 100000);
  camera.position.set(0, 0, 1000);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  Object.assign(viewport, { camera, frustum: 100, container: { clientWidth: 800, clientHeight: 400 }, orbit: { target: new THREE.Vector3() } });
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
  const hit = ray.ray.intersectPlane(new THREE.Plane(new THREE.Vector3(0, 0, 1), 0), new THREE.Vector3());
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
  const bounds = new THREE.Box3(new THREE.Vector3(-800, -900, -1500), new THREE.Vector3(800, 900, 1500));
  Object.assign(viewport, { camera, frustum: 1000, container: { clientWidth: 800, clientHeight: 400 }, orbit: { target: new THREE.Vector3() }, framingBounds: bounds });
  const access = viewport as unknown as { activeCamera(): THREE.PerspectiveCamera };
  for (const zoom of [0.5, 1, 4]) {
    camera.zoom = zoom;
    camera.updateProjectionMatrix();
    const expected = new THREE.Vector3(800, 900, 1500).project(camera).y;
    for (const fov of [1, 15, 30, 45, 65]) {
      viewport.setPerspective(fov);
      const actual = new THREE.Vector3(800, 900, 1500).project(access.activeCamera()).y;
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
  Object.assign(viewport, { camera, orbit, frustum: 1000, container: { clientWidth: 800, clientHeight: 400 }, framingBounds: new THREE.Box3(new THREE.Vector3(-800, -900, -1500), new THREE.Vector3(800, 900, 1500)) });
  const access = viewport as unknown as { activeCamera(): THREE.Camera };
  for (const fov of [1, 15, 45, 65]) {
    viewport.setPerspective(fov);
    const lens = access.activeCamera();
    const initial = lens.position.clone();
    const target = orbit.target.clone();
    const controlHeight = camera.position.y;
    for (const [dx, dy] of [[50, 80], [-120, 40], [20, -170]]) {
      orbit.pan(dx!, dy!);
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
  Object.assign(viewport, { camera, orbit, frustum: 1000, container: { clientWidth: 800, clientHeight: 400 }, framingBounds: new THREE.Box3(new THREE.Vector3(-1800, -200, -500), new THREE.Vector3(1800, 900, 500)) });
  const access = viewport as unknown as {
    activeCamera(): THREE.Camera;
    setupCursorOrbit(el: HTMLCanvasElement): void;
    objectsRoot: THREE.Group;
  };
  const floor = new THREE.Mesh(new THREE.PlaneGeometry(20000, 20000), new THREE.MeshBasicMaterial({ side: THREE.DoubleSide }));
  access.objectsRoot.add(floor);
  floor.updateWorldMatrix(true, false);
  const element = Object.assign(new EventTarget(), {
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 400 }),
    setPointerCapture() {}, releasePointerCapture() {}, hasPointerCapture: () => false,
  });
  access.setupCursorOrbit(element as unknown as HTMLCanvasElement);
  const pointer = (type: string, x: number, y: number) => element.dispatchEvent(Object.assign(new Event(type), { button: 2, pointerId: 1, clientX: x, clientY: y }));
  for (const snap of [true, false]) for (const fov of [0, 1, 15, 45, 65]) {
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
    const pivot = ray.ray.intersectPlane(new THREE.Plane(new THREE.Vector3(0, 1, 0), 0), new THREE.Vector3())!;
    assert.ok(pivot);
    const initialDistance = access.activeCamera().position.distanceTo(pivot);
    pointer("pointerdown", 500, 220);
    for (const [x, y] of [[510, 220], [520, 220], [526, 220], [600, 220], [680, 160], [440, 260], [500, 220]]) {
      pointer("pointermove", x!, y!);
      const lens = access.activeCamera();
      if (snap) {
        const back = new THREE.Vector3(0, 0, 1).applyQuaternion(lens.quaternion);
        const sector = Math.atan2(back.x, back.z) / (Math.PI / 8);
        assert.ok(Math.abs(sector - Math.round(sector)) < 1e-8);
        if (x === 510 || x === 520) assert.ok(lens.quaternion.angleTo(initialRotation) < 1e-7, "hold angle until crossing sector boundary");
        if (x === 526) assert.ok(Math.abs(lens.quaternion.angleTo(initialRotation) - Math.PI / 8) < 1e-7, "jump exactly one sprite angle");
      } else if (x === 510) {
        assert.ok(lens.quaternion.angleTo(initialRotation) > 0.01, "disabling restores continuous rotation");
      }
      assert.ok(Math.abs(lens.position.distanceTo(pivot) - initialDistance) < 1e-7, `orbit distance at ${fov} degrees`);
      const projected = pivot.clone().project(lens);
      assert.ok(Math.abs(projected.x - 0.25) < 1e-8);
      assert.ok(Math.abs(projected.y + 0.1) < 1e-8);
    }
    pointer("pointerup", 500, 220);
    assert.ok(Math.abs(access.activeCamera().position.distanceTo(pivot) - initialDistance) < 1e-7);
  }
  viewport.dispose();
  floor.geometry.dispose(); floor.material.dispose();
});

test("repeated perspective wheel zoom keeps approaching the ground and enlarging objects", () => {
  for (const fov of [1, 15, 45, 65]) {
    const { viewport } = fixture();
    const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000);
    camera.position.set(0, 3000, 6000);
    camera.lookAt(0, 0, 0);
    camera.updateMatrixWorld();
    const orbit = new OrbitControls(camera);
    Object.assign(viewport, { camera, orbit, frustum: 1000, container: { clientWidth: 800, clientHeight: 400 }, framingBounds: new THREE.Box3(new THREE.Vector3(-1800, 0, -1500), new THREE.Vector3(1800, 900, 1500)) });
    const access = viewport as unknown as { activeCamera(): THREE.Camera };
    viewport.setPerspective(fov);
    const initialHeight = access.activeCamera().position.y;
    const initialSize = new THREE.Vector3(1, 0, 0).project(access.activeCamera()).x;
    for (let step = 1; step <= 8; step++) {
      orbit.dollyIn(0.5);
      const lens = access.activeCamera();
      assert.ok(Math.abs(lens.position.y - initialHeight / 2 ** step) < 1e-7);
      const size = new THREE.Vector3(1, 0, 0).project(lens).x;
      assert.ok(Math.abs(size / initialSize - 2 ** step) < 1e-6, `continued magnification at ${fov} degrees, step ${step}`);
    }
    for (let step = 0; step < 8; step++) { orbit.dollyOut(0.5); access.activeCamera(); }
    assert.ok(Math.abs(access.activeCamera().position.y - initialHeight) < 1e-7);
    viewport.dispose();
  }
});

test("narrow perspective uses tight scene bounds instead of losing depth precision", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000);
  camera.position.set(0, 0, 10000); camera.lookAt(0, 0, 0); camera.updateMatrixWorld();
  Object.assign(viewport, { camera, frustum: 1000, container: { clientWidth: 800, clientHeight: 400 }, orbit: { target: new THREE.Vector3() }, projectionBounds: new THREE.Sphere(new THREE.Vector3(), 3000) });
  const access = viewport as unknown as { activeCamera(): THREE.PerspectiveCamera };
  for (const fov of [1, 2, 5, 10, 20, 30]) {
    viewport.setPerspective(fov);
    const lens = access.activeCamera();
    for (const z of [-3000, 3000]) assert.ok(Math.abs(new THREE.Vector3(0, 0, z).project(lens).z) < 1);
    const a = new THREE.Vector3(0, 0, 0).project(lens).z;
    const b = new THREE.Vector3(0, 0, 0.1).project(lens).z;
    assert.ok(Math.abs(a - b) * (2 ** 24) / 2 > 10, `0.1-unit surfaces need distinct depth values at ${fov} degrees`);
  }
  viewport.dispose();
});

test("lens framing follows real geometry rather than empty bounding-box corners", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000);
  camera.position.set(0, 0, 10000);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  const points = [new THREE.Vector3(-600, -900, 1400), new THREE.Vector3(800, 1000, -1500), new THREE.Vector3(600, -600, 1000)];
  Object.assign(viewport, { camera, frustum: 1000, container: { clientWidth: 800, clientHeight: 400 }, orbit: { target: new THREE.Vector3() }, framingBounds: new THREE.Box3().setFromPoints(points), framingPoints: points });
  const extent = (lens: THREE.Camera) => Math.max(...points.map(p => { const projected = p.clone().project(lens); return Math.max(Math.abs(projected.x), Math.abs(projected.y)); }));
  const expected = extent(camera);
  const access = viewport as unknown as { activeCamera(): THREE.Camera };
  for (const fov of [0, 1, 5, 15, 30, 45, 65, 15, 0]) {
    viewport.setPerspective(fov);
    assert.ok(Math.abs(extent(access.activeCamera()) - expected) < 1e-8, `apparent extent at ${fov} degrees`);
  }
  viewport.dispose();
});

test("both cameras retain separation of nearby surfaces while zooming out", () => {
  const { viewport } = fixture();
  const camera = new THREE.OrthographicCamera(-2000, 2000, 1000, -1000, -100000, 100000);
  camera.position.set(0, 0, 10000);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  Object.assign(viewport, { camera, frustum: 1000, container: { clientWidth: 800, clientHeight: 400 }, orbit: { target: new THREE.Vector3() }, framingBounds: new THREE.Box3(new THREE.Vector3(-800, -900, -1500), new THREE.Vector3(800, 900, 1500)) });
  const access = viewport as unknown as { activeCamera(): THREE.PerspectiveCamera | THREE.OrthographicCamera };
  for (const zoom of [0.1, 0.25, 0.5, 1, 4]) {
    camera.zoom = zoom;
    for (const fov of [0, 1, 15, 30, 65]) {
      viewport.setPerspective(fov);
      const lens = access.activeCamera();
      assert.ok(lens.far - lens.near <= 3256.001);
      const a = new THREE.Vector3(0, 0, 1500).project(lens).z;
      const b = new THREE.Vector3(0, 0, 1500.01).project(lens).z;
      assert.ok(Math.abs(a - b) * (2 ** 24) / 2 > 10, `0.01-unit separation at zoom ${zoom}, lens ${fov}`);
    }
  }
  viewport.dispose();
});

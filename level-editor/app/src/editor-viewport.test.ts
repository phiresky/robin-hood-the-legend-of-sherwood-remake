import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
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

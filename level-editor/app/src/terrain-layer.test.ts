import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { createTerrainGrid, type Level3D } from "@rle/shared";
import { TerrainLayer } from "./terrain-layer.ts";

test("local terrain edits retain distant sections and match a clean rebuild", () => {
  const terrain = createTerrainGrid([0, 0, 1024, 1024], 64);
  const document = {
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    terrain,
  } as Level3D;
  const layer = new TerrainLayer();
  layer.sync(document);
  const initial = layer.root.children.slice() as THREE.Mesh[];
  let geometryDisposals = 0,
    materialDisposals = 0;
  for (const mesh of initial) mesh.geometry.addEventListener("dispose", () => geometryDisposals++);
  const material = initial[0]!.material as THREE.MeshLambertMaterial;
  material.addEventListener("dispose", () => materialDisposals++);
  const changed = {
    ...document,
    terrain: {
      ...terrain,
      vertices: terrain.vertices.map((v, i) =>
        i === 0 ? { ...v, position: [0, 0, 40] as [number, number, number] } : v,
      ),
    },
  };
  layer.sync(changed);
  assert.ok(layer.root.children.some((mesh) => initial.includes(mesh as THREE.Mesh)));
  assert.ok(geometryDisposals > 0 && geometryDisposals < initial.length);
  assert.equal(materialDisposals, 0);
  const clean = new TerrainLayer();
  clean.sync(changed);
  const snapshot = (root: THREE.Group) =>
    root.children
      .map((node) => {
        const mesh = node as THREE.Mesh;
        return JSON.stringify({
          cells: mesh.userData.terrainCells,
          attributes: Object.fromEntries(
            Object.entries(mesh.geometry.attributes).map(([name, attribute]) => [
              name,
              Array.from(attribute.array),
            ]),
          ),
          indices: Array.from(mesh.geometry.index!.array),
        });
      })
      .sort();
  assert.deepEqual(snapshot(layer.root), snapshot(clean.root));
  const rotatedCamera = { ...changed, camera: { ...changed.camera, elevation_deg: 50 } };
  layer.sync(rotatedCamera);
  clean.sync(rotatedCamera);
  assert.deepEqual(snapshot(layer.root), snapshot(clean.root));
  layer.clear();
  clean.clear();
  assert.equal(materialDisposals, 1);
});

import assert from "node:assert/strict";
import * as THREE from "three";
import { loadSceneModel } from "../../pipeline/src/scene-assets.ts";
import { contentBakeBounds } from "../src/map-bake-render.ts";
import { partMatrix } from "../../shared/src/level3d.ts";
import { gltfToScene, applyAffineMatrix } from "../../shared/src/geometry.ts";

// Static saved-scene verification uses the same vertex-based frame as the
// editor. Texture decoding and a GPU are unnecessary for this bounds query.
export async function savedMapBakeBounds(document, library = "library") {
  if (document.exportBounds) return document.exportBounds;
  if (document.size) return [0, 0, ...document.size];
  assert.ok(
    !document.terrain && !document.splines?.length,
    "Automatic diagnostic bounds for generated terrain/splines require a browser bake or explicit export frame",
  );
  const root = new THREE.Group();
  const sources = new Map();
  const add = (node, placement) => {
    const extras = node.getExtras();
    const controlled = [
      "reveal_material_patch",
      "reveal_hide_when_applied",
      "reveal_show_when_applied",
    ].some((key) => extras[key] !== undefined);
    if (extras.default_hidden && !controlled) return;
    const matrix = node.getWorldMatrix();
    for (const primitive of node.getMesh()?.listPrimitives() ?? []) {
      const positions = primitive.getAttribute("POSITION");
      assert.ok(positions, "Saved mesh is missing vertex positions");
      const values = new Float64Array(positions.getCount() * 3);
      for (let i = 0; i < positions.getCount(); i++) {
        const point = positions.getElement(i, [0, 0, 0]);
        const world = new THREE.Vector3(...point).applyMatrix4(
          new THREE.Matrix4().fromArray(matrix),
        );
        let scene = gltfToScene(world.toArray());
        if (placement) scene = applyAffineMatrix(placement, scene);
        values.set(scene, i * 3);
      }
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.BufferAttribute(values, 3));
      root.add(new THREE.Mesh(geometry));
    }
    for (const child of node.listChildren()) add(child, placement);
  };
  try {
    const visible = document.objects.filter(
      (object) =>
        !object.hidden &&
        !document.groups.some((group) => group.id === object.group && group.hidden),
    );
    for (const reference of document.assetSources ?? []) {
      const objects = visible.filter((object) => object.node.startsWith(`asset:${reference.id}:`));
      if (!objects.length) continue;
      const model = await loadSceneModel(library, reference);
      sources.set(reference.id, model);
      const nodes = [];
      model
        .getRoot()
        .getDefaultScene()
        .traverse((node) => nodes.push(node));
      for (const object of objects) {
        const name = object.node.slice(`asset:${reference.id}:`.length);
        const matches = nodes.filter((node) => node.getName() === name);
        assert.equal(matches.length, 1, `Ambiguous bounds source: ${object.node}`);
        add(matches[0], partMatrix(document.camera, document, object));
      }
    }
    for (const reference of document.sceneAssets.filter((source) => source.role === "ground")) {
      const model = sources.get(reference.id) ?? (await loadSceneModel(library, reference));
      for (const child of model.getRoot().getDefaultScene().listChildren()) add(child);
    }
    const bounds = contentBakeBounds(root, document.camera, true);
    return [bounds[0], bounds[1], bounds[2] + 1, bounds[3] + 1];
  } finally {
    root.traverse((node) => {
      if (node instanceof THREE.Mesh) {
        node.geometry.dispose();
        node.material.dispose();
      }
    });
  }
}

import assert from "node:assert/strict";
import * as THREE from "three";
import { loadSceneModel } from "../../pipeline/src/scene-assets.ts";
import { prepareWallGameplayAssets } from "../src/wall-gameplay-calibration.ts";

// Reconstruct geometry-only source nodes in the editor's asset-local Z-up frame.
// Wall calibration needs positions and transforms, not textures or GPU resources.
export async function savedMapWallCalibration(document, descriptors, library = "library") {
  const ids = new Set(
    (document.splines ?? [])
      .filter((path) => path.kind === "wall")
      .flatMap((path) => [path.asset, path.cornerAsset])
      .filter(Boolean),
  );
  const sources = new Map();
  const geometries = [];
  const material = new THREE.MeshBasicMaterial();
  const warnings = [];
  const convert = (node) => {
    const group = new THREE.Group();
    group.name = node.getName();
    group.userData = structuredClone(node.getExtras());
    group.matrixAutoUpdate = false;
    group.matrix.fromArray(node.getMatrix());
    for (const primitive of node.getMesh()?.listPrimitives() ?? []) {
      assert.equal(primitive.getMode(), 4, "Wall calibration requires triangle meshes");
      const positions = primitive.getAttribute("POSITION");
      assert.ok(positions, "Wall mesh is missing positions");
      const geometry = new THREE.BufferGeometry();
      geometries.push(geometry);
      geometry.setAttribute("position", new THREE.BufferAttribute(positions.getArray(), 3));
      const indices = primitive.getIndices();
      if (indices) geometry.setIndex(new THREE.BufferAttribute(indices.getArray(), 1));
      group.add(new THREE.Mesh(geometry, material));
    }
    for (const child of node.listChildren()) group.add(convert(child));
    return group;
  };
  try {
    for (const id of ids) {
      const descriptor = descriptors.get(id);
      if (
        !descriptor?.gameplay ||
        descriptor.states ||
        descriptor.gameplay.movementTransitions?.length
      )
        continue;
      try {
        const reference = document.assetSources?.find((reference) => reference.id === id);
        assert.ok(reference, `Missing pinned wall asset: ${id}`);
        const model = await loadSceneModel(library, reference);
        const root = model.getRoot().getDefaultScene() ?? model.getRoot().listScenes()[0];
        assert.ok(root, `Wall model has no scene: ${id}`);
        const map = root.listChildren().find((node) => node.getName() === "map");
        assert.ok(map && map.listChildren().length === 1, `Invalid standalone wall: ${id}`);
        const group = map.listChildren()[0];
        assert.deepEqual(group.getMatrix(), new THREE.Matrix4().toArray(), "Unbaked wall group");
        for (const part of descriptor.parts) {
          const matches = group.listChildren().filter((node) => node.getName() === part.node);
          assert.equal(matches.length, 1, `Missing or ambiguous wall part: ${id}/${part.node}`);
          sources.set(`asset:${id}:${part.node}`, convert(matches[0]));
        }
      } catch (error) {
        warnings.push(`Wall asset ${id}: diagnostic mesh load unavailable: ${String(error)}`);
      }
    }
    const prepared = prepareWallGameplayAssets(document, descriptors, sources);
    return { assets: prepared.assets, warnings: [...warnings, ...prepared.warnings] };
  } finally {
    for (const geometry of geometries) geometry.dispose();
    material.dispose();
  }
}

import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { maskReviewMesh } from "./mask-review-mesh.mjs";

test("GPU mask review retains per-material sidedness, shared alpha texture and vertex sampling", () => {
  const triangle = [
    [0, 0, 0],
    [4, 0, 0],
    [0, 4, 0],
  ];
  const sampling = {
    uv: [
      [-1, 0],
      [2, 0],
      [0, 1],
    ],
    alpha: [0.25, 0.5, 1],
    cutoff: 0.5,
    texture: 0,
    wrap: ["repeat", "mirror"],
  };
  const mask = {
    triangles: [triangle, triangle],
    cullBackfaces: true,
    alphaCoverage: {
      textures: [{ width: 2, height: 1, alphaBase64: "/wA=" }],
      triangles: [sampling, { ...sampling, doubleSided: true }],
    },
  };
  const rendered = maskReviewMesh(mask, ([x, y, z]) => [x + 10, y, z]);
  const [front, double] = rendered.root.children;
  assert.equal(front.material.side, THREE.FrontSide);
  assert.equal(double.material.side, THREE.DoubleSide);
  assert.deepEqual([...front.geometry.getAttribute("uv").array], sampling.uv.flat());
  assert.deepEqual([...front.geometry.getAttribute("coverageAlpha").array], sampling.alpha);
  assert.deepEqual(front.material.uniforms.wrapMode.value.toArray(), [1, 2]);
  assert.equal(
    front.material.uniforms.alphaTexture.value,
    double.material.uniforms.alphaTexture.value,
  );
  assert.deepEqual([...front.material.uniforms.alphaTexture.value.image.data], [255, 0]);
  assert.equal(front.geometry.getAttribute("position").getX(0), 10);
  let disposed = 0;
  front.material.uniforms.alphaTexture.value.addEventListener("dispose", () => disposed++);
  rendered.dispose();
  assert.equal(disposed, 1);
});

test("legacy triangle review remains opaque and rejects mismatched compact metadata", () => {
  const mask = {
    triangles: [
      [
        [0, 0, 0],
        [4, 0, 0],
        [0, 4, 0],
      ],
    ],
    cullBackfaces: true,
  };
  const rendered = maskReviewMesh(mask, (p) => p);
  assert.equal(rendered.root.children[0].material.uniforms.hasTexture.value, false);
  assert.equal(rendered.root.children[0].material.side, THREE.FrontSide);
  rendered.dispose();
  assert.throws(
    () => maskReviewMesh({ ...mask, alphaCoverage: { textures: [], triangles: [] } }, (p) => p),
    /match/,
  );
});

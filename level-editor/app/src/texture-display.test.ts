import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { TextureDisplay } from "./texture-display.ts";

test("smooth and pixel modes apply to all imported material textures", () => {
  const map = new THREE.Texture();
  map.magFilter = THREE.NearestFilter;
  const normalMap = new THREE.Texture();
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial({ map, normalMap }));
  const display = new TextureDisplay();
  display.apply(mesh, 8);
  for (const texture of [map, normalMap]) {
    assert.equal(texture.magFilter, THREE.LinearFilter);
    assert.equal(texture.minFilter, THREE.LinearMipmapLinearFilter);
    assert.equal(texture.anisotropy, 8);
    assert.equal(texture.generateMipmaps, true);
  }
  display.smooth = false;
  display.apply(mesh, 8);
  assert.equal(map.magFilter, THREE.NearestFilter);
  assert.equal(map.minFilter, THREE.NearestFilter);
  assert.equal(map.anisotropy, 1);
});

test("ownership shader preserves opaque geometry and toggles through a shared uniform, including selected clones", () => {
  const display = new TextureDisplay();
  const material = new THREE.MeshBasicMaterial();
  material.userData.source_ownership_fill = "synthesized";
  display.material(material);
  const selected = material.clone();
  display.material(selected);
  for (const candidate of [material, selected]) {
    const shader = { uniforms: {}, vertexShader: "", fragmentShader: "#include <map_fragment>" } as THREE.WebGLProgramParametersWithUniforms;
    candidate.onBeforeCompile(shader, {} as THREE.WebGLRenderer);
    assert.equal(shader.uniforms.showSynthesized, display.synthesized);
    assert.match(shader.fragmentShader, /sampledDiffuseColor\.a = 1\.0/);
    assert.match(shader.fragmentShader, /mix\(vec3\(0\.24\), sampledDiffuseColor\.rgb, sampledDiffuseColor\.a\)/);
    assert.equal(candidate.transparent, false);
  }
  display.synthesized.value = false;
  const untouched = new THREE.MeshBasicMaterial();
  const originalHook = untouched.onBeforeCompile;
  display.material(untouched);
  assert.equal(untouched.onBeforeCompile, originalHook);
});

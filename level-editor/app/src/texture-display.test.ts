import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { TextureDisplay } from "./texture-display.ts";

test("smooth and pixel modes apply to all imported material textures", () => {
  const map = new THREE.Texture();
  map.magFilter = THREE.NearestFilter;
  const normalMap = new THREE.Texture();
  const mesh = new THREE.Mesh(
    new THREE.BoxGeometry(),
    new THREE.MeshStandardMaterial({ map, normalMap }),
  );
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
    const shader = {
      uniforms: {},
      vertexShader: "",
      fragmentShader: "#include <map_fragment>",
    } as THREE.WebGLProgramParametersWithUniforms;
    candidate.onBeforeCompile(shader, {} as THREE.WebGLRenderer);
    assert.equal(shader.uniforms.showSynthesized, display.synthesized);
    assert.match(shader.fragmentShader, /sampledDiffuseColor\.a = 1\.0/);
    assert.match(
      shader.fragmentShader,
      /mix\(vec3\(0\.24\), sampledDiffuseColor\.rgb, sampledDiffuseColor\.a\)/,
    );
    assert.equal(candidate.transparent, false);
  }
  display.synthesized.value = false;
  const untouched = new THREE.MeshBasicMaterial();
  const originalHook = untouched.onBeforeCompile;
  display.material(untouched);
  assert.equal(untouched.onBeforeCompile, originalHook);
});

function foliageMaterial(paired = false) {
  const material = new THREE.MeshBasicMaterial({
    map: new THREE.Texture(),
    vertexColors: true,
    alphaTest: 0.5,
    side: paired ? THREE.FrontSide : THREE.DoubleSide,
  });
  material.userData = {
    foliage_physical_opacity: true,
    opacity_semantics: "physical-coverage",
    source_ownership_semantics: "separate-mask",
    source_ownership_channel: "vertex-color-r",
    source_ownership_backface: "inferred",
    foliage_backface_fill: "neutral",
    ...(paired ? { foliage_card_sides: "paired-one-sided" } : {}),
  };
  return material;
}

test("foliage source-only shading keeps physical alpha and bypasses provenance tint, including clones", () => {
  const display = new TextureDisplay();
  for (const material of [foliageMaterial(), foliageMaterial(true)]) {
    display.material(material);
    const clone = material.clone();
    display.material(clone);
    for (const candidate of [material, clone]) {
      const shader = {
        uniforms: {},
        vertexShader: "",
        fragmentShader:
          "#include <map_fragment>\n#include <color_fragment>\n#include <alphatest_fragment>",
      } as THREE.WebGLProgramParametersWithUniforms;
      candidate.onBeforeCompile(shader, {} as THREE.WebGLRenderer);
      assert.match(shader.fragmentShader, /#include <map_fragment>/);
      assert.match(shader.fragmentShader, /#include <alphatest_fragment>/);
      assert.doesNotMatch(shader.fragmentShader, /sampledDiffuseColor\.a = 1/);
      assert.doesNotMatch(shader.fragmentShader, /#include <color_fragment>/);
      assert.match(shader.fragmentShader, /gl_FrontFacing/);
      assert.match(shader.fragmentShader, /vColor.r/);
      assert.equal(candidate.alphaTest, 0.5);
      assert.equal(candidate.transparent, false);
      assert.equal(shader.uniforms.showSynthesized, display.synthesized);
    }
  }
});

test("foliage fails closed without independent evidence or correct physical alpha settings", () => {
  const display = new TextureDisplay();
  for (const alter of [
    (m: THREE.MeshBasicMaterial) => {
      delete m.userData.source_ownership_channel;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.alphaTest = 0;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.transparent = true;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.vertexColors = false;
    },
  ]) {
    const material = foliageMaterial();
    alter(material);
    assert.throws(() => display.material(material), /Foliage requires/);
  }
  const mesh = new THREE.Mesh(new THREE.PlaneGeometry(), foliageMaterial());
  assert.throws(() => display.apply(mesh), /missing.*COLOR_0/);
});

test("foliage shader variants do not share a program when reverse evidence differs", () => {
  const display = new TextureDisplay();
  const neutral = foliageMaterial(),
    generated = foliageMaterial();
  delete generated.userData.foliage_backface_fill;
  display.material(neutral);
  display.material(generated);
  assert.notEqual(neutral.customProgramCacheKey(), generated.customProgramCacheKey());
});

test("authored room floors keep depth priority without changing shared wall materials", () => {
  const texture = new THREE.Texture();
  const original = new THREE.MeshBasicMaterial({ map: texture });
  const geometry = new THREE.PlaneGeometry();
  const floor = new THREE.Mesh(geometry, original);
  floor.userData = { projection_component: "patch-006-room-floor" };
  const wall = new THREE.Mesh(geometry, original);
  const walkway = new THREE.Mesh(geometry, original);
  walkway.userData = { projection_component: "walkway" };
  const root = new THREE.Group();
  root.add(floor, wall, walkway);
  const display = new TextureDisplay();
  display.apply(root);
  assert.notEqual(floor.material, original);
  assert.equal(floor.material.polygonOffset, true);
  assert.equal(floor.material.polygonOffsetFactor, -2);
  assert.equal(floor.material.polygonOffsetUnits, -2);
  assert.equal(floor.material.map, texture);
  assert.equal(floor.geometry, geometry);
  assert.equal(original.polygonOffset, false);
  for (const mesh of [wall, walkway]) assert.equal(mesh.material, original);
  const configured = floor.material;
  display.apply(root);
  assert.equal(floor.material, configured);
});


test("complementary ownership stays nearest under smooth display without changing shared textures", () => {
  const shared = new THREE.Texture();
  const composite = new THREE.MeshStandardMaterial({ map: shared, emissiveMap: shared, alphaTest: 0.5 });
  composite.userData.export_complementary_binary_ownership = true;
  const ordinary = new THREE.MeshBasicMaterial({ map: shared });
  const root = new THREE.Group();
  root.add(new THREE.Mesh(new THREE.PlaneGeometry(), composite), new THREE.Mesh(new THREE.PlaneGeometry(), ordinary));
  const display = new TextureDisplay();
  display.apply(root, 8);
  const mask = composite.map;
  for (const texture of [composite.map!, composite.emissiveMap!]) {
    assert.notEqual(texture, shared);
    assert.equal(texture.magFilter, THREE.NearestFilter);
    assert.equal(texture.minFilter, THREE.NearestFilter);
    assert.equal(texture.generateMipmaps, false);
    assert.equal(texture.anisotropy, 1);
  }
  assert.equal(shared.magFilter, THREE.LinearFilter);
  assert.equal(shared.minFilter, THREE.LinearMipmapLinearFilter);
  assert.equal(shared.anisotropy, 8);
  display.smooth = false;
  display.apply(root, 8);
  display.smooth = true;
  display.apply(root, 8);
  assert.equal(composite.map, mask);
  assert.equal(mask!.magFilter, THREE.NearestFilter);
  assert.equal(shared.magFilter, THREE.LinearFilter);
});

test("complementary ownership rejects a nonbinary material contract", () => {
  const material = new THREE.MeshStandardMaterial({ map: new THREE.Texture() });
  material.userData.export_complementary_binary_ownership = true;
  assert.throws(() => new TextureDisplay().apply(new THREE.Mesh(new THREE.PlaneGeometry(), material)), /binary MASK/);
});

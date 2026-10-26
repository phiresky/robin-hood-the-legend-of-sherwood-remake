import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { TextureDisplay, retainDisplayMaterialVariants } from "./texture-display.ts";

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

test("smooth toggles preserve authored BC7 mip levels without requesting GPU generation", () => {
  const mipmaps = [4, 2, 1].map((size, level) => ({
    width: size,
    height: size,
    data: new Uint8Array(16).fill(level + 1),
  }));
  const texture = new THREE.CompressedTexture(mipmaps, 4, 4, THREE.RGBA_BPTC_Format);
  const source = texture.source;
  const bytes = mipmaps.map((level) => level.data.slice());
  const mesh = new THREE.Mesh(
    new THREE.BoxGeometry(),
    new THREE.MeshBasicMaterial({ map: texture }),
  );
  const display = new TextureDisplay();
  // Also repair a texture configured by an earlier display pass.
  texture.generateMipmaps = true;
  for (const smooth of [true, false, true]) {
    display.smooth = smooth;
    display.apply(mesh, 8);
    assert.equal(texture.generateMipmaps, false);
    assert.equal(texture.magFilter, smooth ? THREE.LinearFilter : THREE.NearestFilter);
    assert.equal(texture.minFilter, smooth ? THREE.LinearMipmapLinearFilter : THREE.NearestFilter);
    assert.equal(texture.anisotropy, smooth ? 8 : 1);
    assert.equal(texture.mipmaps, mipmaps);
    assert.equal(texture.source, source);
    assert.equal(texture.format, THREE.RGBA_BPTC_Format);
    mipmaps.forEach((level, index) => assert.deepEqual(level.data, bytes[index]));
    const version = texture.version;
    display.apply(mesh, 8);
    assert.equal(texture.version, version, "unchanged settings must not re-upload compressed data");
  }
});

test("base-level compressed textures use linear filtering without requiring missing mip levels", () => {
  const texture = new THREE.CompressedTexture(
    [{ width: 4, height: 4, data: new Uint8Array(16) }],
    4,
    4,
    THREE.RGBA_BPTC_Format,
  );
  const mesh = new THREE.Mesh(
    new THREE.BoxGeometry(),
    new THREE.MeshBasicMaterial({ map: texture }),
  );
  const display = new TextureDisplay();
  for (const smooth of [true, false, true]) {
    display.smooth = smooth;
    display.apply(mesh);
    assert.equal(texture.generateMipmaps, false);
    assert.equal(texture.minFilter, smooth ? THREE.LinearFilter : THREE.NearestFilter);
    assert.equal(texture.magFilter, smooth ? THREE.LinearFilter : THREE.NearestFilter);
    assert.equal(texture.mipmaps.length, 1);
  }
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


test("complementary masks stay nearest while RGB keeps the smooth display preference", () => {
  const shared = new THREE.Texture();
  const composite = new THREE.MeshStandardMaterial({ map: shared, emissiveMap: shared, alphaTest: 0.5 });
  composite.userData.export_complementary_binary_ownership = true;
  const ordinary = new THREE.MeshBasicMaterial({ map: shared });
  const root = new THREE.Group();
  root.add(new THREE.Mesh(new THREE.PlaneGeometry(), composite), new THREE.Mesh(new THREE.PlaneGeometry(), ordinary));
  const display = new TextureDisplay();
  display.apply(root, 8);
  const mask = composite.map;
  assert.notEqual(composite.map, shared);
  assert.equal(composite.map!.magFilter, THREE.NearestFilter);
  assert.equal(composite.map!.minFilter, THREE.NearestFilter);
  assert.equal(composite.map!.generateMipmaps, false);
  assert.equal(composite.map!.anisotropy, 1);
  assert.equal(composite.emissiveMap, shared);
  assert.equal(composite.emissiveMap!.magFilter, THREE.LinearFilter);
  assert.equal(composite.emissiveMap!.minFilter, THREE.LinearMipmapLinearFilter);
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

test("explicit projected RGB sampling stays nearest without affecting shared wall RGB", () => {
  const shared = new THREE.Texture();
  const hole = new THREE.MeshStandardMaterial({ map: shared, emissiveMap: shared, alphaTest: 0.5 });
  hole.userData.export_complementary_binary_ownership = true;
  hole.userData.export_rgb_sampling = "nearest-required";
  const wall = new THREE.MeshStandardMaterial({ map: shared, emissiveMap: shared, alphaTest: 0.5 });
  wall.userData.export_complementary_binary_ownership = true;
  const root = new THREE.Group();
  root.add(new THREE.Mesh(new THREE.PlaneGeometry(), hole), new THREE.Mesh(new THREE.PlaneGeometry(), wall));
  const display = new TextureDisplay();
  display.apply(root, 8);
  const rgb = hole.emissiveMap;
  assert.notEqual(rgb, shared);
  for (const texture of [hole.map!, rgb!]) {
    assert.equal(texture.magFilter, THREE.NearestFilter);
    assert.equal(texture.minFilter, THREE.NearestFilter);
    assert.equal(texture.generateMipmaps, false);
    assert.equal(texture.anisotropy, 1);
  }
  display.smooth = false;
  display.apply(root, 8);
  display.smooth = true;
  display.apply(root, 8);
  assert.equal(hole.emissiveMap, rgb);
  assert.equal(rgb!.magFilter, THREE.NearestFilter);
  assert.equal(wall.emissiveMap, shared);
  assert.equal(shared.magFilter, THREE.LinearFilter);
  assert.equal(shared.minFilter, THREE.LinearMipmapLinearFilter);
  assert.equal(shared.generateMipmaps, true);
  assert.equal(shared.anisotropy, 8);
});

function opaqueSourceAtlas(map: THREE.Texture) {
  const material = new THREE.MeshBasicMaterial({ map });
  material.userData = {
    export_rgb_sampling: "nearest-required",
    source_ownership_fill: "synthesized",
    source_ownership_alpha: "one=observed,zero=inferred;material remains opaque",
  };
  return material;
}

test("opaque source atlases isolate nearest sampling from shared ordinary textures", () => {
  const pixels = new Uint8Array([49, 32, 8, 255, 74, 48, 16, 0]);
  const shared = new THREE.DataTexture(pixels, 2, 1);
  const source = opaqueSourceAtlas(shared);
  const ordinary = new THREE.MeshBasicMaterial({ map: shared });
  const root = new THREE.Group();
  root.add(
    new THREE.Mesh(new THREE.PlaneGeometry(), source),
    new THREE.Mesh(new THREE.PlaneGeometry(), ordinary),
  );
  const display = new TextureDisplay();
  display.apply(root, 8);
  const isolated = source.map!;
  assert.notEqual(isolated, shared);
  assert.equal(isolated.source, shared.source);
  for (const smooth of [false, true]) {
    display.smooth = smooth;
    for (const synthesized of [false, true]) {
      display.synthesized.value = synthesized;
      display.apply(root, 8);
      assert.equal(source.map, isolated);
      assert.equal(isolated.magFilter, THREE.NearestFilter);
      assert.equal(isolated.minFilter, THREE.NearestFilter);
      assert.equal(isolated.generateMipmaps, false);
      assert.equal(isolated.anisotropy, 1);
      assert.equal(shared.magFilter, smooth ? THREE.LinearFilter : THREE.NearestFilter);
      assert.equal(shared.generateMipmaps, smooth);
      assert.equal(ordinary.map, shared);
      assert.deepEqual(pixels, new Uint8Array([49, 32, 8, 255, 74, 48, 16, 0]));
      assert.equal(source.opacity, 1);
      assert.equal(source.transparent, false);
      assert.equal(source.alphaTest, 0);
    }
  }
});

test("inactive opaque source frames are configured before selection and only cloned once", () => {
  const shared = new THREE.Texture();
  const first = opaqueSourceAtlas(shared);
  const next = opaqueSourceAtlas(shared);
  const mesh = new THREE.Mesh(new THREE.PlaneGeometry(), first);
  retainDisplayMaterialVariants(mesh, [first, next]);
  const display = new TextureDisplay();
  display.apply(mesh);
  const firstMap = first.map!;
  const nextMap = next.map!;
  assert.notEqual(firstMap, shared);
  assert.notEqual(nextMap, shared);
  assert.notEqual(firstMap, nextMap);
  mesh.material = next;
  display.apply(mesh);
  for (const [material, texture] of [
    [first, firstMap],
    [next, nextMap],
  ] as const) {
    assert.equal(material.map, texture);
    assert.equal(texture.magFilter, THREE.NearestFilter);
    assert.equal(texture.minFilter, THREE.NearestFilter);
    assert.equal(texture.generateMipmaps, false);
  }
});

test("opaque nearest sampling rejects ambiguous ownership or physical transparency", () => {
  const alterations = [
    (m: THREE.MeshBasicMaterial) => {
      delete m.userData.source_ownership_alpha;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.userData.source_ownership_fill = "neutral";
    },
    (m: THREE.MeshBasicMaterial) => {
      m.transparent = true;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.opacity = 0.5;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.alphaTest = 0.5;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.alphaHash = true;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.alphaToCoverage = true;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.alphaMap = new THREE.Texture();
    },
    (m: THREE.MeshBasicMaterial) => {
      m.premultipliedAlpha = true;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.blending = THREE.AdditiveBlending;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.map!.premultiplyAlpha = true;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.map = null;
    },
  ];
  for (const alter of alterations) {
    const material = opaqueSourceAtlas(new THREE.Texture());
    alter(material);
    assert.throws(
      () => new TextureDisplay().apply(new THREE.Mesh(new THREE.PlaneGeometry(), material)),
      /Required opaque RGB sampling/,
    );
  }
  const lit = new THREE.MeshStandardMaterial({ map: new THREE.Texture() });
  lit.userData = opaqueSourceAtlas(new THREE.Texture()).userData;
  assert.throws(
    () => new TextureDisplay().apply(new THREE.Mesh(new THREE.PlaneGeometry(), lit)),
    /Required opaque RGB sampling/,
  );
  const configured = opaqueSourceAtlas(new THREE.Texture());
  const mesh = new THREE.Mesh(new THREE.PlaneGeometry(), configured);
  const display = new TextureDisplay();
  display.apply(mesh);
  configured.opacity = 0.5;
  assert.throws(() => display.apply(mesh), /Required opaque RGB sampling/);
});

function nonmipmappedSourceAtlas(map: THREE.Texture) {
  const material = new THREE.MeshBasicMaterial({ map });
  material.userData = {
    export_rgb_sampling: "linear-no-mipmap-required",
    source_ownership_bake: true,
    reprojection_source_sha256: "a".repeat(64),
    source_ownership_fill: "neutral",
  };
  return material;
}

test("loaded packed atlases keep linear/nearest choice without mipmaps or shared-map changes", () => {
  const pixels = new Uint8Array([240, 220, 160, 255, 60, 60, 60, 255]);
  const shared = new THREE.DataTexture(pixels, 2, 1);
  const authored = nonmipmappedSourceAtlas(shared);
  const loaded = new THREE.MaterialLoader()
    .setTextures({ [shared.uuid]: shared })
    .parse(JSON.parse(JSON.stringify(authored.toJSON()))) as THREE.MeshBasicMaterial;
  const unmarked = authored.clone();
  delete unmarked.userData.export_rgb_sampling;
  const root = new THREE.Group();
  root.add(
    new THREE.Mesh(new THREE.PlaneGeometry(), loaded),
    new THREE.Mesh(new THREE.PlaneGeometry(), unmarked),
  );
  const display = new TextureDisplay();
  display.apply(root, 8);
  const isolated = loaded.map!;
  assert.notEqual(isolated, shared);
  assert.equal(isolated.source, shared.source);
  for (const smooth of [true, false, true]) {
    display.smooth = smooth;
    display.apply(root, 8);
    assert.equal(loaded.map, isolated);
    assert.equal(isolated.magFilter, smooth ? THREE.LinearFilter : THREE.NearestFilter);
    assert.equal(isolated.minFilter, smooth ? THREE.LinearFilter : THREE.NearestFilter);
    assert.equal(isolated.generateMipmaps, false);
    assert.equal(isolated.anisotropy, smooth ? 8 : 1);
    assert.equal(unmarked.map, shared);
    assert.equal(shared.minFilter, smooth ? THREE.LinearMipmapLinearFilter : THREE.NearestFilter);
    assert.equal(shared.generateMipmaps, smooth);
    assert.equal(loaded.userData.source_ownership_fill, "neutral");
    assert.deepEqual(pixels, new Uint8Array([240, 220, 160, 255, 60, 60, 60, 255]));
  }
});

test("inactive nonmipmapped atlas materials are configured before their phase becomes visible", () => {
  const shared = new THREE.Texture();
  const first = nonmipmappedSourceAtlas(shared);
  const later = nonmipmappedSourceAtlas(shared);
  const mesh = new THREE.Mesh(new THREE.PlaneGeometry(), first);
  retainDisplayMaterialVariants(mesh, [first, later]);
  const display = new TextureDisplay();
  display.apply(mesh);
  const laterMap = later.map;
  assert.notEqual(laterMap, shared);
  mesh.material = later;
  display.smooth = false;
  display.apply(mesh);
  display.smooth = true;
  display.apply(mesh);
  assert.equal(later.map, laterMap);
  assert.equal(later.map!.minFilter, THREE.LinearFilter);
  assert.equal(later.map!.generateMipmaps, false);
});

test("nonmipmapped atlas marker rejects missing provenance and incompatible coverage", () => {
  const alterations = [
    (m: THREE.MeshBasicMaterial) => {
      delete m.userData.source_ownership_bake;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.userData.reprojection_source_sha256 = "unbound";
    },
    (m: THREE.MeshBasicMaterial) => {
      m.transparent = true;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.opacity = 0.5;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.alphaTest = 0.5;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.alphaMap = new THREE.Texture();
    },
    (m: THREE.MeshBasicMaterial) => {
      m.map = null;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.map!.premultiplyAlpha = true;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.userData.export_complementary_binary_ownership = true;
    },
    (m: THREE.MeshBasicMaterial) => {
      m.map = new THREE.CompressedTexture();
    },
  ];
  for (const alter of alterations) {
    const material = nonmipmappedSourceAtlas(new THREE.Texture());
    alter(material);
    assert.throws(
      () => new TextureDisplay().apply(new THREE.Mesh(new THREE.PlaneGeometry(), material)),
      /Required nonmipmapped atlas sampling/,
    );
  }
  const lit = new THREE.MeshStandardMaterial({ map: new THREE.Texture() });
  lit.userData = nonmipmappedSourceAtlas(new THREE.Texture()).userData;
  assert.throws(
    () => new TextureDisplay().apply(new THREE.Mesh(new THREE.PlaneGeometry(), lit)),
    /Required nonmipmapped atlas sampling/,
  );
});

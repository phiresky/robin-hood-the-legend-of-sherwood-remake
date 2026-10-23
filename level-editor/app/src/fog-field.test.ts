import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import {
  ambianceFieldSlopeTexture,
  ambianceFieldTexture,
  ambianceFieldUniforms,
  applyAmbianceField,
  injectAmbianceField,
  isAmbianceNativeMaterial,
  loadAmbianceColorField,
  setAmbianceFieldUniforms,
  validateAmbianceColorField,
  type AmbianceColorField,
} from "./fog-field.ts";
import { TextureDisplay, hasQualifiedSourceDisplayShader } from "./texture-display.ts";

const FOG = "3ea7839c0d8faaed810b716fb774869b39cb2edd2366ee82c9109d3fedf04cb2";
function field(overrides: Partial<AmbianceColorField> = {}): AmbianceColorField {
  return {
    version: 1,
    kind: "native-ambiance-color-field",
    map: "york",
    ambiance: "Fog",
    camera_elevation_deg: 35,
    source: {
      day: { path: "Data/Levels/Day/york.map.png", sha256: "a".repeat(64) },
      fog: { path: "Data/Levels/Fog/york.map.png", sha256: FOG },
    },
    map_size: [128, 64],
    grid: [2, 1],
    block: 64,
    space: "srgb-unorm",
    slope: [0.5, 1],
    offset: [0.25, 0.25, 0.25, 0, 0, 0],
    ...overrides,
  };
}

test("field validation rejects grids that do not tile the map and out-of-range coefficients", () => {
  assert.equal(validateAmbianceColorField(field()).map, "york");
  assert.throws(() => validateAmbianceColorField(field({ grid: [3, 1] })), /tile/);
  assert.throws(() => validateAmbianceColorField(field({ slope: [0, 1] })), /slope/);
  assert.throws(() => validateAmbianceColorField(field({ offset: [0, 0, 0] })), /offset/);
  assert.throws(() => validateAmbianceColorField({ ...field(), ambiance: "Attack" } as never), /Fog, Night/);
  // A Night field must name the Night painting it was fitted on.
  assert.throws(() => validateAmbianceColorField({ ...field(), ambiance: "Night" }), /source\.night/);
});

test("CPU reference interpolates between tile centres and clamps at the map border", () => {
  const f = field();
  // Left tile centre: 0.5 * 100/255 + 0.25 -> 0.446 -> 114
  assert.deepEqual(applyAmbianceField(f, 32, 32, [100, 100, 100]), [114, 114, 114]);
  // Right tile centre: identity.
  assert.deepEqual(applyAmbianceField(f, 96, 32, [100, 100, 100]), [100, 100, 100]);
  // Halfway between centres: slope 0.75, offset 0.125.
  assert.deepEqual(applyAmbianceField(f, 64, 32, [100, 100, 100]), [107, 107, 107]);
  assert.deepEqual(applyAmbianceField(f, 0, 0, [100, 100, 100]), [114, 114, 114]);
});

test("field texture stores offset in RGB and slope in alpha, top tile row first", () => {
  const texture = ambianceFieldTexture(field());
  const data = texture.image.data as Uint16Array;
  assert.equal(texture.type, THREE.HalfFloatType);
  assert.equal(texture.flipY, false);
  assert.equal(THREE.DataUtils.fromHalfFloat(data[0]!), 0.25);
  assert.equal(THREE.DataUtils.fromHalfFloat(data[3]!), 0.5);
  assert.equal(THREE.DataUtils.fromHalfFloat(data[7]!), 1);
});

test("Fog-painted textures are exempt from the field", () => {
  const f = field();
  const projected = new THREE.MeshBasicMaterial();
  projected.userData.reprojection_source_sha256 = FOG;
  const tagged = new THREE.MeshBasicMaterial();
  tagged.userData.source_ambiance = "Fog";
  assert.equal(isAmbianceNativeMaterial(projected, f), true);
  assert.equal(isAmbianceNativeMaterial(tagged, f), true);
  assert.equal(isAmbianceNativeMaterial(new THREE.MeshBasicMaterial(), f), false);
});

test("injection adds the world varying and recolours the final sRGB output only when enabled", () => {
  const shader = {
    uniforms: {},
    vertexShader: THREE.ShaderLib.basic.vertexShader,
    fragmentShader: THREE.ShaderLib.basic.fragmentShader,
  } as unknown as THREE.WebGLProgramParametersWithUniforms;
  const uniforms = ambianceFieldUniforms();
  injectAmbianceField(shader, uniforms);
  assert.match(shader.vertexShader, /vFogFieldWorld = \( modelMatrix \* fogFieldWorld \)\.xyz;/);
  assert.match(shader.fragmentShader, /if \( fogFieldEnabled \)[\s\S]*#include <opaque_fragment>/);
  assert.equal(shader.uniforms.fogFieldEnabled, uniforms.fogFieldEnabled);
  assert.throws(
    () =>
      injectAmbianceField(
        { uniforms: {}, vertexShader: "void main(){}", fragmentShader: "void main(){}" } as never,
        uniforms,
      ),
    /standard/,
  );
});

test("TextureDisplay installs no hook until a field is set, then keeps the shader qualified", () => {
  const display = new TextureDisplay();
  const day = new THREE.MeshBasicMaterial({ map: new THREE.Texture() });
  const fog = new THREE.MeshBasicMaterial({ map: new THREE.Texture() });
  fog.userData.reprojection_source_sha256 = FOG;
  const root = new THREE.Group();
  root.add(new THREE.Mesh(new THREE.BoxGeometry(), day), new THREE.Mesh(new THREE.BoxGeometry(), fog));
  const standardCompile = THREE.Material.prototype.onBeforeCompile;
  display.apply(root);
  assert.equal(day.onBeforeCompile, standardCompile, "Day sessions keep the standard shader");
  const renderer = {} as THREE.WebGLRenderer;
  display.setAmbianceField(field(), renderer);
  display.apply(root);
  assert.notEqual(day.onBeforeCompile, standardCompile);
  assert.match(day.customProgramCacheKey(), /native-ambiance-field-v1/);
  assert.equal(hasQualifiedSourceDisplayShader(day), true);
  assert.equal(fog.onBeforeCompile, standardCompile, "Fog-painted texture untouched");
  const compile = (r: unknown) => {
    const shader = {
      uniforms: {},
      vertexShader: THREE.ShaderLib.basic.vertexShader,
      fragmentShader: THREE.ShaderLib.basic.fragmentShader,
    } as unknown as THREE.WebGLProgramParametersWithUniforms;
    day.onBeforeCompile(shader, r as THREE.WebGLRenderer);
    return shader.fragmentShader.includes("fogFieldEnabled");
  };
  assert.equal(compile(renderer), true);
  assert.equal(compile({}), false, "other renderers (map bake) keep Day colours");
  display.setAmbianceField(null, renderer);
  assert.equal(display.ambiance.fogFieldEnabled.value, false);
  assert.equal(display.ambiance.fogFieldMap.value, null);
});

test("uniform state follows the field elevation and map size", () => {
  const uniforms = ambianceFieldUniforms();
  setAmbianceFieldUniforms(uniforms, field());
  assert.equal(uniforms.fogFieldEnabled.value, true);
  assert.deepEqual(uniforms.fogFieldMapSize.value.toArray(), [128, 64]);
  assert.ok(Math.abs(uniforms.fogFieldElevation.value.x - Math.sin((35 * Math.PI) / 180)) < 1e-12);
});

function memoryRoot(files: Record<string, string>): FileSystemDirectoryHandle {
  const dir = (prefix: string): FileSystemDirectoryHandle =>
    ({
      kind: "directory",
      name: prefix,
      async getDirectoryHandle(name: string) {
        const p = prefix ? `${prefix}/${name}` : name;
        if (!Object.keys(files).some((k) => k.startsWith(p + "/")))
          throw new DOMException("missing", "NotFoundError");
        return dir(p);
      },
      async getFileHandle(name: string) {
        const p = prefix ? `${prefix}/${name}` : name;
        if (!(p in files)) throw new DOMException("missing", "NotFoundError");
        return { kind: "file", name, getFile: async () => new File([files[p]!], name) };
      },
    }) as unknown as FileSystemDirectoryHandle;
  return dir("");
}
async function digest(text: string) {
  const bytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(bytes), (n) => n.toString(16).padStart(2, "0")).join("");
}

test("library loader: no catalog means no field; a pinned row must match its bytes", async () => {
  assert.equal(await loadAmbianceColorField(memoryRoot({}), "york", "Fog"), null);
  const body = JSON.stringify(field());
  const catalog = async (sha256: string) =>
    JSON.stringify({ version: 1, entries: [{ map: "york", ambiance: "Fog", field: { path: "ambiance/york.json", sha256 } }] });
  const ok = memoryRoot({ "ambiance/index.json": await catalog(await digest(body)), "ambiance/york.json": body });
  assert.equal((await loadAmbianceColorField(ok, "york", "Fog"))?.map, "york");
  assert.equal(await loadAmbianceColorField(ok, "derby", "Fog"), null);
  const changed = memoryRoot({ "ambiance/index.json": await catalog("0".repeat(64)), "ambiance/york.json": body });
  await assert.rejects(loadAmbianceColorField(changed, "york", "Fog"), /changed/);
});

const NIGHT = "f1cdd57e62c620b765a43e5f41e66791671cd97018e00e7579a05b86b1d05043";
function nightField(overrides: Partial<AmbianceColorField> = {}): AmbianceColorField {
  return field({
    ambiance: "Night",
    source: {
      day: { path: "Data/Levels/Day/york.map.png", sha256: "a".repeat(64) },
      night: { path: "Data/Levels/Night/york.map.png", sha256: NIGHT },
    },
    slope_channels: 3,
    // Left tile: R 0.5, G 0.25, B 1; right tile: identity.
    slope: [0.5, 0.25, 1, 1, 1, 1],
    offset: [0, 0.2, 0, 0, 0, 0],
    ...overrides,
  });
}

test("Night fields carry one slope per channel and validate their slope count", () => {
  assert.equal(validateAmbianceColorField(nightField()).ambiance, "Night");
  assert.throws(() => validateAmbianceColorField(nightField({ slope: [0.5, 1] })), /slope/);
  assert.throws(() => validateAmbianceColorField(nightField({ slope_channels: 2 as never })), /slope_channels/);
  // Shared-slope Night fields remain valid (slope_channels defaults to 1).
  assert.equal(validateAmbianceColorField(nightField({ slope_channels: undefined, slope: [0.5, 1] })).slope.length, 2);
});

test("per-channel CPU reference applies each channel's own slope", () => {
  // Left tile centre: R 100*0.5 = 50, G 100*0.25 + 0.2*255 = 76, B unchanged.
  assert.deepEqual(applyAmbianceField(nightField(), 32, 32, [100, 100, 100]), [50, 76, 100]);
  assert.deepEqual(applyAmbianceField(nightField(), 96, 32, [100, 100, 100]), [100, 100, 100]);
});

test("per-channel fields upload a slope texture; shared fields keep slope in alpha", () => {
  const f = nightField();
  const offset = ambianceFieldTexture(f).image.data as Uint16Array,
    slope = ambianceFieldSlopeTexture(f)!.image.data as Uint16Array;
  assert.equal(THREE.DataUtils.fromHalfFloat(offset[3]!), 1);
  assert.deepEqual([0, 1, 2].map((c) => THREE.DataUtils.fromHalfFloat(slope[c]!)), [0.5, 0.25, 1]);
  assert.equal(ambianceFieldSlopeTexture(field()), null);
  const uniforms = ambianceFieldUniforms();
  setAmbianceFieldUniforms(uniforms, f);
  assert.equal(uniforms.fogFieldRgbSlope.value, true);
  setAmbianceFieldUniforms(uniforms, field());
  assert.equal(uniforms.fogFieldRgbSlope.value, false);
  assert.equal(uniforms.fogFieldSlopeMap.value, null);
  setAmbianceFieldUniforms(uniforms, null);
  assert.equal(uniforms.fogFieldEnabled.value, false);
});

test("Night-painted textures are exempt from the Night field, Fog-painted ones are not", () => {
  const f = nightField();
  const night = new THREE.MeshBasicMaterial();
  night.userData.reprojection_source_sha256 = NIGHT;
  const tagged = new THREE.MeshBasicMaterial();
  tagged.userData.source_ambiance = "Night";
  const fog = new THREE.MeshBasicMaterial();
  fog.userData.source_ambiance = "Fog";
  assert.equal(isAmbianceNativeMaterial(night, f), true);
  assert.equal(isAmbianceNativeMaterial(tagged, f), true);
  assert.equal(isAmbianceNativeMaterial(fog, f), false);
});

test("the shader selects the per-channel slope texture with a uniform", () => {
  const shader = {
    uniforms: {},
    vertexShader: THREE.ShaderLib.basic.vertexShader,
    fragmentShader: THREE.ShaderLib.basic.fragmentShader,
  } as unknown as THREE.WebGLProgramParametersWithUniforms;
  const uniforms = ambianceFieldUniforms();
  injectAmbianceField(shader, uniforms);
  assert.match(shader.fragmentShader, /fogFieldRgbSlope \? texture2D\( fogFieldSlopeMap, fogFieldUv \)\.rgb : vec3\( fogField\.a \)/);
  assert.equal(shader.uniforms.fogFieldSlopeMap, uniforms.fogFieldSlopeMap);
});

test("library loader returns the row of the requested ambiance", async () => {
  const fogBody = JSON.stringify(field()),
    nightBody = JSON.stringify(nightField());
  const root = memoryRoot({
    "ambiance/index.json": JSON.stringify({
      version: 1,
      entries: [
        { map: "york", ambiance: "Fog", field: { path: "ambiance/fog.json", sha256: await digest(fogBody) } },
        { map: "york", ambiance: "Night", field: { path: "ambiance/night.json", sha256: await digest(nightBody) } },
      ],
    }),
    "ambiance/fog.json": fogBody,
    "ambiance/night.json": nightBody,
  });
  assert.equal((await loadAmbianceColorField(root, "york", "Night"))?.slope_channels, 3);
  assert.equal((await loadAmbianceColorField(root, "york", "Fog"))?.ambiance, "Fog");
});

import * as THREE from "three";
import { safeLibraryPath } from "@rle/shared";
import { isNotFound, readJson, subdir } from "./fs.ts";

/** A native ambiance colour field fitted from the original Day and Fog paintings of one map.
 * Per tile, Fog ≈ slope · Day + offset in 8-bit sRGB (normalized), bilinearly interpolated
 * between tile centres. Day-painted textures are recoloured with it while a Fog mission is open;
 * textures already projected from the Fog painting are left unchanged. */
export interface AmbianceColorField {
  version: 1;
  kind: "native-ambiance-color-field";
  map: string;
  ambiance: "Fog";
  camera_elevation_deg: number;
  source: { day: { path: string; sha256: string }; fog: { path: string; sha256: string } };
  map_size: [number, number];
  grid: [number, number];
  block: number;
  space: "srgb-unorm";
  /** Row-major from the top tile row: grid[0] * grid[1] contrast values. */
  slope: number[];
  /** Row-major from the top tile row: 3 * grid[0] * grid[1] RGB offsets in 0..1. */
  offset: number[];
}

interface CatalogEntry {
  map: string;
  ambiance: string;
  field: { path: string; sha256: string };
}

const sha = /^[a-f0-9]{64}$/;
export function validateAmbianceColorField(value: unknown): AmbianceColorField {
  const fail = (why: string): never => {
    throw new Error(`Invalid ambiance colour field: ${why}`);
  };
  if (!value || typeof value !== "object") fail("not an object");
  const f = value as AmbianceColorField;
  if (f.version !== 1 || f.kind !== "native-ambiance-color-field") fail("version/kind");
  if (typeof f.map !== "string" || !f.map) fail("map");
  if (f.ambiance !== "Fog") fail("only Fog fields are defined");
  if (f.space !== "srgb-unorm") fail("colour space");
  if (!Number.isFinite(f.camera_elevation_deg) || f.camera_elevation_deg <= 0 || f.camera_elevation_deg >= 90)
    fail("camera elevation");
  for (const key of ["day", "fog"] as const) {
    const s = f.source?.[key];
    if (!s || typeof s.path !== "string" || !s.path || typeof s.sha256 !== "string" || !sha.test(s.sha256))
      fail(`source.${key}`);
  }
  const positive = (v: unknown) => Number.isInteger(v) && (v as number) > 0;
  if (!Array.isArray(f.map_size) || f.map_size.length !== 2 || !f.map_size.every(positive)) fail("map_size");
  if (!Array.isArray(f.grid) || f.grid.length !== 2 || !f.grid.every(positive)) fail("grid");
  if (!positive(f.block)) fail("block");
  if (f.grid[0] !== Math.ceil(f.map_size[0] / f.block) || f.grid[1] !== Math.ceil(f.map_size[1] / f.block))
    fail("grid does not tile the map");
  const cells = f.grid[0] * f.grid[1];
  if (!Array.isArray(f.slope) || f.slope.length !== cells || !f.slope.every((v) => Number.isFinite(v) && v > 0 && v <= 2))
    fail("slope");
  if (!Array.isArray(f.offset) || f.offset.length !== 3 * cells || !f.offset.every((v) => Number.isFinite(v) && v >= -1 && v <= 1))
    fail("offset");
  return f;
}

async function pinnedJson(root: FileSystemDirectoryHandle, pin: { path: string; sha256: string }) {
  const parts = pin.path.split("/"),
    name = parts.pop()!,
    directory = await subdir(root, parts);
  if (!directory) throw new Error(`Missing ambiance field: ${pin.path}`);
  const bytes = await (await (await directory.getFileHandle(name)).getFile()).arrayBuffer();
  const hash = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (n) =>
    n.toString(16).padStart(2, "0"),
  ).join("");
  if (hash !== pin.sha256) throw new Error(`Ambiance field changed: ${pin.path}`);
  return JSON.parse(new TextDecoder().decode(bytes)) as unknown;
}

/** A library without an ambiance catalog, or without a row for this map, has no field (null).
 * A declared row that is missing, altered or malformed is an error. */
export async function loadAmbianceColorField(
  root: FileSystemDirectoryHandle,
  map: string,
  ambiance: string,
): Promise<AmbianceColorField | null> {
  const directory = await subdir(root, ["ambiance"]);
  if (!directory) return null;
  let value: unknown;
  try {
    value = await readJson(directory, "index.json");
  } catch (error) {
    if (isNotFound(error)) return null;
    throw error;
  }
  const catalog = value as { version?: unknown; entries?: unknown };
  if (!catalog || catalog.version !== 1 || !Array.isArray(catalog.entries))
    throw new Error("Invalid ambiance catalog");
  const rows = (catalog.entries as CatalogEntry[]).filter((e) => e?.map === map && e?.ambiance === ambiance);
  if (rows.length > 1) throw new Error(`Duplicate ambiance field for ${map} ${ambiance}`);
  const row = rows[0];
  if (!row) return null;
  if (typeof row.field?.path !== "string" || !safeLibraryPath(row.field.path) || !sha.test(row.field.sha256 ?? ""))
    throw new Error(`Invalid ambiance catalog row for ${map} ${ambiance}`);
  const field = validateAmbianceColorField(await pinnedJson(root, row.field));
  if (field.map !== map || field.ambiance !== ambiance)
    throw new Error("Ambiance field differs from its catalog row");
  return field;
}

/** RGB = offset, A = slope; texel (0, 0) is the top-left tile, sampled with v = map_y / height. */
export function ambianceFieldTexture(field: AmbianceColorField): THREE.DataTexture {
  const [gx, gy] = field.grid;
  const data = new Uint16Array(gx * gy * 4);
  for (let i = 0; i < gx * gy; i++) {
    for (let c = 0; c < 3; c++) data[i * 4 + c] = THREE.DataUtils.toHalfFloat(field.offset[i * 3 + c]!);
    data[i * 4 + 3] = THREE.DataUtils.toHalfFloat(field.slope[i]!);
  }
  const texture = new THREE.DataTexture(data, gx, gy, THREE.RGBAFormat, THREE.HalfFloatType);
  texture.colorSpace = THREE.NoColorSpace;
  texture.flipY = false;
  texture.magFilter = THREE.LinearFilter;
  texture.minFilter = THREE.LinearFilter;
  texture.generateMipmaps = false;
  texture.wrapS = texture.wrapT = THREE.ClampToEdgeWrapping;
  texture.needsUpdate = true;
  return texture;
}

/** CPU reference of the shader for one 8-bit sRGB colour at a map pixel (tests, audits). */
export function applyAmbianceField(field: AmbianceColorField, mapX: number, mapY: number, rgb: readonly number[]) {
  const [gx, gy] = field.grid,
    [w, h] = field.map_size;
  // Bilinear between tile centres, clamped at the border (GPU LinearFilter + ClampToEdge).
  const fx = THREE.MathUtils.clamp((mapX / w) * gx - 0.5, 0, gx - 1),
    fy = THREE.MathUtils.clamp((mapY / h) * gy - 0.5, 0, gy - 1);
  const x0 = Math.floor(fx),
    y0 = Math.floor(fy),
    x1 = Math.min(x0 + 1, gx - 1),
    y1 = Math.min(y0 + 1, gy - 1),
    tx = fx - x0,
    ty = fy - y0;
  const at = (values: number[], stride: number, k: number) => {
    const v = (x: number, y: number) => values[(y * gx + x) * stride + k]!;
    return (v(x0, y0) * (1 - tx) + v(x1, y0) * tx) * (1 - ty) + (v(x0, y1) * (1 - tx) + v(x1, y1) * tx) * ty;
  };
  const s = at(field.slope, 1, 0);
  return rgb.map((v, c) => Math.round(THREE.MathUtils.clamp((v / 255) * s + at(field.offset, 3, c), 0, 1) * 255));
}

/** Materials whose texels already come from the Fog painting keep their colours. */
export function isAmbianceNativeMaterial(material: THREE.Material, field: AmbianceColorField) {
  return (
    material.userData.source_ambiance === field.ambiance ||
    material.userData.reprojection_source_sha256 === field.source.fog.sha256
  );
}

export interface AmbianceFieldUniforms {
  fogFieldEnabled: { value: boolean };
  fogFieldMap: { value: THREE.Texture | null };
  fogFieldMapSize: { value: THREE.Vector2 };
  fogFieldElevation: { value: THREE.Vector2 };
}
export function ambianceFieldUniforms(): AmbianceFieldUniforms {
  return {
    fogFieldEnabled: { value: false },
    fogFieldMap: { value: null },
    fogFieldMapSize: { value: new THREE.Vector2(1, 1) },
    fogFieldElevation: { value: new THREE.Vector2(0, 1) },
  };
}
export function setAmbianceFieldUniforms(uniforms: AmbianceFieldUniforms, field: AmbianceColorField | null) {
  uniforms.fogFieldMap.value?.dispose();
  uniforms.fogFieldMap.value = field ? ambianceFieldTexture(field) : null;
  uniforms.fogFieldEnabled.value = !!field;
  if (!field) return;
  const elevation = THREE.MathUtils.degToRad(field.camera_elevation_deg);
  uniforms.fogFieldMapSize.value.set(field.map_size[0], field.map_size[1]);
  uniforms.fogFieldElevation.value.set(Math.sin(elevation), Math.cos(elevation));
}

export const AMBIANCE_FIELD_PROGRAM_KEY = "native-ambiance-field-v1";
const supported = (material: THREE.Material) =>
  material instanceof THREE.MeshBasicMaterial ||
  material instanceof THREE.MeshLambertMaterial ||
  material instanceof THREE.MeshPhongMaterial ||
  material instanceof THREE.MeshStandardMaterial;
export function supportsAmbianceField(material: THREE.Material) {
  return supported(material);
}

/** Inject the field into a compiled standard shader. The editor world is three.js Y-up with the
 * map's Z-up frame rotated -90° about X, so map x = X and map y = Z·sin(e) − Y·cos(e). The field
 * acts on the final sRGB colour, matching how it was fitted on 8-bit paintings. */
export function injectAmbianceField(shader: THREE.WebGLProgramParametersWithUniforms, uniforms: AmbianceFieldUniforms) {
  if (!shader.vertexShader.includes("#include <project_vertex>") || !shader.fragmentShader.includes("#include <opaque_fragment>"))
    throw new Error("Ambiance field requires a standard three.js material shader");
  Object.assign(shader.uniforms, uniforms);
  shader.vertexShader =
    "varying vec3 vFogFieldWorld;\n" +
    shader.vertexShader.replace(
      "#include <project_vertex>",
      `#include <project_vertex>
       vec4 fogFieldWorld = vec4( transformed, 1.0 );
       #ifdef USE_BATCHING
         fogFieldWorld = batchingMatrix * fogFieldWorld;
       #endif
       #ifdef USE_INSTANCING
         fogFieldWorld = instanceMatrix * fogFieldWorld;
       #endif
       vFogFieldWorld = ( modelMatrix * fogFieldWorld ).xyz;`,
    );
  shader.fragmentShader =
    "varying vec3 vFogFieldWorld;\nuniform bool fogFieldEnabled;\nuniform sampler2D fogFieldMap;\nuniform vec2 fogFieldMapSize;\nuniform vec2 fogFieldElevation;\n" +
    shader.fragmentShader.replace(
      "#include <opaque_fragment>",
      `if ( fogFieldEnabled ) {
         vec2 fogFieldUv = vec2( vFogFieldWorld.x, vFogFieldWorld.z * fogFieldElevation.x - vFogFieldWorld.y * fogFieldElevation.y ) / fogFieldMapSize;
         vec4 fogField = texture2D( fogFieldMap, fogFieldUv );
         vec3 fogFieldSrgb = sRGBTransferOETF( vec4( outgoingLight, 1.0 ) ).rgb;
         outgoingLight = sRGBTransferEOTF( vec4( clamp( fogFieldSrgb * fogField.a + fogField.rgb, 0.0, 1.0 ), 1.0 ) ).rgb;
       }
       #include <opaque_fragment>`,
    );
}

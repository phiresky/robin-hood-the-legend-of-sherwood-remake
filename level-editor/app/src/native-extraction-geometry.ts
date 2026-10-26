import * as THREE from "three";
const components: Record<string, number> = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4 };
const attributeNames: Record<string, string> = {
  POSITION: "position",
  NORMAL: "normal",
  TANGENT: "tangent",
  TEXCOORD_0: "uv",
  TEXCOORD_1: "uv1",
  TEXCOORD_2: "uv2",
  TEXCOORD_3: "uv3",
  COLOR_0: "color",
  COLOR_1: "color_1",
};
type Json = {
  accessors: Array<{
    bufferView: number;
    byteOffset?: number;
    componentType: number;
    count: number;
    type: string;
    sparse?: unknown;
    normalized?: boolean;
  }>;
  bufferViews: Array<{
    buffer?: number;
    byteOffset?: number;
    byteLength: number;
    byteStride?: number;
  }>;
  meshes: Array<{
    primitives: Array<{
      mode?: number;
      indices?: number;
      attributes: Record<string, number>;
      targets?: Array<Record<string, number>>;
      extensions?: unknown;
    }>;
  }>;
};
/** One private byte snapshot shared by all accessor checks for a loaded model. */
export function createNativeExtractionGeometryVerifier(bytes: Uint8Array, json: Json) {
  const owned = Uint8Array.from(bytes), metadata = structuredClone(json);
  return (mesh: THREE.Mesh, meshIndex: number, primitiveIndex: number) =>
    verifyOwnedGeometry(owned, metadata, mesh, meshIndex, primitiveIndex);
}

/** Standalone verification owns its input; contexts share one verifier instead. */
export function verifyNativeExtractionGeometry(
  bytes: Uint8Array, json: Json, mesh: THREE.Mesh, meshIndex: number, primitiveIndex: number,
): () => void {
  return createNativeExtractionGeometryVerifier(bytes, json)(mesh, meshIndex, primitiveIndex);
}

function verifyOwnedGeometry(
  bytes: Uint8Array,
  json: Json,
  mesh: THREE.Mesh,
  meshIndex: number,
  primitiveIndex: number,
): () => void {
  const primitive = json.meshes?.[meshIndex]?.primitives?.[primitiveIndex];
  if (
    !primitive ||
    !Number.isSafeInteger(primitiveIndex) ||
    primitiveIndex < 0 ||
    (primitive.mode !== undefined && primitive.mode !== 4) ||
    primitive.extensions
  )
    throw new Error("Native extraction context unsupported primitive association");
  const header = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const chunk = 20 + header.getUint32(12, true);
  if (chunk + 8 > bytes.length || header.getUint32(chunk + 4, true) !== 0x004e4942)
    throw new Error("Native extraction context missing embedded geometry");
  const data = new DataView(
    bytes.buffer,
    bytes.byteOffset + chunk + 8,
    header.getUint32(chunk, true),
  );
  const checks: Array<() => void> = [];
  function verify(
    index: number,
    attribute: THREE.BufferAttribute | THREE.InterleavedBufferAttribute | undefined,
  ) {
    const accessor = json.accessors?.[index],
      view = accessor && json.bufferViews?.[accessor.bufferView];
    if (!accessor || !view || accessor.sparse || (view.buffer ?? 0) !== 0 || !attribute)
      throw new Error("Native extraction context unsupported or missing accessor");
    const size = components[accessor.type];
    const width = (
      { 5120: 1, 5121: 1, 5122: 2, 5123: 2, 5125: 4, 5126: 4 } as Record<number, number>
    )[accessor.componentType];
    if (!size || !width || attribute.count !== accessor.count || attribute.itemSize !== size)
      throw new Error("Native extraction context accessor shape differs");
    const stride = view.byteStride ?? size * width,
      offset = (view.byteOffset ?? 0) + (accessor.byteOffset ?? 0);
    if (
      stride < size * width ||
      offset < (view.byteOffset ?? 0) ||
      offset + Math.max(0, accessor.count - 1) * stride + size * width >
        (view.byteOffset ?? 0) + view.byteLength
    )
      throw new Error("Native extraction context accessor exceeds buffer view");
    const read = (offset: number) => {
      let value: number;
      switch (accessor.componentType) {
        case 5120:
          value = data.getInt8(offset);
          break;
        case 5121:
          value = data.getUint8(offset);
          break;
        case 5122:
          value = data.getInt16(offset, true);
          break;
        case 5123:
          value = data.getUint16(offset, true);
          break;
        case 5125:
          value = data.getUint32(offset, true);
          break;
        case 5126:
          value = data.getFloat32(offset, true);
          break;
        default:
          throw new Error("Unsupported component type");
      }
      if (accessor.normalized) {
        const max = (
          { 5120: 127, 5121: 255, 5122: 32767, 5123: 65535, 5125: 4294967295 } as Record<
            number,
            number
          >
        )[accessor.componentType];
        if (!max) throw new Error("Invalid normalized accessor");
        value = Math.max(-1, value / max);
      }
      return value;
    };
    for (let i = 0; i < accessor.count; i++)
      for (let c = 0; c < size; c++) {
        const expected = read(offset + i * stride + c * width);
        if (!Number.isFinite(expected) || attribute.getComponent(i, c) !== expected)
          throw new Error("Native extraction context loaded accessor differs from pinned model");
      }
    checks.push(() => {
      if (attribute.count !== accessor.count || attribute.itemSize !== size)
        throw new Error("Native extraction accessor changed");
      for (let i = 0; i < accessor.count; i++)
        for (let c = 0; c < size; c++)
          if (attribute.getComponent(i, c) !== read(offset + i * stride + c * width))
            throw new Error("Native extraction immutable accessor changed");
    });
  }
  const geometry = mesh.geometry;
  const relativeMorphs = geometry.morphTargetsRelative;
  checks.push(() => {
    if (geometry.morphTargetsRelative !== relativeMorphs)
      throw new Error("Native extraction morph semantics changed");
  });
  for (const [semantic, index] of Object.entries(primitive.attributes)) {
    const name = attributeNames[semantic];
    if (!name) throw new Error("Native extraction unsupported attribute semantic");
    const attribute = geometry.getAttribute(name);
    verify(index, attribute);
    checks.push(() => {
      if (geometry.getAttribute(name) !== attribute)
        throw new Error("Native extraction attribute replaced");
    });
  }
  if (primitive.indices !== undefined) {
    const index = geometry.index;
    verify(primitive.indices, index ?? undefined);
    checks.push(() => {
      if (geometry.index !== index) throw new Error("Native extraction indices replaced");
    });
  } else if (geometry.index) throw new Error("Native extraction unexpected index buffer");
  for (const [i, target] of (primitive.targets ?? []).entries())
    for (const [semantic, index] of Object.entries(target)) {
      const name = attributeNames[semantic];
      if (
        (name !== "position" && name !== "normal" && name !== "color") ||
        !geometry.morphTargetsRelative
      )
        throw new Error("Native extraction unsupported morph layout");
      const attribute = geometry.morphAttributes[name]?.[i];
      verify(index, attribute);
      checks.push(() => {
        if (geometry.morphAttributes[name]?.[i] !== attribute)
          throw new Error("Native extraction morph replaced");
      });
    }
  return () => {
    if (mesh.geometry !== geometry) throw new Error("Native extraction geometry replaced");
    checks.forEach((check) => check());
  };
}

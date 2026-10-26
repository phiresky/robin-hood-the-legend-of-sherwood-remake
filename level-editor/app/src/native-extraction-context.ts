import { createNativeExtractionGeometryVerifier } from "./native-extraction-geometry.ts";
import * as THREE from "three";
import type { MissionStateContract } from "../../shared/src/mission-state.ts";
type Binding = MissionStateContract["targets"][number];
type Role = "source-owner" | "static-receiver";
export interface NativeExtractionManifest {
  version: 1;
  model_sha256: string;
  owner: { kind: "map-animation"; index: number; sha256: string };
  components: { node: number; role: Role }[];
}
export interface NativeExtractionScenes {
  owner: THREE.Object3D;
  depth: THREE.Object3D;
  dispose(): void;
}
const trusted = new WeakSet<object>();
const retirement = new WeakMap<NativeExtractionContext, () => void>();
const retired = new WeakSet<NativeExtractionContext>();
export interface NativeExtractionContext {
  prepare(object: THREE.Object3D, binding: Binding): NativeExtractionScenes;
}
const digest = async (bytes: Uint8Array) =>
  Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", Uint8Array.from(bytes).buffer)))
    .map((n) => n.toString(16).padStart(2, "0"))
    .join("");
/** Bind a complete loaded family to an explicitly approved receiver partition. */
export async function bindNativeExtractionContext(input: {
  object: THREE.Object3D;
  binding: Binding;
  modelBytes: Uint8Array;
  manifestBytes: Uint8Array;
  manifestSha256: string;
  parserJson: unknown;
  associations: Map<THREE.Object3D, { nodes?: number; meshes?: number; primitives?: number }>;
}): Promise<NativeExtractionContext> {
  const { object, binding } = input;
  // Own asynchronous hash/parse inputs; caller aliases never become authority.
  const bytes = Uint8Array.from(input.modelBytes);
  const manifestBytes = Uint8Array.from(input.manifestBytes);
  const parserJson = structuredClone(input.parserJson);
  if (
    !/^[a-f0-9]{64}$/.test(input.manifestSha256) ||
    (await digest(manifestBytes)) !== input.manifestSha256 ||
    (await digest(bytes)) !== binding.source.model_sha256
  ) throw new Error("Native extraction context pin mismatch");
  const header = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (
    bytes.length < 20 ||
    header.getUint32(0, true) !== 0x46546c67 ||
    header.getUint32(4, true) !== 2 ||
    header.getUint32(8, true) !== bytes.length ||
    header.getUint32(16, true) !== 0x4e4f534a
  )
    throw new Error("Native extraction context requires the pinned GLB");
  const json = JSON.parse(
    new TextDecoder().decode(bytes.subarray(20, 20 + header.getUint32(12, true))),
  );
  const verifyGeometry = createNativeExtractionGeometryVerifier(bytes, json);
  if (JSON.stringify(json) !== JSON.stringify(parserJson))
    throw new Error("Native extraction parser differs from pinned model");
  const manifest = JSON.parse(
    new TextDecoder().decode(manifestBytes),
  ) as NativeExtractionManifest;
  if (
    manifest.version !== 1 ||
    manifest.model_sha256 !== binding.source.model_sha256 ||
    binding.source_kind !== "map-animation" ||
    manifest.owner?.kind !== "map-animation" ||
    manifest.owner.index !== binding.target_index ||
    manifest.owner.sha256 !== binding.target_sha256 ||
    !Array.isArray(manifest.components)
  )
    throw new Error("Native extraction context source family differs");
  const roles = new Map<number, Role>();
  for (const c of manifest.components) {
    if (
      !Number.isSafeInteger(c.node) ||
      c.node < 0 ||
      roles.has(c.node) ||
      !["source-owner", "static-receiver"].includes(c.role) ||
      !Number.isSafeInteger(json.nodes?.[c.node]?.mesh)
    )
      throw new Error("Native extraction context invalid component");
    roles.set(c.node, c.role);
  }
  const meshRoles = new Map<THREE.Object3D, Role>(),
    seen = new Set<number>();
  const geometryChecks: Array<() => void> = [];
  const seenPrimitives = new Set<string>();
  object.traverse((mesh) => {
    if (!(mesh instanceof THREE.Mesh)) return;
    if (mesh instanceof THREE.SkinnedMesh)
      throw new Error("Native extraction context forbids skins");
    let index: number | undefined;
    for (let node: THREE.Object3D | null = mesh; node; node = node.parent) {
      index = input.associations.get(node)?.nodes;
      if (index !== undefined || node === object) break;
    }
    const role = index === undefined ? undefined : roles.get(index);
    if (!role || index === undefined) throw new Error("Native extraction context unbound mesh");
    const primitive = input.associations.get(mesh);
    if (primitive?.meshes !== json.nodes[index].mesh || !Number.isInteger(primitive?.primitives))
      throw new Error("Native extraction context mesh association differs");
    const key = `${index}:${primitive!.primitives}`;
    if (seenPrimitives.has(key)) throw new Error("Native extraction duplicate loaded primitive");
    seenPrimitives.add(key);
    geometryChecks.push(
      verifyGeometry(mesh, primitive!.meshes!, primitive!.primitives!),
    );
    seen.add(index);
    meshRoles.set(mesh, role);
  });
  if (
    seen.size !== roles.size ||
    ![...roles.values()].includes("source-owner") ||
    ![...roles.values()].includes("static-receiver")
  )
    throw new Error("Native extraction context incomplete family");
  for (const node of roles.keys()) {
    const primitives = json.meshes[json.nodes[node].mesh].primitives;
    for (let primitive = 0; primitive < primitives.length; primitive++)
      if (!seenPrimitives.has(`${node}:${primitive}`))
        throw new Error("Native extraction missing loaded primitive");
  }
  const pinnedBinding = JSON.stringify(binding);
  const geometries = new Map(
    [...meshRoles.keys()].map((node) => [node, (node as THREE.Mesh).geometry]),
  );
  const context: NativeExtractionContext = {
    prepare(current, currentBinding) {
      if (retired.has(context)) throw new Error("Native extraction context retired");
      if (current !== object || JSON.stringify(currentBinding) !== pinnedBinding)
        throw new Error("Native extraction context binding changed");
      const currentMeshes: THREE.Object3D[] = [];
      object.traverse((node) => {
        if (node instanceof THREE.Mesh) currentMeshes.push(node);
      });
      if (
        currentMeshes.length !== meshRoles.size ||
        currentMeshes.some(
          (node) => !meshRoles.has(node) || (node as THREE.Mesh).geometry !== geometries.get(node),
        )
      )
        throw new Error("Native extraction context loaded family changed");
      geometryChecks.forEach((check) => check());
      object.updateWorldMatrix(true, true);
      const materials: THREE.Material[] = [];
      const clone = (depthOnly: boolean) => {
        const copy = object.clone(true);
        copy.matrixAutoUpdate = false;
        copy.matrix.copy(object.matrixWorld);
        const copies: THREE.Mesh[] = [];
        copy.traverse((node) => {
          if (node instanceof THREE.Mesh) copies.push(node);
        });
        copies.forEach((mesh, i) => {
          const role = meshRoles.get(currentMeshes[i]!)!;
          if ((role === "static-receiver") !== depthOnly) {
            mesh.layers.mask = 0;
            return;
          }
          if (!depthOnly) return;
          const convert = (original: THREE.Material) => {
            if (
              original.transparent ||
              original.opacity !== 1 ||
              !original.depthTest ||
              !original.depthWrite ||
              original.stencilWrite ||
              original.polygonOffset ||
              original.alphaHash ||
              original.alphaToCoverage
            )
              throw new Error("Native extraction context unsupported receiver depth");
            const material = original.clone();
            material.onBeforeCompile = original.onBeforeCompile;
            material.customProgramCacheKey = original.customProgramCacheKey;
            material.colorWrite = false;
            materials.push(material);
            return material;
          };
          mesh.material = Array.isArray(mesh.material)
            ? mesh.material.map(convert)
            : convert(mesh.material);
        });
        return copy;
      };
      try {
        const owner = clone(false),
          depth = clone(true);
        return {
          owner,
          depth,
          dispose: () => {
            materials.forEach((m) => m.dispose());
          },
        };
      } catch (error) {
        materials.forEach((m) => m.dispose());
        throw error;
      }
    },
  };
  Object.freeze(context);
  trusted.add(context);
  retirement.set(context, () => {
    geometryChecks.length = 0;
    geometries.clear();
    meshRoles.clear();
  });
  return context;
}
export function prepareNativeExtractionContext(
  context: NativeExtractionContext,
  object: THREE.Object3D,
  binding: Binding,
) {
  if (!trusted.has(context)) throw new Error("Unverified native extraction context");
  return context.prepare(object, binding);
}


/** Release verification snapshots after an independent pinned session is ready.
 * The strict handle is permanently invalid; this never skips a live strict check.
 */
export function retireNativeExtractionContext(context: NativeExtractionContext) {
  if (retired.has(context)) return;
  const release = retirement.get(context);
  if (!trusted.has(context) || !release) throw new Error("Unverified native extraction context");
  retired.add(context);
  trusted.delete(context);
  retirement.delete(context);
  release();
}

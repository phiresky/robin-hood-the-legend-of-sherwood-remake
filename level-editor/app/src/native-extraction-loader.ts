import * as THREE from "three";
import { safeLibraryPath } from "../../shared/src/projection-assets.ts";
import type { MissionStateContract } from "../../shared/src/mission-state.ts";
import {
  bindNativeExtractionContext,
  retireNativeExtractionContext,
  type NativeExtractionContext,
} from "./native-extraction-context.ts";

type Association = { nodes?: number; meshes?: number; primitives?: number };
type ModelProof = {
  object: THREE.Object3D;
  modelBytes: Uint8Array;
  parserJson: unknown;
  associations: Map<THREE.Object3D, Association>;
};
export type NativeExtractionPin = { path: string; sha256: string };

/** Retain the exact loader identity and rebind it to a complete placed copy. */
export class NativeExtractionLoader {
  private proofs = new WeakMap<THREE.Object3D, ModelProof>();
  private retired = false;
  private contexts = new Set<NativeExtractionContext>();
  private readonly read: (path: string) => Promise<Uint8Array>;
  constructor(read: (path: string) => Promise<Uint8Array>) {
    this.read = read;
  }
  retain(proof: ModelProof) {
    if (this.retired) throw new Error("Native extraction loader is retired");
    if (this.proofs.has(proof.object)) throw new Error("Native extraction model already retained");
    this.proofs.set(proof.object, {
      ...proof,
      modelBytes: Uint8Array.from(proof.modelBytes),
      parserJson: structuredClone(proof.parserJson),
      associations: new Map([...proof.associations].map(([node, value]) => [node, { ...value }])),
    });
  }
  async bind(
    source: THREE.Object3D,
    placed: THREE.Object3D,
    binding: MissionStateContract["targets"][number],
    pin: NativeExtractionPin,
    isCurrent: () => boolean,
    displayRoot: THREE.Object3D = placed,
  ): Promise<NativeExtractionContext> {
    const current = () => {
      if (this.retired || !isCurrent()) throw new Error("Native extraction load was retired");
    };
    current();
    if (!safeLibraryPath(pin.path) || !/^[a-f0-9]{64}$/.test(pin.sha256))
      throw new Error("Invalid native extraction manifest pin");
    const proof = this.proofs.get(source);
    if (!proof) throw new Error("Native extraction requires the retained pinned model");
    const manifestBytes = await this.read(pin.path);
    current();
    const associations = new Map<THREE.Object3D, Association>();
    const pair = (original: THREE.Object3D, copy: THREE.Object3D) => {
      if (original.type !== copy.type || original.children.length !== copy.children.length)
        throw new Error("Native extraction placed hierarchy differs");
      const association = proof.associations.get(original);
      if (association) associations.set(copy, association);
      for (let i = 0; i < original.children.length; i++)
        pair(original.children[i]!, copy.children[i]!);
    };
    pair(source, placed);
    let ancestor: THREE.Object3D | null = placed;
    while (ancestor && ancestor !== displayRoot) ancestor = ancestor.parent;
    if (!ancestor)
      throw new Error("Native extraction display root does not contain placed content");
    const context = await bindNativeExtractionContext({
      object: displayRoot,
      binding,
      modelBytes: proof.modelBytes,
      parserJson: proof.parserJson,
      manifestBytes,
      manifestSha256: pin.sha256,
      associations,
    });
    try { current(); } catch (error) { retireNativeExtractionContext(context); throw error; }
    this.contexts.add(context);
    return context;
  }
  dispose() {
    this.retired = true;
    this.proofs = new WeakMap();
    for (const context of this.contexts) retireNativeExtractionContext(context);
    this.contexts.clear();
  }
}
